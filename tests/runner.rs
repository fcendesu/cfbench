use std::collections::VecDeque;
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cfbench::cancellation::CancellationToken;
use cfbench::error::TransportError;
use cfbench::measurement::TimingObservation;
use cfbench::plan::{Direction, MeasurementPlan, MeasurementStep};
use cfbench::progress::{ProgressEvent, ProgressFailureKind, ProgressReporter, ProgressStage};
use cfbench::results::{
    ClientLocation, EdgeLocation, MetadataStatus, NetworkMetadata, RpkiReachability,
    RpkiReachabilityStatus,
};
use cfbench::runner::{
    MeasurementFuture, MeasurementTransport, MetadataFuture, RpkiFuture, Runner, RunnerError,
};

#[derive(Clone)]
struct ScriptedTransport {
    script: Arc<Mutex<VecDeque<Result<TimingObservation, TransportError>>>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
    metadata_result: Arc<Mutex<Option<Result<NetworkMetadata, TransportError>>>>,
    metadata_delay: Duration,
    rpki_result: Arc<Mutex<Option<Result<RpkiReachability, TransportError>>>>,
}

impl ScriptedTransport {
    fn new(script: impl IntoIterator<Item = Result<TimingObservation, TransportError>>) -> Self {
        Self {
            script: Arc::new(Mutex::new(script.into_iter().collect())),
            calls: Arc::new(Mutex::new(Vec::new())),
            metadata_result: Arc::new(Mutex::new(None)),
            metadata_delay: Duration::ZERO,
            rpki_result: Arc::new(Mutex::new(None)),
        }
    }

    fn with_metadata_result(
        mut self,
        result: Result<NetworkMetadata, TransportError>,
        delay: Duration,
    ) -> Self {
        self.metadata_result = Arc::new(Mutex::new(Some(result)));
        self.metadata_delay = delay;
        self
    }

    fn with_rpki_result(mut self, result: Result<RpkiReachability, TransportError>) -> Self {
        self.rpki_result = Arc::new(Mutex::new(Some(result)));
        self
    }

    fn transfer_durations(durations_ms: impl IntoIterator<Item = f64>) -> Self {
        Self::new(durations_ms.into_iter().map(|duration_ms| {
            Ok(TimingObservation::from_millis(
                20.0,
                duration_ms,
                0.0,
                100_000,
                "HTTP/1.1",
            ))
        }))
    }

    fn next(&self, call: &'static str) -> Result<TimingObservation, TransportError> {
        self.calls.lock().unwrap().push(call);
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .expect("script contains one result per expected operation")
    }
}

impl MeasurementTransport for ScriptedTransport {
    fn metadata<'a>(&'a self, cancellation: &'a CancellationToken) -> MetadataFuture<'a> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("metadata");
            if !self.metadata_delay.is_zero() {
                tokio::select! {
                    biased;
                    () = cancellation.cancelled() => {
                        return Err(TransportError::Cancelled { payload_bytes: 0 });
                    }
                    () = tokio::time::sleep(self.metadata_delay) => {}
                }
            }
            self.metadata_result
                .lock()
                .unwrap()
                .take()
                .expect("metadata script contains one result")
        })
    }

    fn rpki_reachability<'a>(&'a self, _: &'a CancellationToken) -> RpkiFuture<'a> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("rpki");
            self.rpki_result
                .lock()
                .unwrap()
                .take()
                .expect("RPKI script contains one result")
        })
    }

    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(async move { self.next("latency") })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        Box::pin(async move { self.next("loaded_latency") })
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        Box::pin(async move { self.next("download") })
    }

    fn upload<'a>(&'a self, _: u64, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(async move { self.next("upload") })
    }
}

fn fixture_error() -> TransportError {
    TransportError::HttpStatus {
        endpoint: "https://user:secret@invalid.rpki.cloudflare.com/?token=private#fragment"
            .to_owned(),
        status: 503,
        payload_bytes: 0,
    }
}

fn plan(steps: Vec<MeasurementStep>) -> MeasurementPlan {
    MeasurementPlan {
        upstream_version: "test",
        upstream_commit: "test",
        steps,
    }
}

fn downloads(groups: &[(u64, u32, bool)]) -> MeasurementPlan {
    plan(
        groups
            .iter()
            .map(|&(bytes, count, bypass_finish)| MeasurementStep::Download {
                bytes,
                count,
                bypass_finish,
            })
            .collect(),
    )
}

fn metadata_fixture() -> NetworkMetadata {
    NetworkMetadata {
        public_ip: Some("2001:db8::1".to_owned()),
        asn: Some(64_496),
        as_organization: Some("Example Network".to_owned()),
        client_location: ClientLocation {
            country_code: Some("ZZ".to_owned()),
            city: Some("Example City".to_owned()),
            ..ClientLocation::default()
        },
        edge: EdgeLocation {
            colo: Some("XYZ".to_owned()),
            country_code: Some("ZZ".to_owned()),
            city: Some("Example Edge".to_owned()),
            ..EdgeLocation::default()
        },
    }
}

#[derive(Clone, Copy)]
enum FinalOperationResult {
    Success,
    Failure,
    Cancelled,
}

struct FinalOperationCancellationTransport {
    parent: CancellationToken,
    result: FinalOperationResult,
    metadata_calls: Arc<AtomicUsize>,
}

impl MeasurementTransport for FinalOperationCancellationTransport {
    fn metadata<'a>(&'a self, _: &'a CancellationToken) -> MetadataFuture<'a> {
        Box::pin(async move {
            self.metadata_calls.fetch_add(1, Ordering::SeqCst);
            Ok(metadata_fixture())
        })
    }

    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(async move {
            self.parent.cancel();
            match self.result {
                FinalOperationResult::Success => {
                    Ok(TimingObservation::from_millis(20.0, 20.0, 10.0, 0, "1.1"))
                }
                FinalOperationResult::Failure => Err(TransportError::HttpStatus {
                    endpoint: "https://fixture.invalid/__down".to_owned(),
                    status: 503,
                    payload_bytes: 0,
                }),
                FinalOperationResult::Cancelled => {
                    Err(TransportError::Cancelled { payload_bytes: 0 })
                }
            }
        })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unreachable!("test plan contains no loaded-latency operation")
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unreachable!("test plan contains no download operation")
    }

    fn upload<'a>(&'a self, _: u64, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        unreachable!("test plan contains no upload operation")
    }
}

fn final_operation_cancellation_transport(
    parent: &CancellationToken,
    result: FinalOperationResult,
) -> (FinalOperationCancellationTransport, Arc<AtomicUsize>) {
    let metadata_calls = Arc::new(AtomicUsize::new(0));
    (
        FinalOperationCancellationTransport {
            parent: parent.clone(),
            result,
            metadata_calls: metadata_calls.clone(),
        },
        metadata_calls,
    )
}

fn one_latency_plan() -> MeasurementPlan {
    plan(vec![MeasurementStep::Latency { packets: 1 }])
}

#[tokio::test]
async fn runner_defaults_to_disabled_metadata_for_backward_compatible_test_transports() {
    let transport =
        ScriptedTransport::new([Ok(TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2"))]);
    let calls = transport.calls.clone();
    let outcome = Runner::new(
        transport,
        plan(vec![MeasurementStep::Latency { packets: 1 }]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.error.is_none());
    assert_eq!(
        outcome.result.target.metadata_status,
        MetadataStatus::Disabled
    );
    assert!(outcome.result.target.metadata.is_none());
    assert_eq!(*calls.lock().unwrap(), ["latency"]);
}

#[tokio::test]
async fn direct_run_reconciles_parent_cancellation_after_final_success_and_skips_metadata() {
    let cancellation = CancellationToken::new();
    let (transport, metadata_calls) =
        final_operation_cancellation_transport(&cancellation, FinalOperationResult::Success);

    let outcome = Runner::new(transport, one_latency_plan())
        .with_loaded_latency(false)
        .with_metadata(true)
        .run(&cancellation)
        .await;

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "run"
    ));
    assert_eq!(outcome.result.raw.initial_latency.len(), 1);
    assert_eq!(
        outcome.result.failures,
        ["measurement cancelled during run"]
    );
    assert_eq!(metadata_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        outcome.result.target.metadata_status,
        MetadataStatus::Unavailable
    );
}

#[tokio::test]
async fn direct_run_with_progress_reconciles_final_cancellation_and_keeps_the_point_event() {
    let cancellation = CancellationToken::new();
    let (transport, _) =
        final_operation_cancellation_transport(&cancellation, FinalOperationResult::Success);
    let (progress, receiver) = ProgressReporter::channel(8);

    let outcome = Runner::new(transport, one_latency_plan())
        .with_loaded_latency(false)
        .run_with_progress(&cancellation, progress)
        .await;
    let events = receiver.into_iter().collect::<Vec<_>>();

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "run"
    ));
    assert_eq!(outcome.result.raw.initial_latency.len(), 1);
    assert_eq!(outcome.result.failures.len(), 1);
    assert_eq!(
        events,
        [ProgressEvent::LatencyCompleted {
            current: 1,
            total: 1,
            latency_ms: 10.0,
        }]
    );
}

#[tokio::test]
async fn final_parent_cancellation_supersedes_prior_failure_and_preserves_history() {
    let cancellation = CancellationToken::new();
    let (transport, metadata_calls) =
        final_operation_cancellation_transport(&cancellation, FinalOperationResult::Failure);

    let outcome = Runner::new(transport, one_latency_plan())
        .with_loaded_latency(false)
        .with_metadata(true)
        .run(&cancellation)
        .await;

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "run"
    ));
    assert_eq!(outcome.result.failures.len(), 2);
    assert!(outcome.result.failures[0].contains("transport failed during latency"));
    assert_eq!(
        outcome.result.failures[1],
        "measurement cancelled during run"
    );
    assert_eq!(metadata_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn final_transport_cancellation_is_not_recorded_twice() {
    let cancellation = CancellationToken::new();
    let (transport, metadata_calls) =
        final_operation_cancellation_transport(&cancellation, FinalOperationResult::Cancelled);

    let outcome = Runner::new(transport, one_latency_plan())
        .with_loaded_latency(false)
        .with_metadata(true)
        .run(&cancellation)
        .await;

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "latency"
    ));
    assert_eq!(outcome.result.failures.len(), 1);
    assert_eq!(
        outcome.result.failures[0],
        "measurement cancelled during latency"
    );
    assert_eq!(metadata_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn metadata_runs_once_after_the_exact_plan_without_affecting_usage_or_summary() {
    let metadata_delay = Duration::from_secs(5);
    let transport = ScriptedTransport::new([
        Ok(TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2")),
        Ok(TimingObservation::from_millis(
            20.0, 500.0, 0.0, 100_000, "2",
        )),
        Ok(TimingObservation::from_millis(
            20.0, 500.0, 0.0, 100_000, "2",
        )),
    ])
    .with_metadata_result(Ok(metadata_fixture()), metadata_delay);
    let calls = transport.calls.clone();
    let outcome = Runner::new(
        transport,
        plan(vec![
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 1,
                bypass_finish: true,
            },
            MeasurementStep::Upload {
                bytes: 100_000,
                count: 1,
                bypass_finish: true,
            },
        ]),
    )
    .with_loaded_latency(false)
    .with_metadata(true)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.error.is_none());
    assert_eq!(
        *calls.lock().unwrap(),
        ["latency", "download", "upload", "metadata"]
    );
    assert_eq!(
        outcome.result.target.metadata_status,
        MetadataStatus::Available
    );
    assert_eq!(outcome.result.target.metadata, Some(metadata_fixture()));
    assert_eq!(outcome.result.raw.initial_latency.len(), 1);
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(outcome.result.raw.upload.len(), 1);
    assert!(outcome.result.summary.download_bps.is_some());
    assert!(outcome.result.summary.upload_bps.is_some());
    assert_eq!(outcome.result.usage.download_payload_bytes, 100_000);
    assert_eq!(outcome.result.usage.upload_payload_bytes, 100_000);
    assert!(outcome.result.usage.duration_ms < metadata_delay.as_secs_f64() * 1_000.0);
}

#[tokio::test]
async fn metadata_failure_is_one_redacted_nonfatal_diagnostic() {
    let transport =
        ScriptedTransport::new([Ok(TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2"))])
            .with_metadata_result(
                Err(TransportError::HttpStatus {
                    endpoint: "https://user:secret@fixture.invalid/meta?token=private#fragment"
                        .to_owned(),
                    status: 503,
                    payload_bytes: 0,
                }),
                Duration::ZERO,
            );
    let calls = transport.calls.clone();
    let outcome = Runner::new(
        transport,
        plan(vec![MeasurementStep::Latency { packets: 1 }]),
    )
    .with_loaded_latency(false)
    .with_metadata(true)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.error.is_none());
    assert!(outcome.result.failures.is_empty());
    assert_eq!(
        outcome.result.target.metadata_status,
        MetadataStatus::Unavailable
    );
    assert!(outcome.result.target.metadata.is_none());
    assert_eq!(*calls.lock().unwrap(), ["latency", "metadata"]);
    assert_eq!(outcome.result.diagnostics.len(), 1);
    let diagnostic = &outcome.result.diagnostics[0];
    assert!(diagnostic.contains("metadata"));
    assert!(diagnostic.contains("https://fixture.invalid/meta"));
    assert!(diagnostic.contains("HTTP status 503"));
    assert!(!diagnostic.contains("secret"));
    assert!(!diagnostic.contains("private"));
    assert!(!diagnostic.contains("fragment"));
}

#[tokio::test]
async fn enabled_rpki_failure_is_diagnostic_not_a_terminal_measurement_error() {
    let transport = ScriptedTransport::new([]).with_rpki_result(Err(fixture_error()));
    let calls = transport.calls.clone();

    let outcome = Runner::new(transport, plan(Vec::new()))
        .with_loaded_latency(false)
        .with_rpki_check(true)
        .run(&CancellationToken::new())
        .await;

    assert!(outcome.error.is_none());
    assert!(outcome.result.failures.is_empty());
    assert_eq!(outcome.result.rpki.status, RpkiReachabilityStatus::Error);
    assert_eq!(
        outcome.result.rpki.host.as_deref(),
        Some("invalid.rpki.cloudflare.com")
    );
    assert_eq!(*calls.lock().unwrap(), ["rpki"]);
    assert_eq!(outcome.result.diagnostics.len(), 1);
    let diagnostic = &outcome.result.diagnostics[0];
    assert!(diagnostic.contains("RPKI"));
    assert!(diagnostic.contains("https://invalid.rpki.cloudflare.com/"));
    assert!(diagnostic.contains("HTTP status 503"));
    assert!(!diagnostic.contains("secret"));
    assert!(!diagnostic.contains("private"));
    assert!(!diagnostic.contains("fragment"));
}

#[tokio::test(start_paused = true)]
async fn rpki_runs_after_the_plan_without_affecting_core_summary_or_usage() {
    let transport = ScriptedTransport::new([Ok(TimingObservation::from_millis(
        20.0, 500.0, 0.0, 100_000, "2",
    ))])
    .with_rpki_result(Ok(RpkiReachability {
        status: RpkiReachabilityStatus::Unreachable,
        host: Some("invalid.rpki.cloudflare.com".to_owned()),
        detail: Some("request could not reach the invalid-route host".to_owned()),
    }));
    let calls = transport.calls.clone();

    let outcome = Runner::new(
        transport,
        plan(vec![MeasurementStep::Download {
            bytes: 100_000,
            count: 1,
            bypass_finish: true,
        }]),
    )
    .with_loaded_latency(false)
    .with_rpki_check(true)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.error.is_none());
    assert_eq!(*calls.lock().unwrap(), ["download", "rpki"]);
    assert_eq!(
        outcome.result.rpki.status,
        RpkiReachabilityStatus::Unreachable
    );
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert!(outcome.result.summary.download_bps.is_some());
    assert_eq!(outcome.result.usage.download_payload_bytes, 100_000);
    assert_eq!(outcome.result.usage.upload_payload_bytes, 0);
    assert!(outcome.result.usage.duration_ms < 1_000.0);
}

#[tokio::test]
async fn metadata_failure_does_not_replace_an_existing_terminal_measurement_error() {
    let transport = ScriptedTransport::new([Err(TransportError::BodyTimeout {
        endpoint: "https://fixture.invalid/__down".to_owned(),
        payload_bytes: 25_000,
    })])
    .with_metadata_result(
        Err(TransportError::HeaderTimeout {
            endpoint: "https://fixture.invalid/meta".to_owned(),
            payload_bytes: 0,
        }),
        Duration::ZERO,
    );
    let calls = transport.calls.clone();
    let outcome = Runner::new(
        transport,
        plan(vec![MeasurementStep::Download {
            bytes: 100_000,
            count: 1,
            bypass_finish: true,
        }]),
    )
    .with_loaded_latency(false)
    .with_metadata(true)
    .run(&CancellationToken::new())
    .await;

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Transport { ref stage, .. }) if stage == "download"
    ));
    assert_eq!(outcome.result.failures.len(), 1);
    assert!(outcome.result.failures[0].contains("during download"));
    assert_eq!(outcome.result.diagnostics.len(), 1);
    assert!(outcome.result.diagnostics[0].contains("metadata"));
    assert_eq!(outcome.result.usage.download_payload_bytes, 25_000);
    assert_eq!(*calls.lock().unwrap(), ["download", "metadata"]);
}

#[tokio::test]
async fn cancellation_before_plan_execution_skips_metadata_io() {
    let transport =
        ScriptedTransport::new([]).with_metadata_result(Ok(metadata_fixture()), Duration::ZERO);
    let calls = transport.calls.clone();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let outcome = Runner::new(
        transport,
        plan(vec![MeasurementStep::Latency { packets: 1 }]),
    )
    .with_loaded_latency(false)
    .with_metadata(true)
    .run(&cancellation)
    .await;

    assert!(matches!(outcome.error, Some(RunnerError::Cancelled { .. })));
    assert_eq!(
        outcome.result.target.metadata_status,
        MetadataStatus::Unavailable
    );
    assert!(outcome.result.target.metadata.is_none());
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancelled_transport_outcome_skips_metadata_without_token_cancellation() {
    let transport = ScriptedTransport::new([Err(TransportError::Cancelled { payload_bytes: 0 })])
        .with_metadata_result(Ok(metadata_fixture()), Duration::ZERO);
    let calls = transport.calls.clone();
    let cancellation = CancellationToken::new();

    let outcome = Runner::new(
        transport,
        plan(vec![MeasurementStep::Latency { packets: 1 }]),
    )
    .with_loaded_latency(false)
    .with_metadata(true)
    .run(&cancellation)
    .await;

    assert!(!cancellation.is_cancelled());
    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "latency"
    ));
    assert_eq!(outcome.result.failures.len(), 1);
    assert_eq!(
        outcome.result.target.metadata_status,
        MetadataStatus::Unavailable
    );
    assert!(outcome.result.target.metadata.is_none());
    assert_eq!(*calls.lock().unwrap(), ["latency"]);
}

#[tokio::test]
async fn cancellation_during_metadata_is_awaited_and_returns_a_cancelled_outcome() {
    let transport = ScriptedTransport::new([])
        .with_metadata_result(Ok(metadata_fixture()), Duration::from_secs(60));
    let calls = transport.calls.clone();
    let cancellation = CancellationToken::new();
    let run_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        Runner::new(transport, plan(Vec::new()))
            .with_loaded_latency(false)
            .with_metadata(true)
            .run(&run_cancellation)
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if calls.lock().unwrap().as_slice() == ["metadata"] {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("metadata request starts");
    cancellation.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("runner awaits metadata cancellation")
        .expect("runner task joins");

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "metadata"
    ));
    assert_eq!(outcome.result.failures.len(), 1);
    assert_eq!(
        outcome.result.target.metadata_status,
        MetadataStatus::Unavailable
    );
    assert!(outcome.result.target.metadata.is_none());
    assert!(outcome.result.diagnostics.is_empty());
    assert_eq!(*calls.lock().unwrap(), ["metadata"]);
}

#[tokio::test]
async fn cancellation_during_metadata_supersedes_a_prior_measurement_error_terminally() {
    let transport = ScriptedTransport::new([
        Ok(TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2")),
        Err(TransportError::BodyTimeout {
            endpoint: "https://fixture.invalid/__down".to_owned(),
            payload_bytes: 25_000,
        }),
    ])
    .with_metadata_result(Ok(metadata_fixture()), Duration::from_secs(60));
    let calls = transport.calls.clone();
    let cancellation = CancellationToken::new();
    let run_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        Runner::new(
            transport,
            plan(vec![
                MeasurementStep::Latency { packets: 1 },
                MeasurementStep::Download {
                    bytes: 100_000,
                    count: 1,
                    bypass_finish: true,
                },
            ]),
        )
        .with_loaded_latency(false)
        .with_metadata(true)
        .run(&run_cancellation)
        .await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if calls.lock().unwrap().as_slice() == ["latency", "download", "metadata"] {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("metadata request starts after the failed measurement");
    cancellation.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("runner awaits metadata cancellation")
        .expect("runner task joins");

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "metadata"
    ));
    assert_eq!(outcome.result.raw.initial_latency.len(), 1);
    assert_eq!(outcome.result.usage.download_payload_bytes, 25_000);
    assert_eq!(outcome.result.failures.len(), 2);
    assert!(outcome.result.failures[0].contains("during download"));
    assert_eq!(
        outcome.result.failures[1],
        "measurement cancelled during metadata"
    );
    assert!(outcome.result.diagnostics.is_empty());
}

#[tokio::test]
async fn runner_records_negotiated_ip_family_and_contract_http_version() {
    let observation =
        TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2").with_ip_family("ipv6");
    let outcome = Runner::new(
        ScriptedTransport::new([Ok(observation)]),
        plan(vec![MeasurementStep::Latency { packets: 1 }]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert_eq!(outcome.result.target.ip_family.as_deref(), Some("ipv6"));
    assert_eq!(outcome.result.target.http_version.as_deref(), Some("2"));
}

#[tokio::test]
async fn runner_aggregates_mixed_http_versions_and_ip_families() {
    let observations = [
        TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2").with_ip_family("ipv6"),
        TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2").with_ip_family("ipv6"),
        TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "1.1").with_ip_family("ipv4"),
        TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "3").with_ip_family("ipv6"),
    ];
    let outcome = Runner::new(
        ScriptedTransport::new(observations.into_iter().map(Ok)),
        plan(vec![MeasurementStep::Latency { packets: 4 }]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert_eq!(outcome.result.target.ip_family.as_deref(), Some("mixed"));
    assert_eq!(outcome.result.target.http_version.as_deref(), Some("mixed"));
}

#[tokio::test]
async fn runner_stamps_every_accepted_point_from_one_nondecreasing_run_clock() {
    let observations = [
        TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2"),
        TimingObservation::from_millis(21.0, 31.0, 10.0, 0, "2"),
        TimingObservation::from_millis(20.0, 500.0, 0.0, 100_000, "2"),
    ];
    let outcome = Runner::new(
        ScriptedTransport::new(observations.into_iter().map(Ok)),
        plan(vec![
            MeasurementStep::Latency { packets: 2 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 1,
                bypass_finish: true,
            },
        ]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.error.is_none());
    assert!(humantime::parse_rfc3339(&outcome.result.started_at).is_ok());
    let timestamps = outcome
        .result
        .raw
        .latency
        .iter()
        .map(|point| point.measured_at_unix_ms)
        .chain(
            outcome
                .result
                .raw
                .download
                .iter()
                .map(|point| point.measured_at_unix_ms),
        )
        .collect::<Vec<_>>();

    assert_eq!(timestamps.len(), 3);
    assert!(timestamps.iter().all(|timestamp| *timestamp > 0));
    assert!(timestamps.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[tokio::test]
async fn runner_ignores_missing_metadata_and_no_observations_remain_null() {
    let mut missing = TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2");
    missing.http_version = None;
    let outcome = Runner::new(
        ScriptedTransport::new([
            Ok(TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2").with_ip_family("ipv6")),
            Ok(missing),
        ]),
        plan(vec![MeasurementStep::Latency { packets: 2 }]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;
    assert_eq!(outcome.result.target.ip_family.as_deref(), Some("ipv6"));
    assert_eq!(outcome.result.target.http_version.as_deref(), Some("2"));

    let no_observations = Runner::new(
        ScriptedTransport::new([]),
        plan(vec![MeasurementStep::PacketLossUnsupported {
            packets: 1_000,
            responses_wait_ms: 3_000,
        }]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;
    assert!(no_observations.result.target.ip_family.is_none());
    assert!(no_observations.result.target.http_version.is_none());
}

#[tokio::test]
async fn conversion_rejected_observation_still_contributes_transport_metadata() {
    let first = TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "2").with_ip_family("ipv6");
    let rejected = TimingObservation::from_millis(f64::NAN, 30.0, 10.0, 0, "1.1")
        .with_ip_family("ipv4")
        .with_endpoint("https://user:password@fixture.invalid/__down?secret=value#fragment");
    let outcome = Runner::new(
        ScriptedTransport::new([Ok(first), Ok(rejected)]),
        plan(vec![MeasurementStep::Latency { packets: 2 }]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Conversion { .. })
    ));
    assert_eq!(outcome.result.target.ip_family.as_deref(), Some("mixed"));
    assert_eq!(outcome.result.target.http_version.as_deref(), Some("mixed"));
    let error = outcome.error.as_ref().unwrap();
    assert!(error.to_string().contains("during latency"));
    assert!(error.to_string().contains("https://fixture.invalid/__down"));
    assert!(!error.to_string().contains("password"));
    assert!(!error.to_string().contains("secret"));
    assert!(
        error
            .source()
            .is_some_and(|source| source.is::<cfbench::measurement::MeasurementConversionError>())
    );
}

#[tokio::test]
async fn runner_transport_error_retains_typed_source_and_endpoint() {
    let outcome = Runner::new(
        ScriptedTransport::new([Err(TransportError::BodyTimeout {
            endpoint: "https://fixture.invalid/__down".to_owned(),
            payload_bytes: 0,
        })]),
        plan(vec![MeasurementStep::Latency { packets: 1 }]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    let error = outcome.error.as_ref().unwrap();
    assert!(error.to_string().contains("during latency"));
    assert!(error.to_string().contains("https://fixture.invalid/__down"));
    assert!(
        error
            .source()
            .is_some_and(|source| source.is::<TransportError>())
    );
}

#[tokio::test]
async fn finish_uses_strict_minimum_after_whole_group() {
    let transport = ScriptedTransport::transfer_durations([1001.0, 1500.0, 1000.0, 1100.0]);
    let outcome = Runner::new(
        transport,
        downloads(&[(100_000, 3, false), (1_000_000, 1, false)]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;
    assert_eq!(outcome.result.raw.download.len(), 4);

    let transport = ScriptedTransport::transfer_durations([1001.0, 1500.0, 1000.01]);
    let outcome = Runner::new(
        transport,
        downloads(&[(100_000, 3, false), (1_000_000, 1, false)]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;
    assert_eq!(outcome.result.raw.download.len(), 3);
}

#[tokio::test]
async fn bypass_and_direction_finish_states_are_independent() {
    let transport = ScriptedTransport::transfer_durations([2000.0, 1200.0, 1300.0, 500.0]);
    let calls = transport.calls.clone();
    let outcome = Runner::new(
        transport,
        plan(vec![
            MeasurementStep::Download {
                bytes: 100_000,
                count: 1,
                bypass_finish: true,
            },
            MeasurementStep::Download {
                bytes: 1_000_000,
                count: 2,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 100_000,
                count: 1,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 10_000_000,
                count: 1,
                bypass_finish: false,
            },
        ]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.raw.download.len(), 3);
    assert_eq!(outcome.result.raw.upload.len(), 1);
    assert_eq!(
        *calls.lock().unwrap(),
        ["download", "download", "download", "upload"]
    );
}

#[tokio::test]
async fn later_latency_phase_replaces_initial_estimate() {
    let observations = (0..21).map(|index| {
        Ok(TimingObservation::from_millis(
            20.0 + f64::from(index),
            30.0,
            10.0,
            0,
            "HTTP/1.1",
        ))
    });
    let outcome = Runner::new(
        ScriptedTransport::new(observations),
        plan(vec![
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::PacketLossUnsupported {
                packets: 1000,
                responses_wait_ms: 3000,
            },
            MeasurementStep::Latency { packets: 20 },
        ]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.result.raw.initial_latency.is_empty());
    assert_eq!(outcome.result.raw.latency.len(), 20);
    assert_eq!(outcome.result.raw.latency[0].ping_ms, 11.0);
}

#[tokio::test]
async fn failed_later_stage_preserves_completed_points() {
    let transport = ScriptedTransport::new([
        Ok(TimingObservation::from_millis(
            20.0, 20.0, 10.0, 0, "HTTP/1.1",
        )),
        Ok(TimingObservation::from_millis(
            20.0, 500.0, 0.0, 100_000, "HTTP/1.1",
        )),
        Err(TransportError::BodyTimeout {
            endpoint: "https://fixture.invalid/__down".to_owned(),
            payload_bytes: 25_000,
        }),
    ]);
    let outcome = Runner::new(
        transport,
        plan(vec![
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 2,
                bypass_finish: false,
            },
        ]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert!(matches!(outcome.error, Some(RunnerError::Transport { .. })));
    assert_eq!(outcome.result.raw.initial_latency.len(), 1);
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(outcome.result.usage.download_payload_bytes, 125_000);
    assert_eq!(outcome.result.failures.len(), 1);
}

#[tokio::test]
async fn successful_transfer_usage_is_not_double_counted() {
    let outcome = Runner::new(
        ScriptedTransport::new([Ok(TimingObservation::from_millis(
            20.0, 500.0, 0.0, 100_000, "HTTP/1.1",
        ))]),
        downloads(&[(100_000, 1, true)]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.usage.download_payload_bytes, 100_000);
}

#[tokio::test]
async fn failed_twenty_packet_phase_keeps_completed_replacement_points() {
    let transport = ScriptedTransport::new([
        Ok(TimingObservation::from_millis(
            20.0, 20.0, 10.0, 0, "HTTP/1.1",
        )),
        Ok(TimingObservation::from_millis(
            30.0, 30.0, 10.0, 0, "HTTP/1.1",
        )),
        Err(TransportError::HeaderTimeout {
            endpoint: "https://fixture.invalid/__down".to_owned(),
            payload_bytes: 0,
        }),
    ]);
    let outcome = Runner::new(
        transport,
        plan(vec![
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::Latency { packets: 20 },
        ]),
    )
    .with_loaded_latency(false)
    .run(&CancellationToken::new())
    .await;

    assert!(outcome.result.raw.initial_latency.is_empty());
    assert_eq!(outcome.result.raw.latency.len(), 1);
    assert_eq!(outcome.result.raw.latency[0].ping_ms, 20.0);
}

#[tokio::test]
async fn progress_reports_only_accepted_points_with_phase_local_counters() {
    let observations = [
        TimingObservation::from_millis(20.0, 30.0, 10.0, 0, "HTTP/1.1"),
        TimingObservation::from_millis(20.0, 500.0, 0.0, 100_000, "HTTP/1.1"),
        TimingObservation::from_millis(30.0, 40.0, 10.0, 0, "HTTP/1.1"),
        TimingObservation::from_millis(31.0, 41.0, 10.0, 0, "HTTP/1.1"),
        TimingObservation::from_millis(20.0, 500.0, 0.0, 100_000, "HTTP/1.1"),
        TimingObservation::from_millis(20.0, 500.0, 0.0, 100_000, "HTTP/1.1"),
    ];
    let runner = Runner::new(
        ScriptedTransport::new(observations.into_iter().map(Ok)),
        plan(vec![
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 1,
                bypass_finish: true,
            },
            MeasurementStep::Latency { packets: 2 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 2,
                bypass_finish: false,
            },
        ]),
    )
    .with_loaded_latency(false);
    let (progress, receiver) = ProgressReporter::channel(256);

    let outcome = runner
        .run_with_progress(&CancellationToken::new(), progress)
        .await;
    let events: Vec<_> = receiver.into_iter().collect();

    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.raw.latency.len(), 2);
    assert_eq!(outcome.result.raw.download.len(), 3);
    assert_eq!(
        events,
        vec![
            ProgressEvent::LatencyCompleted {
                current: 1,
                total: 1,
                latency_ms: 10.0,
            },
            ProgressEvent::TransferCompleted {
                direction: Direction::Download,
                requested_bytes: 100_000,
                current: 1,
                total: 1,
                bps: 1_608_000,
                adjusted_duration_ms: 500.0,
            },
            ProgressEvent::LatencyCompleted {
                current: 1,
                total: 2,
                latency_ms: 20.0,
            },
            ProgressEvent::LatencyCompleted {
                current: 2,
                total: 2,
                latency_ms: 21.0,
            },
            ProgressEvent::TransferCompleted {
                direction: Direction::Download,
                requested_bytes: 100_000,
                current: 1,
                total: 2,
                bps: 1_608_000,
                adjusted_duration_ms: 500.0,
            },
            ProgressEvent::TransferCompleted {
                direction: Direction::Download,
                requested_bytes: 100_000,
                current: 2,
                total: 2,
                bps: 1_608_000,
                adjusted_duration_ms: 500.0,
            },
        ]
    );
}

#[tokio::test]
async fn progress_reports_one_safe_failure_and_preserves_accepted_points() {
    let runner = Runner::new(
        ScriptedTransport::new([
            Ok(TimingObservation::from_millis(
                20.0, 500.0, 0.0, 100_000, "HTTP/1.1",
            )),
            Err(TransportError::HttpStatus {
                endpoint: "https://fixture.invalid/__down".to_owned(),
                status: 403,
                payload_bytes: 0,
            }),
        ]),
        downloads(&[(100_000, 2, false)]),
    )
    .with_loaded_latency(false);
    let (progress, receiver) = ProgressReporter::channel(256);

    let outcome = runner
        .run_with_progress(&CancellationToken::new(), progress)
        .await;
    let events: Vec<_> = receiver.into_iter().collect();

    assert!(matches!(outcome.error, Some(RunnerError::Transport { .. })));
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::RequestFailed { .. }))
            .count(),
        1
    );
    assert_eq!(
        events.last(),
        Some(&ProgressEvent::RequestFailed {
            stage: ProgressStage::Transfer {
                direction: Direction::Download,
                requested_bytes: 100_000,
            },
            current: Some(2),
            total: Some(2),
            kind: ProgressFailureKind::HttpStatus(403),
        })
    );
}

#[tokio::test]
async fn progress_reports_direction_finished_only_at_the_first_skipped_group() {
    let runner = Runner::new(
        ScriptedTransport::transfer_durations([1_001.0]),
        downloads(&[
            (100_000, 1, false),
            (1_000_000, 1, false),
            (10_000_000, 1, false),
        ]),
    )
    .with_loaded_latency(false);
    let (progress, receiver) = ProgressReporter::channel(256);

    let outcome = runner
        .run_with_progress(&CancellationToken::new(), progress)
        .await;
    let events: Vec<_> = receiver.into_iter().collect();

    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::DirectionFinished { .. }))
            .collect::<Vec<_>>(),
        vec![&ProgressEvent::DirectionFinished {
            direction: Direction::Download,
        }]
    );
}

#[tokio::test]
async fn packet_loss_placeholder_does_not_emit_progress_or_a_measurement_point() {
    let runner = Runner::new(
        ScriptedTransport::new([]),
        plan(vec![MeasurementStep::PacketLossUnsupported {
            packets: 1_000,
            responses_wait_ms: 3_000,
        }]),
    )
    .with_loaded_latency(false);
    let (progress, receiver) = ProgressReporter::channel(256);

    let outcome = runner
        .run_with_progress(&CancellationToken::new(), progress)
        .await;
    let events: Vec<_> = receiver.into_iter().collect();

    assert!(outcome.error.is_none());
    assert!(outcome.result.summary.packet_loss_ratio.is_none());
    assert!(events.is_empty());
}

#[tokio::test]
async fn initial_and_later_hundred_kilobyte_groups_have_independent_counters() {
    let runner = Runner::new(
        ScriptedTransport::transfer_durations(std::iter::repeat_n(500.0, 10)),
        downloads(&[(100_000, 1, true), (100_000, 9, false)]),
    )
    .with_loaded_latency(false);
    let (progress, receiver) = ProgressReporter::channel(256);

    let outcome = runner
        .run_with_progress(&CancellationToken::new(), progress)
        .await;
    let events: Vec<_> = receiver.into_iter().collect();

    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.raw.download.len(), 10);
    assert!(matches!(
        events.first(),
        Some(ProgressEvent::TransferCompleted {
            direction: Direction::Download,
            requested_bytes: 100_000,
            current: 1,
            total: 1,
            ..
        })
    ));
    assert!(matches!(
        events.get(1),
        Some(ProgressEvent::TransferCompleted {
            direction: Direction::Download,
            requested_bytes: 100_000,
            current: 1,
            total: 9,
            ..
        })
    ));
    assert!(matches!(
        events.last(),
        Some(ProgressEvent::TransferCompleted {
            direction: Direction::Download,
            requested_bytes: 100_000,
            current: 9,
            total: 9,
            ..
        })
    ));
}
