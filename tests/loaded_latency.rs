use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cfbench::cancellation::CancellationToken;
use cfbench::error::TransportError;
use cfbench::measurement::TimingObservation;
use cfbench::plan::{Direction, MeasurementPlan, MeasurementStep};
use cfbench::progress::{ProgressEvent, ProgressReporter};
use cfbench::runner::{MeasurementFuture, MeasurementTransport, Runner};

type ScriptedTransfer = Result<(Duration, f64), TransportError>;

#[derive(Clone)]
struct TimedTransport {
    transfers: Arc<Mutex<VecDeque<ScriptedTransfer>>>,
    probe_starts: Arc<AtomicUsize>,
    active_probes: Arc<AtomicUsize>,
    fail_probe: bool,
    transfer_http_version: &'static str,
    transfer_ip_family: &'static str,
    probe_http_version: &'static str,
    probe_ip_family: &'static str,
    invalid_probe: bool,
}

impl TimedTransport {
    fn new(transfers: impl IntoIterator<Item = (Duration, f64)>) -> Self {
        Self {
            transfers: Arc::new(Mutex::new(transfers.into_iter().map(Ok).collect())),
            probe_starts: Arc::new(AtomicUsize::new(0)),
            active_probes: Arc::new(AtomicUsize::new(0)),
            fail_probe: false,
            transfer_http_version: "1.1",
            transfer_ip_family: "ipv4",
            probe_http_version: "1.1",
            probe_ip_family: "ipv4",
            invalid_probe: false,
        }
    }
}

impl MeasurementTransport for TimedTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(async { unreachable!("test plan contains no unloaded latency steps") })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: Direction,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        Box::pin(async move {
            self.probe_starts.fetch_add(1, Ordering::SeqCst);
            self.active_probes.fetch_add(1, Ordering::SeqCst);
            let result = if self.fail_probe {
                Err(TransportError::HeaderTimeout {
                    endpoint: "https://fixture.invalid/__down".to_owned(),
                    payload_bytes: 0,
                })
            } else {
                if cancellation.is_cancelled() {
                    Err(TransportError::Cancelled { payload_bytes: 0 })
                } else {
                    let ttfb_ms = if self.invalid_probe { f64::NAN } else { 12.0 };
                    Ok(TimingObservation::from_millis(
                        ttfb_ms,
                        12.0,
                        2.0,
                        0,
                        self.probe_http_version,
                    )
                    .with_ip_family(self.probe_ip_family)
                    .with_endpoint("https://fixture.invalid/__down"))
                }
            };
            self.active_probes.fetch_sub(1, Ordering::SeqCst);
            result
        })
    }

    fn download<'a>(
        &'a self,
        bytes: u64,
        _: Option<&'a str>,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        Box::pin(async move {
            let scripted = self.transfers.lock().unwrap().pop_front().unwrap();
            let (wall_time, adjusted_ms) = scripted?;
            tokio::select! {
                () = cancellation.cancelled() => Err(TransportError::Cancelled { payload_bytes: 0 }),
                () = tokio::time::sleep(wall_time) => Ok(TimingObservation::from_millis(10.0, adjusted_ms, 0.0, bytes, self.transfer_http_version).with_ip_family(self.transfer_ip_family)),
            }
        })
    }

    fn upload<'a>(
        &'a self,
        bytes: u64,
        cancellation: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        self.download(bytes, None, cancellation)
    }
}

fn download_plan(count: u32) -> MeasurementPlan {
    MeasurementPlan {
        upstream_version: "test",
        upstream_commit: "test",
        steps: vec![MeasurementStep::Download {
            bytes: 100_000,
            count,
            bypass_finish: false,
        }],
    }
}

#[tokio::test(start_paused = true)]
async fn loaded_probe_starts_at_20ms_then_throttles_400ms() {
    let transport = TimedTransport::new([(Duration::from_millis(900), 900.0)]);
    let starts = transport.probe_starts.clone();
    let outcome_task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run(&CancellationToken::new())
            .await
    });
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    tokio::time::advance(Duration::from_millis(19)).await;
    tokio::task::yield_now().await;
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_millis(399)).await;
    tokio::task::yield_now().await;
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    tokio::time::advance(Duration::from_millis(500)).await;
    let outcome = outcome_task.await.unwrap();
    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.raw.download_loaded_latency.len(), 3);
}

#[tokio::test(start_paused = true)]
async fn one_probe_loop_spans_sequential_transfers_and_is_awaited() {
    let transport = TimedTransport::new([
        (Duration::from_millis(300), 300.0),
        (Duration::from_millis(300), 300.0),
    ]);
    let starts = transport.probe_starts.clone();
    let active = transport.active_probes.clone();
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(2))
            .run(&CancellationToken::new())
            .await
    });
    tokio::time::advance(Duration::from_millis(600)).await;
    let outcome = task.await.unwrap();

    assert!(starts.load(Ordering::SeqCst) >= 2);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert!(!outcome.result.raw.download_loaded_latency.is_empty());
}

#[tokio::test(start_paused = true)]
async fn ineligible_group_discards_loaded_points_and_probe_errors_are_diagnostics() {
    let transport = TimedTransport {
        fail_probe: true,
        ..TimedTransport::new([(Duration::from_millis(300), 249.0)])
    };
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run(&CancellationToken::new())
            .await
    });
    tokio::time::advance(Duration::from_millis(300)).await;
    let outcome = task.await.unwrap();

    assert!(outcome.error.is_none());
    assert!(outcome.result.raw.download_loaded_latency.is_empty());
    assert!(!outcome.result.diagnostics.is_empty());
    let diagnostic = &outcome.result.diagnostics[0];
    assert!(diagnostic.contains("during download"));
    assert!(diagnostic.contains("https://fixture.invalid/__down"));
    assert!(diagnostic.contains("response headers"));
}

#[tokio::test(start_paused = true)]
async fn loaded_results_keep_only_latest_twenty() {
    let transport = TimedTransport::new([(Duration::from_secs(9), 900.0)]);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run(&CancellationToken::new())
            .await
    });
    tokio::time::advance(Duration::from_secs(9)).await;
    let outcome = task.await.unwrap();
    assert_eq!(outcome.result.raw.download_loaded_latency.len(), 20);
}

#[tokio::test(start_paused = true)]
async fn loaded_probe_metadata_participates_in_run_aggregation() {
    let transport = TimedTransport {
        transfer_http_version: "2",
        transfer_ip_family: "ipv6",
        probe_http_version: "1.1",
        probe_ip_family: "ipv4",
        ..TimedTransport::new([(Duration::from_millis(300), 300.0)])
    };
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run(&CancellationToken::new())
            .await
    });
    tokio::time::advance(Duration::from_millis(300)).await;
    let outcome = task.await.unwrap();

    assert_eq!(outcome.result.target.http_version.as_deref(), Some("mixed"));
    assert_eq!(outcome.result.target.ip_family.as_deref(), Some("mixed"));
}

#[tokio::test(start_paused = true)]
async fn loaded_probe_conversion_diagnostic_includes_direction_endpoint_and_cause() {
    let transport = TimedTransport {
        invalid_probe: true,
        ..TimedTransport::new([(Duration::from_millis(300), 300.0)])
    };
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run(&CancellationToken::new())
            .await
    });
    tokio::time::advance(Duration::from_millis(300)).await;
    let outcome = task.await.unwrap();

    let diagnostic = outcome
        .result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.contains("conversion"))
        .unwrap();
    assert!(diagnostic.contains("during download"));
    assert!(diagnostic.contains("https://fixture.invalid/__down"));
    assert!(diagnostic.contains("timing observation is invalid"));
    assert!(!diagnostic.contains('?'));
}

#[tokio::test(start_paused = true)]
async fn top_level_cancellation_stops_transfer_and_joins_probe() {
    let transport = TimedTransport::new([(Duration::from_secs(30), 30_000.0)]);
    let active = transport.active_probes.clone();
    let cancellation = CancellationToken::new();
    let run_token = cancellation.clone();
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run(&run_token)
            .await
    });
    tokio::time::advance(Duration::from_millis(25)).await;
    cancellation.cancel();
    let outcome = task.await.unwrap();

    assert!(outcome.error.is_some());
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert!(outcome.result.raw.download.is_empty());
}

#[tokio::test(start_paused = true)]
async fn later_transfer_failure_keeps_eligible_loaded_points_and_joins_probe() {
    let transport = TimedTransport::new([(Duration::from_millis(300), 300.0)]);
    transport
        .transfers
        .lock()
        .unwrap()
        .push_back(Err(TransportError::BodyTimeout {
            endpoint: "https://fixture.invalid/__down".to_owned(),
            payload_bytes: 0,
        }));
    let active = transport.active_probes.clone();
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(2))
            .run(&CancellationToken::new())
            .await
    });
    tokio::time::advance(Duration::from_millis(300)).await;
    let outcome = task.await.unwrap();

    assert!(outcome.error.is_some());
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert!(!outcome.result.raw.download_loaded_latency.is_empty());
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn loaded_progress_is_direction_local_and_includes_ineligible_probes() {
    let transport = TimedTransport::new([
        (Duration::from_millis(300), 249.0),
        (Duration::from_millis(300), 300.0),
        (Duration::from_millis(300), 300.0),
    ]);
    let plan = MeasurementPlan {
        upstream_version: "test",
        upstream_commit: "test",
        steps: vec![
            MeasurementStep::Download {
                bytes: 100_000,
                count: 1,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 100_000,
                count: 1,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 1_000_000,
                count: 1,
                bypass_finish: false,
            },
        ],
    };
    let (progress, receiver) = ProgressReporter::channel(256);
    let task = tokio::spawn(async move {
        Runner::new(transport, plan)
            .run_with_progress(&CancellationToken::new(), progress)
            .await
    });

    for _ in 0..3 {
        tokio::time::advance(Duration::from_millis(300)).await;
        tokio::task::yield_now().await;
    }
    let outcome = task.await.unwrap();
    let loaded_events: Vec<_> = receiver
        .into_iter()
        .filter(|event| matches!(event, ProgressEvent::LoadedLatencyCompleted { .. }))
        .collect();

    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.raw.download_loaded_latency.len(), 1);
    assert_eq!(outcome.result.raw.upload_loaded_latency.len(), 1);
    assert_eq!(
        loaded_events,
        vec![
            ProgressEvent::LoadedLatencyCompleted {
                direction: Direction::Download,
                sequence: 1,
                latency_ms: 10.0,
            },
            ProgressEvent::LoadedLatencyCompleted {
                direction: Direction::Upload,
                sequence: 1,
                latency_ms: 10.0,
            },
            ProgressEvent::LoadedLatencyCompleted {
                direction: Direction::Download,
                sequence: 2,
                latency_ms: 10.0,
            },
        ]
    );
}
