use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use cfbench::app::{OutputOptions, run_with_signal, write_outcome, write_progress};
use cfbench::cancellation::CancellationToken;
use cfbench::error::TransportError;
use cfbench::measurement::TimingObservation;
use cfbench::plan::{MeasurementPlan, MeasurementStep};
use cfbench::runner::{MeasurementFuture, MeasurementTransport, RunOutcome, Runner, RunnerError};

#[test]
fn json_mode_writes_one_partial_document_without_progress_then_returns_one() {
    let options = OutputOptions {
        json: true,
        quiet: false,
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    write_progress(options, &mut stderr).unwrap();
    let exit = write_outcome(partial_failure(), options, &mut stdout, &mut stderr).unwrap();

    assert_eq!(exit, 1);
    let parsed: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["summary"]["download_bps"].is_null());
    assert!(
        !String::from_utf8(stdout)
            .unwrap()
            .contains("Testing against")
    );
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(!stderr.contains("Testing against"));
    assert!(stderr.contains("error: measurement cancelled"));
}

#[test]
fn quiet_suppresses_progress_not_text_result_or_terminal_error() {
    let options = OutputOptions {
        json: false,
        quiet: true,
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    write_progress(options, &mut stderr).unwrap();
    let exit = write_outcome(partial_failure(), options, &mut stdout, &mut stderr).unwrap();

    assert_eq!(exit, 1);
    assert!(
        String::from_utf8(stdout)
            .unwrap()
            .starts_with("cfbench 0.1.0\n")
    );
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(!stderr.contains("Testing against"));
    assert!(stderr.contains("error:"));
}

#[tokio::test]
async fn signal_is_polled_before_runner_starts_network_work() {
    let installed = Arc::new(AtomicBool::new(false));
    let runner = Runner::new(
        InstallAwareTransport(installed.clone()),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 1 }],
        },
    )
    .with_loaded_latency(false);

    let outcome = run_with_signal(&runner, InstallSignal(installed)).await;

    assert!(outcome.error.is_none());
    assert_eq!(outcome.result.raw.initial_latency.len(), 1);
}

fn partial_failure() -> RunOutcome {
    let error = RunnerError::Cancelled {
        stage: "download".to_owned(),
    };
    let mut result = cfbench::results::RunResult::empty();
    result.failures.push(error.to_string());
    RunOutcome {
        result,
        error: Some(error),
    }
}

struct InstallSignal(Arc<AtomicBool>);

impl Future for InstallSignal {
    type Output = std::io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.store(true, Ordering::SeqCst);
        Poll::Pending
    }
}

struct InstallAwareTransport(Arc<AtomicBool>);

impl MeasurementTransport for InstallAwareTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        assert!(self.0.load(Ordering::SeqCst));
        Box::pin(async { Ok(TimingObservation::from_millis(20.0, 20.0, 10.0, 0, "1.1")) })
    }

    fn loaded_latency<'a>(
        &'a self,
        _: cfbench::plan::Direction,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn download<'a>(
        &'a self,
        _: u64,
        _: Option<&'a str>,
        _: &'a CancellationToken,
    ) -> MeasurementFuture<'a> {
        unused_measurement()
    }

    fn upload<'a>(&'a self, _: u64, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        unused_measurement()
    }
}

fn unused_measurement<'a>() -> MeasurementFuture<'a> {
    Box::pin(async { Err(TransportError::Cancelled) })
}
