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

pub const EXIT_COMPLETE: u8 = 0;
pub const EXIT_FAILURE: u8 = 1;
pub const EXIT_PARTIAL: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinalOutput {
    Text,
    Json,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputOptions {
    pub json: bool,
    pub quiet: bool,
    pub verbose: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressMode {
    Disabled,
    Compact,
    Verbose,
}

impl OutputOptions {
    fn final_output(self) -> FinalOutput {
        if self.quiet {
            FinalOutput::None
        } else if self.json {
            FinalOutput::Json
        } else {
            FinalOutput::Text
        }
    }

    fn progress_enabled(self) -> bool {
        self.verbose && !self.quiet && !self.json
    }

    pub fn progress_mode(self, stderr_is_terminal: bool) -> ProgressMode {
        if self.quiet || self.json {
            ProgressMode::Disabled
        } else if self.verbose {
            ProgressMode::Verbose
        } else if stderr_is_terminal {
            ProgressMode::Compact
        } else {
            ProgressMode::Disabled
        }
    }
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

/// Runs with live line-oriented progress only for verbose, non-quiet text.
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
    if !options.progress_enabled() {
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
    if options.progress_enabled() {
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
    match options.final_output() {
        FinalOutput::Text => write_rendered(stdout, &render_text(&outcome.result))?,
        FinalOutput::Json => write_rendered(stdout, &render_json(&outcome.result)?)?,
        FinalOutput::None => {}
    }

    if !options.quiet {
        for diagnostic in &outcome.result.diagnostics {
            writeln!(stderr, "diagnostic: {diagnostic}")?;
        }
    }

    if let Some(error) = &outcome.error {
        writeln!(stderr, "error: {error}")?;
    }

    Ok(exit_status(&outcome))
}

fn write_rendered(stdout: &mut impl Write, rendered: &str) -> Result<(), AppError> {
    stdout.write_all(rendered.as_bytes())?;
    if !rendered.ends_with('\n') {
        stdout.write_all(b"\n")?;
    }
    stdout.flush()?;
    Ok(())
}

/// Classifies a rendered measurement outcome for automation callers.
pub fn exit_status(outcome: &RunOutcome) -> u8 {
    if matches!(outcome.error, Some(RunnerError::Cancelled { .. })) {
        return EXIT_FAILURE;
    }

    let accepted = !outcome.result.raw.latency.is_empty()
        || !outcome.result.raw.download.is_empty()
        || !outcome.result.raw.upload.is_empty();

    match (accepted, outcome.error.is_some()) {
        (true, false) => EXIT_COMPLETE,
        (true, true) => EXIT_PARTIAL,
        (false, _) => EXIT_FAILURE,
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
    if matches!(outcome.error, Some(RunnerError::Cancelled { .. })) {
        return;
    }

    let error = RunnerError::Cancelled {
        stage: "run".to_owned(),
    };
    outcome.result.failures.push(error.to_string());
    outcome.error = Some(error);
}
