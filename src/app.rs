use std::future::{Future, poll_fn};
use std::io::Write;
use std::task::Poll;

use thiserror::Error;

use crate::cancellation::CancellationToken;
use crate::error::OutputError;
use crate::output::{render_json, render_text};
use crate::runner::{MeasurementTransport, RunOutcome, Runner, RunnerError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputOptions {
    pub json: bool,
    pub quiet: bool,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error("could not write command output: {0}")]
    Write(#[from] std::io::Error),
}

/// Polls signal installation before allowing the runner to start network work.
///
/// Cancellation remains caller-owned: after a signal, the runner is awaited so
/// its transfer and loaded-latency tasks finish before results are rendered.
pub async fn run_with_signal<T, S>(runner: &Runner<T>, signal: S) -> RunOutcome
where
    T: MeasurementTransport,
    S: Future<Output = std::io::Result<()>>,
{
    let cancellation = CancellationToken::new();
    tokio::pin!(signal);

    let initial_signal = poll_fn(|context| {
        Poll::Ready(match signal.as_mut().poll(context) {
            Poll::Ready(result) => Some(result),
            Poll::Pending => None,
        })
    })
    .await;

    if let Some(signal) = initial_signal {
        cancellation.cancel();
        let mut outcome = runner.run(&cancellation).await;
        record_signal_error(&mut outcome, signal);
        ensure_cancelled_outcome(&mut outcome);
        return outcome;
    }

    let run = runner.run(&cancellation);
    tokio::pin!(run);
    tokio::select! {
        biased;
        signal = &mut signal => {
            cancellation.cancel();
            let mut outcome = run.await;
            record_signal_error(&mut outcome, signal);
            ensure_cancelled_outcome(&mut outcome);
            outcome
        },
        outcome = &mut run => outcome,
    }
}

pub fn write_progress(options: OutputOptions, stderr: &mut impl Write) -> Result<(), AppError> {
    if !options.quiet && !options.json {
        writeln!(stderr, "Testing against Cloudflare edge...")?;
    }
    Ok(())
}

/// Writes one final result and returns the process status required by its outcome.
pub fn write_outcome(
    outcome: RunOutcome,
    options: OutputOptions,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<u8, AppError> {
    let rendered = if options.json {
        render_json(&outcome.result)?
    } else {
        render_text(&outcome.result)
    };
    stdout.write_all(rendered.as_bytes())?;
    if !rendered.ends_with('\n') {
        stdout.write_all(b"\n")?;
    }
    stdout.flush()?;

    for diagnostic in &outcome.result.diagnostics {
        writeln!(stderr, "diagnostic: {diagnostic}")?;
    }

    match outcome.error {
        Some(error) => {
            writeln!(stderr, "error: {error}")?;
            Ok(1)
        }
        None => Ok(0),
    }
}

fn record_signal_error(outcome: &mut RunOutcome, signal: std::io::Result<()>) {
    if let Err(error) = signal {
        outcome
            .result
            .diagnostics
            .push(format!("could not install Ctrl+C handler: {error}"));
    }
}

fn ensure_cancelled_outcome(outcome: &mut RunOutcome) {
    if outcome.error.is_none() {
        let error = RunnerError::Cancelled {
            stage: "run".to_owned(),
        };
        outcome.result.failures.push(error.to_string());
        outcome.error = Some(error);
    }
}
