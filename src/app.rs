use std::future::{Future, poll_fn};
use std::io::Write;
use std::sync::mpsc::Receiver;
use std::task::Poll;
use std::thread::JoinHandle;

use thiserror::Error;

use crate::cancellation::CancellationToken;
use crate::error::OutputError;
use crate::output::{render_json, render_progress, render_text};
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::runner::{MeasurementTransport, RunOutcome, Runner, RunnerError};

const PROGRESS_CHANNEL_CAPACITY: usize = 256;
const OPENING_PROGRESS_LINE: &str = "Testing against Cloudflare edge...";

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
    #[error("progress renderer thread terminated unexpectedly")]
    ProgressRendererPanicked,
}

/// A completed runner lifecycle and any separately retained progress failure.
#[derive(Debug)]
pub struct ProgressRunOutcome {
    pub outcome: RunOutcome,
    pub progress_error: Option<AppError>,
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
    run_with_signal_inner(runner, signal, &cancellation, ProgressReporter::disabled()).await
}

/// Runs with live line-oriented progress only for ordinary non-quiet text.
///
/// The renderer owns its blocking writer on a dedicated thread. The thread is
/// joined after the runner (including loaded probes) has released every sender
/// clone and before this function returns to final-result rendering.
pub async fn run_with_signal_and_progress<T, S, W>(
    runner: &Runner<T>,
    signal: S,
    options: OutputOptions,
    stderr: W,
) -> ProgressRunOutcome
where
    T: MeasurementTransport,
    S: Future<Output = std::io::Result<()>>,
    W: Write + Send + 'static,
{
    let cancellation = CancellationToken::new();
    if options.quiet || options.json {
        drop(stderr);
        return ProgressRunOutcome {
            outcome: run_with_signal_inner(
                runner,
                signal,
                &cancellation,
                ProgressReporter::disabled(),
            )
            .await,
            progress_error: None,
        };
    }

    let (progress, receiver) = ProgressReporter::channel(PROGRESS_CHANNEL_CAPACITY);
    let renderer = spawn_progress_renderer(receiver, stderr, cancellation.clone());
    let mut progress_error = None;
    let renderer = match renderer {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            progress_error = Some(error);
            None
        }
    };
    let outcome = run_with_signal_inner(runner, signal, &cancellation, progress).await;

    if let Some(renderer) = renderer {
        progress_error = match renderer.join() {
            Ok(result) => result.err(),
            Err(_) => Some(AppError::ProgressRendererPanicked),
        };
    }

    ProgressRunOutcome {
        outcome,
        progress_error,
    }
}

/// Starts the blocking stderr renderer used by live progress mode.
pub fn spawn_progress_renderer<W>(
    receiver: Receiver<ProgressEvent>,
    writer: W,
    cancellation: CancellationToken,
) -> Result<JoinHandle<Result<(), AppError>>, AppError>
where
    W: Write + Send + 'static,
{
    let spawn_failure_cancellation = cancellation.clone();
    let renderer = std::thread::Builder::new()
        .name("cfbench-progress".to_owned())
        .spawn(move || {
            let mut cancel_on_panic = CancelOnDrop::new(cancellation.clone());
            let result = render_progress_lines(receiver, writer);
            if result.is_err() {
                cancellation.cancel();
            }
            cancel_on_panic.disarm();
            result
        });

    match renderer {
        Ok(renderer) => Ok(renderer),
        Err(error) => {
            spawn_failure_cancellation.cancel();
            Err(AppError::Write(error))
        }
    }
}

async fn run_with_signal_inner<T, S>(
    runner: &Runner<T>,
    signal: S,
    cancellation: &CancellationToken,
    progress: ProgressReporter,
) -> RunOutcome
where
    T: MeasurementTransport,
    S: Future<Output = std::io::Result<()>>,
{
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
        let mut outcome = runner.run_with_progress(cancellation, progress).await;
        record_signal_error(&mut outcome, signal);
        ensure_cancelled_outcome(&mut outcome);
        return outcome;
    }

    let run = runner.run_with_progress(cancellation, progress);
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

fn render_progress_lines<W>(
    receiver: Receiver<ProgressEvent>,
    mut writer: W,
) -> Result<(), AppError>
where
    W: Write,
{
    write_progress_line(&mut writer, OPENING_PROGRESS_LINE)?;
    for event in receiver {
        write_progress_line(&mut writer, &render_progress(&event))?;
    }
    Ok(())
}

fn write_progress_line(writer: &mut impl Write, line: &str) -> Result<(), AppError> {
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

struct CancelOnDrop {
    cancellation: CancellationToken,
    armed: bool,
}

impl CancelOnDrop {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.cancel();
        }
    }
}

pub fn write_progress(options: OutputOptions, stderr: &mut impl Write) -> Result<(), AppError> {
    if !options.quiet && !options.json {
        write_progress_line(stderr, OPENING_PROGRESS_LINE)?;
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
