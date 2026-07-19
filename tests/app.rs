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

#[test]
fn diagnostics_are_written_for_successful_and_failed_outcomes() {
    let options = OutputOptions {
        json: true,
        quiet: true,
    };
    let mut success = cfbench::results::RunResult::empty();
    success.diagnostics.push("successful diagnostic".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = write_outcome(
        RunOutcome {
            result: success,
            error: None,
        },
        options,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(exit, 0);
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "diagnostic: successful diagnostic\n"
    );
    serde_json::from_slice::<serde_json::Value>(&stdout).unwrap();

    let mut failure = partial_failure();
    failure
        .result
        .diagnostics
        .push("failed diagnostic".to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = write_outcome(
        failure,
        OutputOptions {
            json: false,
            quiet: false,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(exit, 1);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("diagnostic: failed diagnostic\n"));
    assert!(stderr.contains("error: measurement cancelled"));
    assert!(
        String::from_utf8(stdout)
            .unwrap()
            .starts_with("cfbench 0.1.0\n")
    );
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

#[tokio::test]
async fn selected_signal_forces_terminal_outcome_after_concurrent_final_success() {
    let final_operation_polled = Arc::new(AtomicBool::new(false));
    let runner = Runner::new(
        ConcurrentSuccessTransport(final_operation_polled.clone()),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 1 }],
        },
    )
    .with_loaded_latency(false);

    let outcome = run_with_signal(&runner, ReadyWithFinalOperation(final_operation_polled)).await;

    assert!(matches!(
        outcome.error,
        Some(RunnerError::Cancelled { ref stage }) if stage == "run"
    ));
    assert_eq!(outcome.result.raw.initial_latency.len(), 1);
    assert!(
        outcome
            .result
            .failures
            .iter()
            .any(|failure| failure == "measurement cancelled during run")
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit = write_outcome(
        outcome,
        OutputOptions {
            json: true,
            quiet: true,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(exit, 1);
    serde_json::from_slice::<serde_json::Value>(&stdout).unwrap();
}

#[tokio::test]
async fn transport_failure_stderr_includes_stage_redacted_endpoint_and_cause() {
    let endpoint = "https://speed.cloudflare.com/__down";
    let runner = Runner::new(
        ErrorTransport(endpoint.to_owned()),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Latency { packets: 1 }],
        },
    )
    .with_loaded_latency(false);
    let outcome = runner.run(&CancellationToken::new()).await;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let exit = write_outcome(
        outcome,
        OutputOptions {
            json: true,
            quiet: true,
        },
        &mut stdout,
        &mut stderr,
    )
    .unwrap();

    assert_eq!(exit, 1);
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stderr.contains("during latency"));
    assert!(stderr.contains(endpoint));
    assert!(stderr.contains("HTTP status 503"));
    assert!(!stderr.contains("bytes="));
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
    Box::pin(async { Err(TransportError::Cancelled { payload_bytes: 0 }) })
}

struct ReadyWithFinalOperation(Arc<AtomicBool>);

impl Future for ReadyWithFinalOperation {
    type Output = std::io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0.load(Ordering::SeqCst) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

struct ConcurrentSuccessTransport(Arc<AtomicBool>);

impl MeasurementTransport for ConcurrentSuccessTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        Box::pin(PendingThenSuccess {
            first_poll: true,
            final_operation_polled: self.0.clone(),
        })
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

struct PendingThenSuccess {
    first_poll: bool,
    final_operation_polled: Arc<AtomicBool>,
}

impl Future for PendingThenSuccess {
    type Output = Result<TimingObservation, TransportError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.first_poll {
            self.first_poll = false;
            self.final_operation_polled.store(true, Ordering::SeqCst);
            context.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(Ok(TimingObservation::from_millis(
                20.0, 20.0, 10.0, 0, "1.1",
            )))
        }
    }
}

struct ErrorTransport(String);

impl MeasurementTransport for ErrorTransport {
    fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
        let endpoint = self.0.clone();
        Box::pin(async move {
            Err(TransportError::HttpStatus {
                endpoint,
                status: 503,
                payload_bytes: 0,
            })
        })
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
