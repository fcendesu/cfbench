use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

use crate::cancellation::CancellationToken;
use crate::clock::RunClock;
use crate::error::TransportError;
use crate::plan::Direction;
use crate::progress::{ProgressEvent, ProgressFailureKind, ProgressReporter, ProgressStage};
use crate::results::LatencyPoint;
use crate::runner::MeasurementTransport;

const INITIAL_PROBE_DELAY: Duration = Duration::from_millis(20);
const PROBE_THROTTLE: Duration = Duration::from_millis(400);

#[derive(Debug, Default)]
pub(crate) struct LoadedProbeOutcome {
    pub points: Vec<LatencyPoint>,
    pub diagnostics: Vec<String>,
    pub http_versions: Vec<String>,
    pub ip_families: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct LoadedProbeCancellation {
    group: CancellationToken,
    parent: CancellationToken,
    normal_group_shutdown: Arc<AtomicBool>,
}

impl LoadedProbeCancellation {
    pub fn child_of(parent: &CancellationToken) -> Self {
        Self {
            group: parent.child_token(),
            parent: parent.clone(),
            normal_group_shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel_group(&self, normal_group_shutdown: bool) {
        if normal_group_shutdown {
            self.normal_group_shutdown.store(true, Ordering::Release);
        }
        self.group.cancel();
    }

    fn should_report_parent_cancellation(&self) -> bool {
        self.parent.is_cancelled() && !self.normal_group_shutdown.load(Ordering::Acquire)
    }
}

pub(crate) fn spawn_loaded_probe_loop<T>(
    transport: Arc<T>,
    direction: Direction,
    cancellation: LoadedProbeCancellation,
    progress: ProgressReporter,
    sequence: Arc<AtomicU64>,
    clock: RunClock,
) -> JoinHandle<LoadedProbeOutcome>
where
    T: MeasurementTransport,
{
    tokio::spawn(async move {
        let mut outcome = LoadedProbeOutcome::default();
        if wait_or_cancel(INITIAL_PROBE_DELAY, &cancellation.group).await {
            return outcome;
        }

        loop {
            let started = Instant::now();
            match transport
                .loaded_latency(direction, &cancellation.group)
                .await
            {
                Ok(observation) => {
                    let endpoint = observation.endpoint.clone();
                    if let Some(version) = observation.http_version.as_ref() {
                        outcome.http_versions.push(version.clone());
                    }
                    if let Some(family) = observation.ip_family.as_ref() {
                        outcome.ip_families.push(family.clone());
                    }
                    match crate::measurement::latency_point(observation, clock.now_unix_ms()) {
                        Ok(point) => {
                            let latency_ms = point.ping_ms;
                            let sequence = sequence.fetch_add(1, Ordering::Relaxed) + 1;
                            progress.emit(ProgressEvent::LoadedLatencyCompleted {
                                direction,
                                sequence,
                                latency_ms,
                            });
                            outcome.points.push(point);
                        }
                        Err(error) => {
                            progress.emit(ProgressEvent::RequestFailed {
                                stage: ProgressStage::LoadedLatency { direction },
                                current: None,
                                total: None,
                                kind: ProgressFailureKind::InvalidMeasurement,
                            });
                            outcome.diagnostics.push(format!(
                                "loaded latency conversion failed during {} for endpoint {endpoint}: {error}",
                                direction_name(direction)
                            ));
                        }
                    }
                }
                Err(TransportError::Cancelled { .. }) if cancellation.group.is_cancelled() => {
                    if cancellation.should_report_parent_cancellation() {
                        progress.emit(ProgressEvent::RequestFailed {
                            stage: ProgressStage::LoadedLatency { direction },
                            current: None,
                            total: None,
                            kind: ProgressFailureKind::Cancelled,
                        });
                    }
                    break;
                }
                Err(error) => {
                    let kind = crate::runner::progress_failure_kind(&error);
                    progress.emit(ProgressEvent::RequestFailed {
                        stage: ProgressStage::LoadedLatency { direction },
                        current: None,
                        total: None,
                        kind,
                    });
                    outcome.diagnostics.push(format!(
                        "loaded latency probe failed during {}: {error}",
                        direction_name(direction)
                    ));
                }
            }

            let remaining = PROBE_THROTTLE.saturating_sub(started.elapsed());
            if wait_or_cancel(remaining, &cancellation.group).await {
                break;
            }
        }
        outcome
    })
}

const fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Download => "download",
        Direction::Upload => "upload",
    }
}

async fn wait_or_cancel(duration: Duration, cancellation: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => true,
        () = tokio::time::sleep(duration) => false,
    }
}
