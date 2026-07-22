use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cfbench::cancellation::CancellationToken;
use cfbench::error::TransportError;
use cfbench::measurement::TimingObservation;
use cfbench::plan::{Direction, MeasurementPlan, MeasurementStep};
use cfbench::progress::{ProgressEvent, ProgressFailureKind, ProgressReporter, ProgressStage};
use cfbench::runner::{MeasurementFuture, MeasurementTransport, Runner, RunnerError};
use tokio::sync::Notify;

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
    block_probe_until_cancelled: bool,
    ignore_transfer_cancellation: bool,
    probe_cancel_observed: Option<Arc<Notify>>,
    release_cancelled_probe: Option<Arc<Notify>>,
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
            block_probe_until_cancelled: false,
            ignore_transfer_cancellation: false,
            probe_cancel_observed: None,
            release_cancelled_probe: None,
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
            } else if self.block_probe_until_cancelled {
                cancellation.cancelled().await;
                if let Some(observed) = &self.probe_cancel_observed {
                    observed.notify_one();
                }
                if let Some(release) = &self.release_cancelled_probe {
                    release.notified().await;
                }
                Err(TransportError::Cancelled { payload_bytes: 0 })
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
            let observation = TimingObservation::from_millis(
                10.0,
                adjusted_ms,
                0.0,
                bytes,
                self.transfer_http_version,
            )
            .with_ip_family(self.transfer_ip_family);
            if self.ignore_transfer_cancellation {
                tokio::time::sleep(wall_time).await;
                Ok(observation)
            } else {
                tokio::select! {
                    () = cancellation.cancelled() => Err(TransportError::Cancelled { payload_bytes: 0 }),
                    () = tokio::time::sleep(wall_time) => Ok(observation),
                }
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
    let starts = transport.probe_starts.clone();
    let (progress, receiver) = ProgressReporter::channel(256);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run_with_progress(&CancellationToken::new(), progress)
            .await
    });
    tokio::time::advance(Duration::from_secs(9)).await;
    let outcome = task.await.unwrap();
    let loaded_events: Vec<_> = receiver
        .into_iter()
        .filter_map(|event| match event {
            ProgressEvent::LoadedLatencyCompleted {
                direction,
                sequence,
                ..
            } => Some((direction, sequence)),
            _ => None,
        })
        .collect();

    assert_eq!(outcome.result.raw.download_loaded_latency.len(), 20);
    assert!(loaded_events.len() > 20);
    assert_eq!(loaded_events.len(), starts.load(Ordering::SeqCst));
    assert!(
        loaded_events
            .iter()
            .enumerate()
            .all(|(index, event)| *event == (Direction::Download, index as u64 + 1))
    );
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
async fn accepted_loaded_and_transfer_points_receive_run_clock_timestamps() {
    let transport = TimedTransport::new([(Duration::from_millis(300), 300.0)]);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run(&CancellationToken::new())
            .await
    });
    tokio::time::advance(Duration::from_millis(300)).await;
    let outcome = task.await.unwrap();

    assert!(outcome.error.is_none());
    assert!(
        outcome.result.raw.download[0].measured_at_unix_ms > 0,
        "the accepted transfer point must use the run clock"
    );
    let loaded_timestamps = outcome
        .result
        .raw
        .download_loaded_latency
        .as_slice()
        .iter()
        .map(|point| point.measured_at_unix_ms)
        .collect::<Vec<_>>();
    assert!(!loaded_timestamps.is_empty());
    assert!(loaded_timestamps.iter().all(|timestamp| *timestamp > 0));
    assert!(loaded_timestamps.windows(2).all(|pair| pair[0] <= pair[1]));
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
async fn parent_cancellation_emits_one_loaded_cancelled_progress_event() {
    let transport = TimedTransport {
        block_probe_until_cancelled: true,
        ..TimedTransport::new([(Duration::from_secs(30), 30_000.0)])
    };
    let active = transport.active_probes.clone();
    let cancellation = CancellationToken::new();
    let run_token = cancellation.clone();
    let (progress, receiver) = ProgressReporter::channel(256);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run_with_progress(&run_token, progress)
            .await
    });

    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(20)).await;
    tokio::task::yield_now().await;
    assert_eq!(active.load(Ordering::SeqCst), 1);
    cancellation.cancel();
    let outcome = task.await.unwrap();
    let events = receiver.into_iter().collect::<Vec<_>>();

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "download"
    ));
    assert_eq!(outcome.result.failures.len(), 1);
    assert!(outcome.result.diagnostics.is_empty());
    assert!(outcome.result.raw.download.is_empty());
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::RequestFailed { .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                ProgressEvent::RequestFailed {
                    stage: ProgressStage::LoadedLatency {
                        direction: Direction::Download,
                    },
                    current: None,
                    total: None,
                    kind: ProgressFailureKind::Cancelled,
                }
            ))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                ProgressEvent::RequestFailed {
                    stage: ProgressStage::Transfer {
                        direction: Direction::Download,
                        requested_bytes: 100_000,
                    },
                    current: Some(1),
                    total: Some(1),
                    kind: ProgressFailureKind::Cancelled,
                }
            ))
            .count(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn parent_cancellation_after_normal_group_teardown_stays_silent() {
    let probe_cancel_observed = Arc::new(Notify::new());
    let release_cancelled_probe = Arc::new(Notify::new());
    let transport = TimedTransport {
        block_probe_until_cancelled: true,
        probe_cancel_observed: Some(probe_cancel_observed.clone()),
        release_cancelled_probe: Some(release_cancelled_probe.clone()),
        ..TimedTransport::new([(Duration::from_millis(300), 300.0)])
    };
    let active = transport.active_probes.clone();
    let cancellation = CancellationToken::new();
    let run_token = cancellation.clone();
    let (progress, receiver) = ProgressReporter::channel(256);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run_with_progress(&run_token, progress)
            .await
    });

    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(300)).await;
    probe_cancel_observed.notified().await;
    assert_eq!(active.load(Ordering::SeqCst), 1);
    cancellation.cancel();
    release_cancelled_probe.notify_one();
    let outcome = task.await.unwrap();
    let loaded_failures = receiver
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                ProgressEvent::RequestFailed {
                    stage: ProgressStage::LoadedLatency {
                        direction: Direction::Download,
                    },
                    ..
                }
            )
        })
        .count();

    assert!(outcome.error.is_none());
    assert!(outcome.result.diagnostics.is_empty());
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(loaded_failures, 0);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn parent_cancellation_is_reported_when_transfer_completes_concurrently() {
    let probe_cancel_observed = Arc::new(Notify::new());
    let release_cancelled_probe = Arc::new(Notify::new());
    let transport = TimedTransport {
        block_probe_until_cancelled: true,
        ignore_transfer_cancellation: true,
        probe_cancel_observed: Some(probe_cancel_observed.clone()),
        release_cancelled_probe: Some(release_cancelled_probe.clone()),
        ..TimedTransport::new([(Duration::from_millis(300), 300.0)])
    };
    let active = transport.active_probes.clone();
    let cancellation = CancellationToken::new();
    let run_token = cancellation.clone();
    let (progress, receiver) = ProgressReporter::channel(256);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run_with_progress(&run_token, progress)
            .await
    });

    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(20)).await;
    tokio::task::yield_now().await;
    assert_eq!(active.load(Ordering::SeqCst), 1);
    cancellation.cancel();
    probe_cancel_observed.notified().await;
    tokio::time::advance(Duration::from_millis(280)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    release_cancelled_probe.notify_one();
    let outcome = task.await.unwrap();
    let loaded_cancellations = receiver
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                ProgressEvent::RequestFailed {
                    stage: ProgressStage::LoadedLatency {
                        direction: Direction::Download,
                    },
                    current: None,
                    total: None,
                    kind: ProgressFailureKind::Cancelled,
                }
            )
        })
        .count();

    assert!(outcome.error.is_none());
    assert!(outcome.result.diagnostics.is_empty());
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(loaded_cancellations, 1);
    assert_eq!(active.load(Ordering::SeqCst), 0);
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

#[tokio::test(start_paused = true)]
async fn loaded_transport_failure_emits_once_and_remains_nonterminal() {
    let transport = TimedTransport {
        fail_probe: true,
        ..TimedTransport::new([(Duration::from_millis(300), 300.0)])
    };
    let (progress, receiver) = ProgressReporter::channel(256);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run_with_progress(&CancellationToken::new(), progress)
            .await
    });

    tokio::time::advance(Duration::from_millis(300)).await;
    let outcome = task.await.unwrap();
    let failures: Vec<_> = receiver
        .into_iter()
        .filter(|event| matches!(event, ProgressEvent::RequestFailed { .. }))
        .collect();

    assert!(outcome.error.is_none());
    assert!(outcome.result.raw.download_loaded_latency.is_empty());
    assert_eq!(outcome.result.diagnostics.len(), 1);
    assert_eq!(
        failures,
        vec![ProgressEvent::RequestFailed {
            stage: ProgressStage::LoadedLatency {
                direction: Direction::Download,
            },
            current: None,
            total: None,
            kind: ProgressFailureKind::Timeout,
        }]
    );
}

#[tokio::test(start_paused = true)]
async fn loaded_conversion_rejection_emits_once_and_remains_nonterminal() {
    let transport = TimedTransport {
        invalid_probe: true,
        ..TimedTransport::new([(Duration::from_millis(300), 300.0)])
    };
    let (progress, receiver) = ProgressReporter::channel(256);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run_with_progress(&CancellationToken::new(), progress)
            .await
    });

    tokio::time::advance(Duration::from_millis(300)).await;
    let outcome = task.await.unwrap();
    let failures: Vec<_> = receiver
        .into_iter()
        .filter(|event| matches!(event, ProgressEvent::RequestFailed { .. }))
        .collect();

    assert!(outcome.error.is_none());
    assert!(outcome.result.raw.download_loaded_latency.is_empty());
    assert_eq!(outcome.result.diagnostics.len(), 1);
    assert_eq!(
        failures,
        vec![ProgressEvent::RequestFailed {
            stage: ProgressStage::LoadedLatency {
                direction: Direction::Download,
            },
            current: None,
            total: None,
            kind: ProgressFailureKind::InvalidMeasurement,
        }]
    );
}

#[tokio::test(start_paused = true)]
async fn normal_group_shutdown_cancellation_emits_no_loaded_failure() {
    let transport = TimedTransport {
        block_probe_until_cancelled: true,
        ..TimedTransport::new([(Duration::from_millis(300), 300.0)])
    };
    let active = transport.active_probes.clone();
    let (progress, receiver) = ProgressReporter::channel(256);
    let task = tokio::spawn(async move {
        Runner::new(transport, download_plan(1))
            .run_with_progress(&CancellationToken::new(), progress)
            .await
    });

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(300)).await;
    let outcome = task.await.unwrap();
    let failures = receiver
        .into_iter()
        .filter(|event| matches!(event, ProgressEvent::RequestFailed { .. }))
        .count();

    assert!(outcome.error.is_none());
    assert!(outcome.result.diagnostics.is_empty());
    assert!(outcome.result.raw.download_loaded_latency.is_empty());
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(failures, 0);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}
