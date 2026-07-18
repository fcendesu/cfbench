use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

use crate::cancellation::CancellationToken;
use crate::error::TransportError;
use crate::plan::Direction;
use crate::results::LatencyPoint;
use crate::runner::MeasurementTransport;

const INITIAL_PROBE_DELAY: Duration = Duration::from_millis(20);
const PROBE_THROTTLE: Duration = Duration::from_millis(400);

#[derive(Debug, Default)]
pub(crate) struct LoadedProbeOutcome {
    pub points: Vec<LatencyPoint>,
    pub diagnostics: Vec<String>,
}

pub(crate) fn spawn_loaded_probe_loop<T>(
    transport: Arc<T>,
    direction: Direction,
    cancellation: CancellationToken,
) -> JoinHandle<LoadedProbeOutcome>
where
    T: MeasurementTransport,
{
    tokio::spawn(async move {
        let mut outcome = LoadedProbeOutcome::default();
        if wait_or_cancel(INITIAL_PROBE_DELAY, &cancellation).await {
            return outcome;
        }

        loop {
            let started = Instant::now();
            match transport.loaded_latency(direction, &cancellation).await {
                Ok(observation) => match crate::measurement::latency_point(observation) {
                    Ok(point) => outcome.points.push(point),
                    Err(error) => outcome
                        .diagnostics
                        .push(format!("loaded latency observation rejected: {error}")),
                },
                Err(TransportError::Cancelled) if cancellation.is_cancelled() => break,
                Err(error) => outcome
                    .diagnostics
                    .push(format!("loaded latency probe failed: {error}")),
            }

            let remaining = PROBE_THROTTLE.saturating_sub(started.elapsed());
            if wait_or_cancel(remaining, &cancellation).await {
                break;
            }
        }
        outcome
    })
}

async fn wait_or_cancel(duration: Duration, cancellation: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        () = cancellation.cancelled() => true,
        () = tokio::time::sleep(duration) => false,
    }
}
