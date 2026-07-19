use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cfbench::cancellation::CancellationToken;
use cfbench::error::TransportError;
use cfbench::measurement::TimingObservation;
use cfbench::plan::{MeasurementPlan, MeasurementStep};
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
    let rejected =
        TimingObservation::from_millis(f64::NAN, 30.0, 10.0, 0, "1.1").with_ip_family("ipv4");
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
    assert_eq!(outcome.result.failures.len(), 1);
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
