use std::collections::VecDeque;
use std::error::Error;
use std::sync::{Arc, Mutex};

use cfbench::cancellation::CancellationToken;
use cfbench::error::TransportError;
use cfbench::measurement::TimingObservation;
use cfbench::plan::{Direction, MeasurementPlan, MeasurementStep};
use cfbench::progress::{ProgressEvent, ProgressFailureKind, ProgressReporter, ProgressStage};
use cfbench::runner::{MeasurementFuture, MeasurementTransport, Runner, RunnerError};

#[derive(Clone)]
struct ScriptedTransport {
    script: Arc<Mutex<VecDeque<Result<TimingObservation, TransportError>>>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl ScriptedTransport {
    fn new(script: impl IntoIterator<Item = Result<TimingObservation, TransportError>>) -> Self {
        Self {
            script: Arc::new(Mutex::new(script.into_iter().collect())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
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
async fn progress_reports_packet_loss_unavailable_once_without_a_point() {
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
    assert_eq!(events, vec![ProgressEvent::PacketLossUnavailable]);
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
