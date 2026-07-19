use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use thiserror::Error;

use crate::cancellation::CancellationToken;
use crate::error::TransportError;
use crate::measurement::loaded_latency::{LoadedProbeOutcome, spawn_loaded_probe_loop};
use crate::measurement::{TimingObservation, bandwidth_point, latency_point};
use crate::plan::{Direction, MeasurementPlan, MeasurementStep};
use crate::results::{RunResult, reduce};
use crate::transport::ReqwestTransport;

const MIN_FINISH_DURATION_MS: f64 = 1_000.0;
const MIN_LOADED_GROUP_DURATION_MS: f64 = 250.0;

/// A boxed measurement operation used by scripted transports in runner tests.
pub type MeasurementFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TimingObservation, TransportError>> + Send + 'a>>;

/// The transport surface required by ordered measurement orchestration.
pub trait MeasurementTransport: Send + Sync + 'static {
    fn latency<'a>(&'a self, cancellation: &'a CancellationToken) -> MeasurementFuture<'a>;

    fn loaded_latency<'a>(
        &'a self,
        direction: Direction,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a>;

    fn download<'a>(
        &'a self,
        bytes: u64,
        during: Option<&'a str>,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a>;

    fn upload<'a>(
        &'a self,
        bytes: u64,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a>;
}

impl MeasurementTransport for ReqwestTransport {
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
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        Box::pin(ReqwestTransport::download(
            self,
            bytes,
            during,
            cancellation,
        ))
    }

    fn upload<'a>(
        &'a self,
        bytes: u64,
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
    #[error("transport failed during {stage}: {message}")]
    Transport { stage: String, message: String },
    #[error("measurement conversion failed during {stage}: {message}")]
    Conversion { stage: String, message: String },
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
        }
    }

    pub fn with_loaded_latency(mut self, enabled: bool) -> Self {
        self.loaded_latency = enabled;
        self
    }

    pub async fn run(&self, cancellation: &CancellationToken) -> RunOutcome {
        let started = Instant::now();
        let mut result = RunResult::empty();
        let error = self.execute(&mut result, cancellation).await;
        result.usage.duration_ms = started.elapsed().as_secs_f64() * 1_000.0;
        result.summary = reduce(&result.raw);

        RunOutcome { result, error }
    }

    async fn execute(
        &self,
        result: &mut RunResult,
        cancellation: &CancellationToken,
    ) -> Option<RunnerError> {
        let mut download_finished = false;
        let mut upload_finished = false;

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
                    self.run_latency_phase(packets, result, cancellation).await
                }
                MeasurementStep::Download {
                    bytes,
                    count,
                    bypass_finish,
                } if !download_finished => {
                    match self
                        .run_transfer_group(Direction::Download, bytes, count, result, cancellation)
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
                        .run_transfer_group(Direction::Upload, bytes, count, result, cancellation)
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
                MeasurementStep::PacketLossUnsupported { .. }
                | MeasurementStep::Download { .. }
                | MeasurementStep::Upload { .. } => None,
            };

            if let Some(error) = error {
                return Some(record_failure(result, error));
            }
        }
        None
    }

    async fn run_latency_phase(
        &self,
        packets: u32,
        result: &mut RunResult,
        cancellation: &CancellationToken,
    ) -> Option<RunnerError> {
        let initial_phase =
            packets == 1 && result.raw.initial_latency.is_empty() && result.raw.latency.is_empty();
        if !initial_phase {
            result.raw.initial_latency.clear();
            result.raw.latency.clear();
            result.raw.latency.reserve(packets as usize);
        }

        for _ in 0..packets {
            let observation = match self.transport.latency(cancellation).await {
                Ok(observation) => observation,
                Err(error) => return Some(map_transport_error("latency", error)),
            };
            let ip_family = observation.ip_family.clone();
            let point = match latency_point(observation) {
                Ok(point) => point,
                Err(error) => {
                    return Some(RunnerError::Conversion {
                        stage: "latency".to_owned(),
                        message: error.to_string(),
                    });
                }
            };
            update_ip_family(result, ip_family.as_deref());
            update_http_version(result, point.http_version.as_deref());
            if initial_phase {
                result.raw.initial_latency.push(point);
            } else {
                result.raw.latency.push(point);
            }
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
    ) -> Result<bool, RunnerError> {
        let group_cancellation = cancellation.child_token();
        let probe_task = self.loaded_latency.then(|| {
            spawn_loaded_probe_loop(
                self.transport.clone(),
                direction,
                group_cancellation.clone(),
            )
        });
        let mut durations = Vec::with_capacity(count as usize);
        let mut terminal_error = None;

        for _ in 0..count {
            let observation = match direction {
                Direction::Download => self.transport.download(bytes, None, cancellation).await,
                Direction::Upload => self.transport.upload(bytes, cancellation).await,
            };
            let observation = match observation {
                Ok(observation) => observation,
                Err(error) => {
                    terminal_error = Some(map_transport_error(direction_name(direction), error));
                    break;
                }
            };
            let ip_family = observation.ip_family.clone();
            let point = match bandwidth_point(direction, bytes, observation) {
                Ok(point) => point,
                Err(error) => {
                    terminal_error = Some(RunnerError::Conversion {
                        stage: direction_name(direction).to_owned(),
                        message: error.to_string(),
                    });
                    break;
                }
            };
            update_ip_family(result, ip_family.as_deref());
            update_http_version(result, point.http_version.as_deref());
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
        }

        group_cancellation.cancel();
        let probe_outcome = match probe_task {
            Some(task) => match task.await {
                Ok(outcome) => outcome,
                Err(error) => LoadedProbeOutcome {
                    points: Vec::new(),
                    diagnostics: vec![format!("loaded latency task failed to join: {error}")],
                },
            },
            None => LoadedProbeOutcome::default(),
        };
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
        TransportError::Cancelled => RunnerError::Cancelled {
            stage: stage.to_owned(),
        },
        error => RunnerError::Transport {
            stage: stage.to_owned(),
            message: error.to_string(),
        },
    }
}

fn record_failure(result: &mut RunResult, error: RunnerError) -> RunnerError {
    result.failures.push(error.to_string());
    error
}

fn update_http_version(result: &mut RunResult, version: Option<&str>) {
    if result.target.http_version.is_none() {
        result.target.http_version = version.map(ToOwned::to_owned);
    }
}

fn update_ip_family(result: &mut RunResult, family: Option<&str>) {
    if result.target.ip_family.is_none() {
        result.target.ip_family = family.map(ToOwned::to_owned);
    }
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
