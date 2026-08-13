use std::future::{Future, poll_fn};
use std::io::Write;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::task::Poll;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use thiserror::Error;

use crate::cancellation::CancellationToken;
use crate::error::OutputError;
use crate::output::{CompactProgressState, render_json, render_progress, render_text};
use crate::progress::{ProgressEvent, ProgressReporter};
use crate::runner::{MeasurementTransport, RunOutcome, Runner, RunnerError};

const PROGRESS_CHANNEL_CAPACITY: usize = 256;
const OPENING_PROGRESS_LINE: &str = "Testing against Cloudflare edge...";
const COMPACT_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const COMPACT_SPINNER_FRAMES: &[&str] = &["|", "/", "-", "\\"];

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

/// Runs with the selected progress lifecycle without changing measurement ownership.
///
/// A renderer is joined after the runner (including loaded probes) has released
/// every sender clone and before this function returns to final-result rendering.
pub async fn run_with_signal_and_progress<T, S, W>(
    runner: &Runner<T>,
    signal: S,
    options: OutputOptions,
    progress_mode: ProgressMode,
    stderr: W,
) -> ProgressRunOutcome
where
    T: MeasurementTransport,
    S: Future<Output = std::io::Result<()>>,
    W: Write + Send + 'static,
{
    run_with_signal_and_progress_inner(runner, signal, options, progress_mode, stderr, None).await
}

/// Runs compact progress with an injected draw target for lifecycle testing.
#[doc(hidden)]
pub async fn run_with_signal_and_progress_with_compact_draw_target<T, S, W>(
    runner: &Runner<T>,
    signal: S,
    options: OutputOptions,
    stderr: W,
    draw_target: ProgressDrawTarget,
) -> ProgressRunOutcome
where
    T: MeasurementTransport,
    S: Future<Output = std::io::Result<()>>,
    W: Write + Send + 'static,
{
    run_with_signal_and_progress_inner(
        runner,
        signal,
        options,
        ProgressMode::Compact,
        stderr,
        Some(draw_target),
    )
    .await
}

async fn run_with_signal_and_progress_inner<T, S, W>(
    runner: &Runner<T>,
    signal: S,
    _options: OutputOptions,
    progress_mode: ProgressMode,
    stderr: W,
    compact_draw_target: Option<ProgressDrawTarget>,
) -> ProgressRunOutcome
where
    T: MeasurementTransport,
    S: Future<Output = std::io::Result<()>>,
    W: Write + Send + 'static,
{
    let cancellation = CancellationToken::new();
    match progress_mode {
        ProgressMode::Disabled => {
            drop(stderr);
            ProgressRunOutcome {
                outcome: run_with_signal_inner(
                    runner,
                    signal,
                    &cancellation,
                    ProgressReporter::disabled(),
                )
                .await,
                progress_error: None,
            }
        }
        ProgressMode::Verbose => {
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
        ProgressMode::Compact => {
            drop(stderr);
            let (progress, receiver) = ProgressReporter::channel(PROGRESS_CHANNEL_CAPACITY);
            let draw_target = compact_draw_target.unwrap_or_else(ProgressDrawTarget::stderr);
            let renderer = spawn_compact_progress_renderer(receiver, draw_target).ok();
            let outcome = run_with_signal_inner(runner, signal, &cancellation, progress).await;
            if let Some(renderer) = renderer {
                let _ = renderer.join();
            }
            ProgressRunOutcome {
                outcome,
                progress_error: None,
            }
        }
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

/// Starts the best-effort dynamic renderer used by compact terminal mode.
pub fn spawn_compact_progress_renderer(
    receiver: Receiver<ProgressEvent>,
    draw_target: ProgressDrawTarget,
) -> Result<JoinHandle<()>, AppError> {
    std::thread::Builder::new()
        .name("cfbench-compact-progress".to_owned())
        .spawn(move || {
            let spinner = ProgressBar::with_draw_target(None, draw_target);
            spinner.set_style(
                ProgressStyle::with_template("{wide_msg}")
                    .expect("constant compact progress template is valid"),
            );
            let mut pump = CompactRenderPump::new(Instant::now());
            spinner.set_message(pump.opening_frame());

            loop {
                match receiver.recv_timeout(pump.time_until_refresh(Instant::now())) {
                    Ok(event) => {
                        if let Some(frame) = pump.apply_event(&event, Instant::now()) {
                            spinner.set_message(frame);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if let Some(frame) = pump.refresh(Instant::now()) {
                            spinner.set_message(frame);
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }

            if let Some(frame) = pump.finish() {
                spinner.set_message(frame);
            }
            spinner.finish_and_clear();
        })
        .map_err(AppError::Write)
}

/// Coalesces compact progress events and produces at most one regular frame per interval.
///
/// The renderer drives this pump with a monotonic clock. Keeping the timing policy here makes
/// event bursts deterministic to test and avoids independent Indicatif redraw paths.
struct CompactRenderPump {
    state: CompactProgressState,
    pending_message: String,
    rendered_message: Option<String>,
    next_refresh_at: Instant,
    spinner_frame: usize,
}

impl CompactRenderPump {
    fn new(started: Instant) -> Self {
        Self {
            state: CompactProgressState::default(),
            pending_message: OPENING_PROGRESS_LINE.to_owned(),
            rendered_message: None,
            next_refresh_at: started + COMPACT_REFRESH_INTERVAL,
            spinner_frame: 0,
        }
    }

    fn opening_frame(&mut self) -> String {
        self.render_frame()
    }

    fn apply_event(&mut self, event: &ProgressEvent, now: Instant) -> Option<String> {
        if let Some(message) = self.state.render(event) {
            self.pending_message = message;
        }
        self.refresh(now)
    }

    fn refresh(&mut self, now: Instant) -> Option<String> {
        if now < self.next_refresh_at {
            return None;
        }

        self.next_refresh_at = now + COMPACT_REFRESH_INTERVAL;
        Some(self.render_frame())
    }

    fn time_until_refresh(&self, now: Instant) -> Duration {
        self.next_refresh_at.saturating_duration_since(now)
    }

    fn finish(&mut self) -> Option<String> {
        (self.rendered_message.as_deref() != Some(self.pending_message.as_str()))
            .then(|| self.render_frame())
    }

    fn render_frame(&mut self) -> String {
        let spinner = COMPACT_SPINNER_FRAMES[self.spinner_frame];
        self.spinner_frame = (self.spinner_frame + 1) % COMPACT_SPINNER_FRAMES.len();
        self.rendered_message = Some(self.pending_message.clone());
        format!("{spinner} {}", self.pending_message)
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
        if matches!(
            event,
            ProgressEvent::RequestStarted { .. } | ProgressEvent::TransferAdvanced { .. }
        ) {
            continue;
        }
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

pub fn write_progress(mode: ProgressMode, stderr: &mut impl Write) -> Result<(), AppError> {
    if mode == ProgressMode::Verbose {
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::plan::Direction;
    use crate::progress::ProgressStage;

    #[test]
    fn compact_render_pump_caps_regular_frames_at_two_hundred_fifty_milliseconds() {
        let started = Instant::now();
        let mut pump = CompactRenderPump::new(started);

        assert_eq!(pump.opening_frame(), "| Testing against Cloudflare edge...");
        assert_eq!(
            pump.apply_event(
                &ProgressEvent::RequestStarted {
                    stage: ProgressStage::Transfer {
                        direction: Direction::Download,
                        requested_bytes: 100_000_000,
                    },
                    current: Some(1),
                    total: Some(3),
                },
                started + Duration::from_millis(50),
            ),
            None,
        );
        assert_eq!(
            pump.apply_event(
                &ProgressEvent::TransferAdvanced {
                    direction: Direction::Download,
                    requested_bytes: 100_000_000,
                    current: 1,
                    total: 3,
                    transferred_bytes: 63_000_000,
                    window_bytes: 20_062_500,
                    window_duration_ms: 250.0,
                },
                started + Duration::from_millis(120),
            ),
            None,
        );
        assert_eq!(pump.refresh(started + Duration::from_millis(249)), None);
        assert_eq!(
            pump.refresh(started + Duration::from_millis(250)),
            Some("/ Download 100 MB 1/3 · 642 Mbps · 63%".to_owned()),
        );

        assert_eq!(
            pump.apply_event(
                &ProgressEvent::LoadedLatencyCompleted {
                    direction: Direction::Download,
                    sequence: 1,
                    latency_ms: 32.4,
                },
                started + Duration::from_millis(300),
            ),
            None,
        );
        assert_eq!(pump.refresh(started + Duration::from_millis(499)), None);
        assert_eq!(
            pump.refresh(started + Duration::from_millis(500)),
            Some("- Download 100 MB 1/3 · 642 Mbps · 63% · loaded 32.4 ms".to_owned()),
        );
    }

    #[test]
    fn compact_render_pump_forces_a_pending_final_frame() {
        let started = Instant::now();
        let mut pump = CompactRenderPump::new(started);
        let _ = pump.opening_frame();

        assert_eq!(
            pump.apply_event(
                &ProgressEvent::RequestStarted {
                    stage: ProgressStage::Transfer {
                        direction: Direction::Upload,
                        requested_bytes: 50_000_000,
                    },
                    current: Some(2),
                    total: Some(3),
                },
                started + Duration::from_millis(40),
            ),
            None,
        );
        assert_eq!(
            pump.apply_event(
                &ProgressEvent::TransferCompleted {
                    direction: Direction::Upload,
                    requested_bytes: 50_000_000,
                    current: 2,
                    total: 3,
                    bps: 318_000_000,
                    adjusted_duration_ms: 1_250.0,
                },
                started + Duration::from_millis(80),
            ),
            None,
        );

        assert_eq!(
            pump.finish(),
            Some("/ Upload 50 MB 2/3 · 318 Mbps · 100%".to_owned()),
        );
        assert_eq!(pump.finish(), None);
    }
}
