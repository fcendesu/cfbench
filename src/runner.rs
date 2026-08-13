use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use thiserror::Error;

use crate::cancellation::CancellationToken;
use crate::clock::RunClock;
use crate::error::TransportError;
use crate::measurement::loaded_latency::{
    LoadedProbeCancellation, LoadedProbeOutcome, spawn_loaded_probe_loop,
};
use crate::measurement::{
    MeasurementConversionError, TimingObservation, bandwidth_point, latency_point,
};
use crate::plan::{Direction, MeasurementPlan, MeasurementStep};
use crate::progress::{
    ProgressEvent, ProgressFailureKind, ProgressReporter, ProgressStage, TransferTelemetry,
};
use crate::results::{
    MetadataStatus, NetworkMetadata, RpkiReachability, RpkiReachabilityStatus, RunResult, reduce,
};
use crate::transport::ReqwestTransport;

const MIN_FINISH_DURATION_MS: f64 = 1_000.0;
const MIN_LOADED_GROUP_DURATION_MS: f64 = 250.0;

/// A boxed measurement operation used by scripted transports in runner tests.
pub type MeasurementFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TimingObservation, TransportError>> + Send + 'a>>;

/// A boxed post-plan metadata operation used without exposing HTTP client types.
pub type MetadataFuture<'a> =
    Pin<Box<dyn Future<Output = Result<NetworkMetadata, TransportError>> + Send + 'a>>;

/// A boxed post-plan RPKI-invalid-route diagnostic operation.
pub type RpkiFuture<'a> =
    Pin<Box<dyn Future<Output = Result<RpkiReachability, TransportError>> + Send + 'a>>;

/// The transport surface required by ordered measurement orchestration.
pub trait MeasurementTransport: Send + Sync + 'static {
    fn metadata<'a>(&'a self, _: &'a CancellationToken) -> MetadataFuture<'a> {
        Box::pin(async { Err(TransportError::MetadataUnsupported) })
    }

    fn rpki_reachability<'a>(&'a self, _: &'a CancellationToken) -> RpkiFuture<'a> {
        Box::pin(async {
            Ok(RpkiReachability {
                status: RpkiReachabilityStatus::Error,
                host: None,
                detail: Some(
                    "RPKI-invalid-route reachability is unavailable for this transport".to_owned(),
                ),
            })
        })
    }

    fn latency<'a>(&'a self, cancellation: &'a CancellationToken) -> MeasurementFuture<'a>;

    /// Starts one loaded probe in the caller's task.
    ///
    /// Implementations must not detach request work and must complete promptly
    /// after `cancellation` is cancelled. The probe controller cancels this
    /// token and awaits the same future before it reports parent cancellation
    /// or treats normal group teardown as silent.
    fn loaded_latency<'a>(
        &'a self,
        direction: Direction,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a>;

    fn download<'a>(
        &'a self,
        bytes: u64,
        during: Option<&'a str>,
        telemetry: TransferTelemetry,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a>;

    fn upload<'a>(
        &'a self,
        bytes: u64,
        telemetry: TransferTelemetry,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a>;
}

impl MeasurementTransport for ReqwestTransport {
    fn metadata<'a>(&'a self, cancellation: &'a CancellationToken) -> MetadataFuture<'a> {
        Box::pin(ReqwestTransport::metadata(self, cancellation))
    }

    fn rpki_reachability<'a>(&'a self, cancellation: &'a CancellationToken) -> RpkiFuture<'a> {
        Box::pin(ReqwestTransport::rpki_reachability(self, cancellation))
    }

    fn latency<'a>(&'a self, cancellation: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(ReqwestTransport::latency(self, cancellation))
    }

    fn loaded_latency<'a>(
        &'a self,
        direction: Direction,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        Box::pin(ReqwestTransport::download(
            self,
            0,
            Some(direction_name(direction)),
            cancellation,
        ))
    }

    fn download<'a>(
        &'a self,
        bytes: u64,
        during: Option<&'a str>,
        telemetry: TransferTelemetry,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        Box::pin(ReqwestTransport::download_with_telemetry(
            self,
            bytes,
            during,
            Some(telemetry),
            cancellation,
        ))
    }

    fn upload<'a>(
        &'a self,
        bytes: u64,
        _: TransferTelemetry,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        Box::pin(ReqwestTransport::upload(self, bytes, cancellation))
    }
}

/// A terminal runner failure. Successful earlier points remain in `RunOutcome`.
#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("measurement cancelled during {stage}")]
    Cancelled { stage: String },
    #[error("transport failed during {stage}: {source}")]
    Transport {
        stage: String,
        #[source]
        source: TransportError,
    },
    #[error("measurement conversion failed during {stage} for endpoint {endpoint}: {source}")]
    Conversion {
        stage: String,
        endpoint: String,
        #[source]
        source: MeasurementConversionError,
    },
}

/// The always-available result envelope and an optional terminal failure.
#[derive(Debug)]
pub struct RunOutcome {
    pub result: RunResult,
    pub error: Option<RunnerError>,
}

/// Executes an immutable measurement plan in source order.
pub struct Runner<T> {
    transport: Arc<T>,
    plan: MeasurementPlan,
    loaded_latency: bool,
    metadata: bool,
    rpki_check: bool,
}

struct TransferProgress {
    reporter: ProgressReporter,
    loaded_sequence: Arc<AtomicU64>,
    clock: RunClock,
}

impl<T> Runner<T>
where
    T: MeasurementTransport,
{
    pub fn new(transport: T, plan: MeasurementPlan) -> Self {
        Self {
            transport: Arc::new(transport),
            plan,
            loaded_latency: true,
            metadata: false,
            rpki_check: false,
        }
    }

    pub fn with_loaded_latency(mut self, enabled: bool) -> Self {
        self.loaded_latency = enabled;
        self
    }

    pub fn with_metadata(mut self, enabled: bool) -> Self {
        self.metadata = enabled;
        self
    }

    pub fn with_rpki_check(mut self, enabled: bool) -> Self {
        self.rpki_check = enabled;
        self
    }

    pub async fn run(&self, cancellation: &CancellationToken) -> RunOutcome {
        self.run_with_progress(cancellation, ProgressReporter::disabled())
            .await
    }

    pub async fn run_with_progress(
        &self,
        cancellation: &CancellationToken,
        progress: ProgressReporter,
    ) -> RunOutcome {
        let mut result = RunResult::empty();
        let clock = RunClock::start();
        result.started_at = clock.started_at().to_owned();
        let mut error = self
            .execute(&mut result, cancellation, &progress, &clock)
            .await;
        result.usage.duration_ms = clock.elapsed().as_secs_f64() * 1_000.0;
        result.summary = reduce(&result.raw);
        reconcile_parent_cancellation(&mut result, &mut error, cancellation);
        let terminally_cancelled = matches!(error.as_ref(), Some(RunnerError::Cancelled { .. }));
        if !self.metadata {
            result.target.metadata_status = MetadataStatus::Disabled;
        } else if !terminally_cancelled {
            match self.transport.metadata(cancellation).await {
                Ok(metadata) => {
                    result.target.metadata_status = MetadataStatus::Available;
                    result.target.metadata = Some(metadata);
                }
                Err(TransportError::Cancelled { .. }) => {
                    error = Some(record_failure(
                        &mut result,
                        RunnerError::Cancelled {
                            stage: "metadata".to_owned(),
                        },
                    ));
                }
                Err(_) if cancellation.is_cancelled() => {
                    reconcile_parent_cancellation(&mut result, &mut error, cancellation);
                }
                Err(metadata_error) => result
                    .diagnostics
                    .push(metadata_diagnostic(&metadata_error)),
            }
        }
        reconcile_parent_cancellation(&mut result, &mut error, cancellation);
        let terminally_cancelled = matches!(error.as_ref(), Some(RunnerError::Cancelled { .. }));
        if self.rpki_check && !terminally_cancelled {
            match self.transport.rpki_reachability(cancellation).await {
                Ok(rpki) => {
                    if rpki.status == RpkiReachabilityStatus::Error {
                        result.diagnostics.push(rpki_result_diagnostic(&rpki));
                    }
                    result.rpki = rpki;
                }
                Err(rpki_error) => {
                    let rpki = rpki_error_result(&rpki_error);
                    result.diagnostics.push(rpki_result_diagnostic(&rpki));
                    result.rpki = rpki;
                }
            }
        }
        reconcile_parent_cancellation(&mut result, &mut error, cancellation);

        RunOutcome { result, error }
    }

    async fn execute(
        &self,
        result: &mut RunResult,
        cancellation: &CancellationToken,
        progress: &ProgressReporter,
        clock: &RunClock,
    ) -> Option<RunnerError> {
        let mut download_finished = false;
        let mut upload_finished = false;
        let mut download_finish_reported = false;
        let mut upload_finish_reported = false;
        let download_loaded_sequence = Arc::new(AtomicU64::new(0));
        let upload_loaded_sequence = Arc::new(AtomicU64::new(0));

        for step in &self.plan.steps {
            if cancellation.is_cancelled() {
                return Some(record_failure(
                    result,
                    RunnerError::Cancelled {
                        stage: stage_name(*step),
                    },
                ));
            }

            let error = match *step {
                MeasurementStep::Latency { packets } => {
                    self.run_latency_phase(packets, result, cancellation, progress, clock)
                        .await
                }
                MeasurementStep::Download {
                    bytes,
                    count,
                    bypass_finish,
                } if !download_finished => {
                    match self
                        .run_transfer_group(
                            Direction::Download,
                            bytes,
                            count,
                            result,
                            cancellation,
                            TransferProgress {
                                reporter: progress.clone(),
                                loaded_sequence: download_loaded_sequence.clone(),
                                clock: clock.clone(),
                            },
                        )
                        .await
                    {
                        Ok(group_finished) => {
                            if !bypass_finish && group_finished {
                                download_finished = true;
                            }
                            None
                        }
                        Err(error) => Some(error),
                    }
                }
                MeasurementStep::Upload {
                    bytes,
                    count,
                    bypass_finish,
                } if !upload_finished => {
                    match self
                        .run_transfer_group(
                            Direction::Upload,
                            bytes,
                            count,
                            result,
                            cancellation,
                            TransferProgress {
                                reporter: progress.clone(),
                                loaded_sequence: upload_loaded_sequence.clone(),
                                clock: clock.clone(),
                            },
                        )
                        .await
                    {
                        Ok(group_finished) => {
                            if !bypass_finish && group_finished {
                                upload_finished = true;
                            }
                            None
                        }
                        Err(error) => Some(error),
                    }
                }
                MeasurementStep::PacketLossUnsupported { .. } => None,
                MeasurementStep::Download { .. } => {
                    if !download_finish_reported {
                        progress.emit(ProgressEvent::DirectionFinished {
                            direction: Direction::Download,
                        });
                        download_finish_reported = true;
                    }
                    None
                }
                MeasurementStep::Upload { .. } => {
                    if !upload_finish_reported {
                        progress.emit(ProgressEvent::DirectionFinished {
                            direction: Direction::Upload,
                        });
                        upload_finish_reported = true;
                    }
                    None
                }
            };

            if let Some(error) = error {
                return Some(error);
            }
        }
        None
    }

    async fn run_latency_phase(
        &self,
        packets: u32,
        result: &mut RunResult,
        cancellation: &CancellationToken,
        progress: &ProgressReporter,
        clock: &RunClock,
    ) -> Option<RunnerError> {
        result.raw.latency.reserve(packets as usize);

        for (index, _) in (0..packets).enumerate() {
            let current = progress_counter(index.saturating_add(1));
            let total = progress_counter(packets as usize);
            progress.emit(ProgressEvent::RequestStarted {
                stage: ProgressStage::Latency,
                current: Some(current),
                total: Some(total),
            });
            let observation = match self.transport.latency(cancellation).await {
                Ok(observation) => observation,
                Err(error) => {
                    let kind = progress_failure_kind(&error);
                    let runner_error = map_transport_error("latency", error);
                    return Some(record_request_failure(
                        result,
                        runner_error,
                        progress,
                        ProgressStage::Latency,
                        current,
                        total,
                        kind,
                    ));
                }
            };
            let ip_family = observation.ip_family.clone();
            let http_version = observation.http_version.clone();
            let endpoint = observation.endpoint.clone();
            update_ip_family(result, ip_family.as_deref());
            update_http_version(result, http_version.as_deref());
            let point = match latency_point(observation, clock.now_unix_ms()) {
                Ok(point) => point,
                Err(error) => {
                    return Some(record_request_failure(
                        result,
                        RunnerError::Conversion {
                            stage: "latency".to_owned(),
                            endpoint,
                            source: error,
                        },
                        progress,
                        ProgressStage::Latency,
                        current,
                        total,
                        ProgressFailureKind::InvalidMeasurement,
                    ));
                }
            };
            let latency_ms = point.ping_ms;
            result.raw.latency.push(point);
            progress.emit(ProgressEvent::LatencyCompleted {
                current,
                total,
                latency_ms,
            });
        }
        None
    }

    async fn run_transfer_group(
        &self,
        direction: Direction,
        bytes: u64,
        count: u32,
        result: &mut RunResult,
        cancellation: &CancellationToken,
        progress: TransferProgress,
    ) -> Result<bool, RunnerError> {
        let TransferProgress {
            reporter,
            loaded_sequence,
            clock,
        } = progress;
        let probe_cancellation = LoadedProbeCancellation::new(cancellation);
        let probe_task = self.loaded_latency.then(|| {
            spawn_loaded_probe_loop(
                self.transport.clone(),
                direction,
                probe_cancellation.clone(),
                reporter.clone(),
                loaded_sequence,
                clock.clone(),
            )
        });
        let mut durations = Vec::with_capacity(count as usize);
        let mut terminal_error = None;

        for (index, _) in (0..count).enumerate() {
            let current = progress_counter(index.saturating_add(1));
            let total = progress_counter(count as usize);
            reporter.emit(ProgressEvent::RequestStarted {
                stage: ProgressStage::Transfer {
                    direction,
                    requested_bytes: bytes,
                },
                current: Some(current),
                total: Some(total),
            });
            let telemetry =
                TransferTelemetry::new(reporter.clone(), direction, bytes, current, total);
            let observation = match direction {
                Direction::Download => {
                    self.transport
                        .download(bytes, None, telemetry, cancellation)
                        .await
                }
                Direction::Upload => self.transport.upload(bytes, telemetry, cancellation).await,
            };
            let observation = match observation {
                Ok(observation) => observation,
                Err(error) => {
                    match direction {
                        Direction::Download => {
                            result.usage.download_payload_bytes = result
                                .usage
                                .download_payload_bytes
                                .saturating_add(error.payload_bytes());
                        }
                        Direction::Upload => {
                            result.usage.upload_payload_bytes = result
                                .usage
                                .upload_payload_bytes
                                .saturating_add(error.payload_bytes());
                        }
                    }
                    let kind = progress_failure_kind(&error);
                    terminal_error = Some(record_request_failure(
                        result,
                        map_transport_error(direction_name(direction), error),
                        &reporter,
                        ProgressStage::Transfer {
                            direction,
                            requested_bytes: bytes,
                        },
                        current,
                        total,
                        kind,
                    ));
                    break;
                }
            };
            let ip_family = observation.ip_family.clone();
            let http_version = observation.http_version.clone();
            let endpoint = observation.endpoint.clone();
            update_ip_family(result, ip_family.as_deref());
            update_http_version(result, http_version.as_deref());
            let point = match bandwidth_point(direction, bytes, observation, clock.now_unix_ms()) {
                Ok(point) => point,
                Err(error) => {
                    terminal_error = Some(record_request_failure(
                        result,
                        RunnerError::Conversion {
                            stage: direction_name(direction).to_owned(),
                            endpoint,
                            source: error,
                        },
                        &reporter,
                        ProgressStage::Transfer {
                            direction,
                            requested_bytes: bytes,
                        },
                        current,
                        total,
                        ProgressFailureKind::InvalidMeasurement,
                    ));
                    break;
                }
            };
            let bps = point.bps;
            let adjusted_duration_ms = point.adjusted_duration_ms;
            durations.push(point.adjusted_duration_ms);
            match direction {
                Direction::Download => {
                    result.usage.download_payload_bytes = result
                        .usage
                        .download_payload_bytes
                        .saturating_add(point.payload_bytes);
                    result.raw.download.push(point);
                }
                Direction::Upload => {
                    result.usage.upload_payload_bytes = result
                        .usage
                        .upload_payload_bytes
                        .saturating_add(point.payload_bytes);
                    result.raw.upload.push(point);
                }
            }
            reporter.emit(ProgressEvent::TransferCompleted {
                direction,
                requested_bytes: bytes,
                current,
                total,
                bps,
                adjusted_duration_ms,
            });
        }

        probe_cancellation.cancel_group();
        let probe_outcome = match probe_task {
            Some(task) => match task.await {
                Ok(outcome) => outcome,
                Err(error) => LoadedProbeOutcome {
                    points: Vec::new(),
                    diagnostics: vec![format!("loaded latency task failed to join: {error}")],
                    http_versions: Vec::new(),
                    ip_families: Vec::new(),
                },
            },
            None => LoadedProbeOutcome::default(),
        };
        for version in &probe_outcome.http_versions {
            update_http_version(result, Some(version));
        }
        for family in &probe_outcome.ip_families {
            update_ip_family(result, Some(family));
        }
        result.diagnostics.extend(probe_outcome.diagnostics);

        if !durations.is_empty()
            && durations
                .iter()
                .all(|duration| *duration >= MIN_LOADED_GROUP_DURATION_MS)
        {
            match direction {
                Direction::Download => result
                    .raw
                    .download_loaded_latency
                    .extend(probe_outcome.points),
                Direction::Upload => result
                    .raw
                    .upload_loaded_latency
                    .extend(probe_outcome.points),
            }
        }

        if let Some(error) = terminal_error {
            return Err(error);
        }

        Ok(!durations.is_empty()
            && durations.len() == count as usize
            && durations
                .iter()
                .copied()
                .reduce(f64::min)
                .is_some_and(|minimum| minimum > MIN_FINISH_DURATION_MS))
    }
}

fn map_transport_error(stage: &str, error: TransportError) -> RunnerError {
    match error {
        TransportError::Cancelled { .. } => RunnerError::Cancelled {
            stage: stage.to_owned(),
        },
        error => RunnerError::Transport {
            stage: stage.to_owned(),
            source: error,
        },
    }
}

fn record_failure(result: &mut RunResult, error: RunnerError) -> RunnerError {
    result.failures.push(error.to_string());
    error
}

fn reconcile_parent_cancellation(
    result: &mut RunResult,
    error: &mut Option<RunnerError>,
    cancellation: &CancellationToken,
) {
    if cancellation.is_cancelled() && !matches!(error.as_ref(), Some(RunnerError::Cancelled { .. }))
    {
        *error = Some(record_failure(
            result,
            RunnerError::Cancelled {
                stage: "run".to_owned(),
            },
        ));
    }
}

fn record_request_failure(
    result: &mut RunResult,
    error: RunnerError,
    progress: &ProgressReporter,
    stage: ProgressStage,
    current: u16,
    total: u16,
    kind: ProgressFailureKind,
) -> RunnerError {
    let error = record_failure(result, error);
    progress.emit(ProgressEvent::RequestFailed {
        stage,
        current: Some(current),
        total: Some(total),
        kind,
    });
    error
}

const fn progress_counter(value: usize) -> u16 {
    if value > u16::MAX as usize {
        u16::MAX
    } else {
        value as u16
    }
}

pub(crate) const fn progress_failure_kind(error: &TransportError) -> ProgressFailureKind {
    match error {
        TransportError::HttpStatus { status, .. } => ProgressFailureKind::HttpStatus(*status),
        TransportError::HeaderTimeout { .. } | TransportError::BodyTimeout { .. } => {
            ProgressFailureKind::Timeout
        }
        TransportError::Cancelled { .. } => ProgressFailureKind::Cancelled,
        TransportError::BodyStream { .. } => ProgressFailureKind::BodyStream,
        TransportError::DownloadPayloadMismatch { .. }
        | TransportError::UploadPayloadMismatch { .. } => ProgressFailureKind::PayloadMismatch,
        TransportError::Request { .. }
        | TransportError::MetadataBodyTooLarge { .. }
        | TransportError::MetadataJson { .. }
        | TransportError::MetadataStructure { .. }
        | TransportError::MetadataUnsupported
        | TransportError::InvalidBaseUrl(_)
        | TransportError::InvalidRequestContext
        | TransportError::ClientBuild(_) => ProgressFailureKind::Request,
    }
}

fn update_http_version(result: &mut RunResult, version: Option<&str>) {
    merge_metadata(&mut result.target.http_version, version);
}

fn update_ip_family(result: &mut RunResult, family: Option<&str>) {
    merge_metadata(&mut result.target.ip_family, family);
}

fn merge_metadata(current: &mut Option<String>, observed: Option<&str>) {
    let Some(observed) = observed else {
        return;
    };
    match current.as_deref() {
        None => *current = Some(observed.to_owned()),
        Some("mixed") => {}
        Some(existing) if existing == observed => {}
        Some(_) => *current = Some("mixed".to_owned()),
    }
}

fn metadata_diagnostic(error: &TransportError) -> String {
    match error {
        TransportError::HeaderTimeout { endpoint, .. } => {
            metadata_endpoint_diagnostic(endpoint, "timed out waiting for response headers")
        }
        TransportError::BodyTimeout { endpoint, .. } => {
            metadata_endpoint_diagnostic(endpoint, "timed out while reading the response body")
        }
        TransportError::Request { endpoint, .. } => {
            metadata_endpoint_diagnostic(endpoint, "HTTP request failed")
        }
        TransportError::HttpStatus {
            endpoint, status, ..
        } => metadata_endpoint_diagnostic(endpoint, &format!("HTTP status {status}")),
        TransportError::BodyStream { endpoint, .. } => {
            metadata_endpoint_diagnostic(endpoint, "response body stream failed")
        }
        TransportError::MetadataBodyTooLarge { endpoint, limit } => {
            metadata_endpoint_diagnostic(endpoint, &format!("response body exceeds {limit} bytes"))
        }
        TransportError::MetadataJson { endpoint, .. } => {
            metadata_endpoint_diagnostic(endpoint, "response body is not valid JSON")
        }
        TransportError::MetadataStructure { endpoint, .. } => {
            metadata_endpoint_diagnostic(endpoint, "response JSON has an invalid structure")
        }
        TransportError::Cancelled { .. } => "metadata collection was cancelled".to_owned(),
        TransportError::MetadataUnsupported => {
            "metadata collection is unavailable for this transport".to_owned()
        }
        TransportError::InvalidBaseUrl(_)
        | TransportError::InvalidRequestContext
        | TransportError::ClientBuild(_) => {
            "metadata collection failed before a safe request could be built".to_owned()
        }
        TransportError::DownloadPayloadMismatch { .. }
        | TransportError::UploadPayloadMismatch { .. } => {
            "metadata collection failed with an unexpected payload error".to_owned()
        }
    }
}

fn metadata_endpoint_diagnostic(endpoint: &str, detail: &str) -> String {
    format!(
        "metadata collection failed for endpoint {}: {detail}",
        redact_endpoint(endpoint)
    )
}

fn rpki_error_result(error: &TransportError) -> RpkiReachability {
    let endpoint = transport_error_endpoint(error);
    let host = endpoint.and_then(endpoint_host);
    let detail = match error {
        TransportError::HeaderTimeout { endpoint, .. } => format!(
            "timed out waiting for response headers from endpoint {}",
            redact_endpoint(endpoint)
        ),
        TransportError::BodyTimeout { endpoint, .. } => format!(
            "timed out while reading the response body from endpoint {}",
            redact_endpoint(endpoint)
        ),
        TransportError::Request { endpoint, .. } => {
            format!(
                "HTTP request failed for endpoint {}",
                redact_endpoint(endpoint)
            )
        }
        TransportError::HttpStatus {
            endpoint, status, ..
        } => format!(
            "endpoint {} returned HTTP status {status}",
            redact_endpoint(endpoint)
        ),
        TransportError::BodyStream { endpoint, .. } => format!(
            "response body stream failed for endpoint {}",
            redact_endpoint(endpoint)
        ),
        TransportError::Cancelled { .. } => "RPKI invalid-route check was cancelled".to_owned(),
        _ => "RPKI invalid-route check failed before a safe response was available".to_owned(),
    };
    RpkiReachability {
        status: RpkiReachabilityStatus::Error,
        host,
        detail: Some(detail),
    }
}

fn transport_error_endpoint(error: &TransportError) -> Option<&str> {
    match error {
        TransportError::HeaderTimeout { endpoint, .. }
        | TransportError::BodyTimeout { endpoint, .. }
        | TransportError::Request { endpoint, .. }
        | TransportError::HttpStatus { endpoint, .. }
        | TransportError::BodyStream { endpoint, .. }
        | TransportError::DownloadPayloadMismatch { endpoint, .. }
        | TransportError::UploadPayloadMismatch { endpoint, .. }
        | TransportError::MetadataBodyTooLarge { endpoint, .. }
        | TransportError::MetadataJson { endpoint, .. }
        | TransportError::MetadataStructure { endpoint, .. } => Some(endpoint),
        TransportError::Cancelled { .. }
        | TransportError::MetadataUnsupported
        | TransportError::InvalidBaseUrl(_)
        | TransportError::InvalidRequestContext
        | TransportError::ClientBuild(_) => None,
    }
}

fn endpoint_host(endpoint: &str) -> Option<String> {
    let endpoint = redact_endpoint(endpoint);
    let (_, remainder) = endpoint.split_once("://")?;
    let authority = remainder.split('/').next()?;
    let host = authority
        .strip_prefix('[')
        .and_then(|value| value.split_once(']').map(|(host, _)| host))
        .or_else(|| authority.split(':').next())
        .unwrap_or(authority);
    (!host.is_empty()).then(|| host.to_owned())
}

fn rpki_result_diagnostic(rpki: &RpkiReachability) -> String {
    let host = rpki.host.as_deref().unwrap_or("unknown host");
    let detail = rpki.detail.as_deref().unwrap_or("no detail available");
    format!("RPKI invalid-route check failed for {host}: {detail}")
}

fn redact_endpoint(endpoint: &str) -> String {
    let without_fragment = endpoint.split('#').next().unwrap_or(endpoint);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let Some((scheme, remainder)) = without_query.split_once("://") else {
        return "metadata endpoint".to_owned();
    };
    if !matches!(scheme, "http" | "https") {
        return "metadata endpoint".to_owned();
    }
    let (authority, path) = remainder
        .split_once('/')
        .map_or((remainder, "/"), |(authority, path)| (authority, path));
    let authority = authority.rsplit('@').next().unwrap_or_default();
    if authority.is_empty() {
        return "metadata endpoint".to_owned();
    }
    format!("{scheme}://{authority}/{}", path.trim_start_matches('/'))
}

const fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Download => "download",
        Direction::Upload => "upload",
    }
}

fn stage_name(step: MeasurementStep) -> String {
    match step {
        MeasurementStep::Latency { .. } => "latency".to_owned(),
        MeasurementStep::Download { .. } => "download".to_owned(),
        MeasurementStep::Upload { .. } => "upload".to_owned(),
        MeasurementStep::PacketLossUnsupported { .. } => "packet loss".to_owned(),
    }
}
