use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
}

impl LoadedProbeCancellation {
    pub fn new(parent: &CancellationToken) -> Self {
        Self {
            // Parent cancellation and normal group teardown are deliberately
            // independent inputs to the biased selections below. A child token
            // would publish cancellation before its reason is linearized.
            group: CancellationToken::new(),
            parent: parent.clone(),
        }
    }

    pub fn cancel_group(&self) {
        self.group.cancel();
    }
}

enum ProbeWait {
    ParentCancelled,
    GroupCancelled,
    Elapsed,
}

enum ProbeRequest {
    ParentCancelled,
    GroupCancelled,
    Completed(Result<crate::measurement::TimingObservation, TransportError>),
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
        if !matches!(
            wait_or_cancel(INITIAL_PROBE_DELAY, &cancellation).await,
            ProbeWait::Elapsed
        ) {
            return outcome;
        }

        loop {
            let started = Instant::now();
            let request = transport.loaded_latency(direction, &cancellation.group);
            tokio::pin!(request);
            let selection = tokio::select! {
                biased;
                () = cancellation.parent.cancelled() => ProbeRequest::ParentCancelled,
                () = cancellation.group.cancelled() => ProbeRequest::GroupCancelled,
                result = &mut request => ProbeRequest::Completed(result),
            };
            let request = match selection {
                ProbeRequest::ParentCancelled => {
                    cancellation.group.cancel();
                    let _ = request.await;
                    progress.emit(ProgressEvent::RequestFailed {
                        stage: ProgressStage::LoadedLatency { direction },
                        current: None,
                        total: None,
                        kind: ProgressFailureKind::Cancelled,
                    });
                    break;
                }
                ProbeRequest::GroupCancelled => {
                    let _ = request.await;
                    break;
                }
                ProbeRequest::Completed(result) => result,
            };

            match request {
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
            if !matches!(
                wait_or_cancel(remaining, &cancellation).await,
                ProbeWait::Elapsed
            ) {
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

async fn wait_or_cancel(duration: Duration, cancellation: &LoadedProbeCancellation) -> ProbeWait {
    tokio::select! {
        biased;
        () = cancellation.parent.cancelled() => ProbeWait::ParentCancelled,
        () = cancellation.group.cancelled() => ProbeWait::GroupCancelled,
        () = tokio::time::sleep(duration) => ProbeWait::Elapsed,
    }
}
