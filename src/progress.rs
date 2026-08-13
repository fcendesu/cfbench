//! Reqwest-independent progress events for measurement orchestration.

use std::{
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    time::{Duration, Instant},
};

use crate::plan::Direction;

const TRANSFER_TELEMETRY_INTERVAL: Duration = Duration::from_millis(250);

/// A completed or informational measurement event for line-oriented progress.
#[derive(Clone, Debug, PartialEq)]
pub enum ProgressEvent {
    /// A measurement request is about to begin.
    RequestStarted {
        stage: ProgressStage,
        current: Option<u16>,
        total: Option<u16>,
    },
    /// An accepted unloaded latency point.
    LatencyCompleted {
        current: u16,
        total: u16,
        latency_ms: f64,
    },
    /// An accepted download or upload point.
    TransferCompleted {
        direction: Direction,
        requested_bytes: u64,
        current: u16,
        total: u16,
        bps: u64,
        adjusted_duration_ms: f64,
    },
    /// A nonblocking in-progress transfer snapshot.
    TransferAdvanced {
        direction: Direction,
        requested_bytes: u64,
        current: u16,
        total: u16,
        transferred_bytes: u64,
        window_bytes: u64,
        window_duration_ms: f64,
    },
    /// A converted loaded-latency probe point.
    LoadedLatencyCompleted {
        direction: Direction,
        sequence: u64,
        latency_ms: f64,
    },
    /// A failed measurement request with a safe failure category.
    RequestFailed {
        stage: ProgressStage,
        current: Option<u16>,
        total: Option<u16>,
        kind: ProgressFailureKind,
    },
    /// A direction's later payload groups were skipped after adaptive stopping.
    DirectionFinished { direction: Direction },
}

/// The measurement stage that produced a request failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressStage {
    Latency,
    Transfer {
        direction: Direction,
        requested_bytes: u64,
    },
    LoadedLatency {
        direction: Direction,
    },
}

/// A sanitized failure classification suitable for line-oriented progress output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressFailureKind {
    HttpStatus(u16),
    Timeout,
    Cancelled,
    BodyStream,
    PayloadMismatch,
    InvalidMeasurement,
    Request,
}

/// A bounded, nonblocking sender for progress events.
///
/// Events are intentionally dropped when the receiver is slow or closed so
/// progress reporting never delays measurement timing or changes run results.
#[derive(Clone)]
pub struct ProgressReporter(Option<SyncSender<ProgressEvent>>);

impl ProgressReporter {
    /// Creates a bounded reporter and its receiver.
    pub fn channel(capacity: usize) -> (Self, Receiver<ProgressEvent>) {
        let (sender, receiver) = sync_channel(capacity);
        (Self(Some(sender)), receiver)
    }

    /// Creates a reporter that suppresses all events.
    pub const fn disabled() -> Self {
        Self(None)
    }

    /// Attempts to report an event without waiting for the receiver.
    pub fn emit(&self, event: ProgressEvent) {
        if let Some(sender) = &self.0 {
            let _ = sender.try_send(event);
        }
    }
}

/// Samples an individual transfer without affecting its measurement timing.
pub struct TransferTelemetry {
    reporter: ProgressReporter,
    direction: Direction,
    requested_bytes: u64,
    current: u16,
    total: u16,
    last_sample: Option<(Instant, u64)>,
    finished: bool,
}

impl TransferTelemetry {
    /// Creates a sampler for one planned transfer request.
    pub fn new(
        reporter: ProgressReporter,
        direction: Direction,
        requested_bytes: u64,
        current: u16,
        total: u16,
    ) -> Self {
        Self {
            reporter,
            direction,
            requested_bytes,
            current,
            total,
            last_sample: None,
            finished: false,
        }
    }

    /// Starts the transfer's sampling window.
    pub fn begin(&mut self) {
        self.begin_at(Instant::now());
    }

    /// Records transfer progress and optionally completes its final window.
    pub fn observe(&mut self, transferred_bytes: u64, finished: bool) {
        self.observe_at(transferred_bytes, Instant::now(), finished);
    }

    fn begin_at(&mut self, started: Instant) {
        self.last_sample = Some((started, 0));
        self.finished = false;
    }

    fn observe_at(&mut self, transferred_bytes: u64, observed_at: Instant, finished: bool) {
        if self.finished {
            return;
        }
        if finished {
            self.finished = true;
        }
        if transferred_bytes > self.requested_bytes {
            return;
        }

        let Some((last_sample_at, last_sample_bytes)) = self.last_sample else {
            return;
        };
        let Some(window_duration) = observed_at.checked_duration_since(last_sample_at) else {
            return;
        };
        let Some(window_bytes) = transferred_bytes.checked_sub(last_sample_bytes) else {
            return;
        };

        if !finished && window_duration < TRANSFER_TELEMETRY_INTERVAL {
            return;
        }
        if window_duration.is_zero() || window_bytes == 0 {
            return;
        }

        self.reporter.emit(ProgressEvent::TransferAdvanced {
            direction: self.direction,
            requested_bytes: self.requested_bytes,
            current: self.current,
            total: self.total,
            transferred_bytes,
            window_bytes,
            window_duration_ms: window_duration.as_secs_f64() * 1_000.0,
        });
        self.last_sample = Some((observed_at, transferred_bytes));
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{ProgressEvent, ProgressReporter, TransferTelemetry};
    use crate::plan::Direction;

    #[test]
    fn transfer_telemetry_throttles_intermediate_samples_and_emits_self_contained_windows() {
        let (reporter, receiver) = ProgressReporter::channel(8);
        let started = Instant::now();
        let mut telemetry = TransferTelemetry::new(reporter, Direction::Download, 1_000_000, 1, 3);
        telemetry.begin_at(started);
        telemetry.observe_at(100_000, started + Duration::from_millis(100), false);
        assert!(receiver.try_recv().is_err());

        telemetry.observe_at(400_000, started + Duration::from_millis(250), false);
        assert_eq!(
            receiver.try_recv().unwrap(),
            ProgressEvent::TransferAdvanced {
                direction: Direction::Download,
                requested_bytes: 1_000_000,
                current: 1,
                total: 3,
                transferred_bytes: 400_000,
                window_bytes: 400_000,
                window_duration_ms: 250.0,
            }
        );
    }

    #[test]
    fn transfer_telemetry_emits_a_final_sample_before_the_interval() {
        let (reporter, receiver) = ProgressReporter::channel(1);
        let started = Instant::now();
        let mut telemetry = TransferTelemetry::new(reporter, Direction::Upload, 1_000, 2, 3);
        telemetry.begin_at(started);

        telemetry.observe_at(100, started + Duration::from_millis(10), true);

        assert_eq!(
            receiver.try_recv().unwrap(),
            ProgressEvent::TransferAdvanced {
                direction: Direction::Upload,
                requested_bytes: 1_000,
                current: 2,
                total: 3,
                transferred_bytes: 100,
                window_bytes: 100,
                window_duration_ms: 10.0,
            }
        );
    }

    #[test]
    fn transfer_telemetry_emits_only_one_final_sample() {
        let (reporter, receiver) = ProgressReporter::channel(2);
        let started = Instant::now();
        let mut telemetry = TransferTelemetry::new(reporter, Direction::Upload, 1_000, 2, 3);
        telemetry.begin_at(started);

        telemetry.observe_at(100, started + Duration::from_millis(10), true);
        telemetry.observe_at(200, started + Duration::from_millis(20), true);

        assert_eq!(
            receiver.try_recv().unwrap(),
            ProgressEvent::TransferAdvanced {
                direction: Direction::Upload,
                requested_bytes: 1_000,
                current: 2,
                total: 3,
                transferred_bytes: 100,
                window_bytes: 100,
                window_duration_ms: 10.0,
            }
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn transfer_telemetry_suppresses_regressing_and_oversized_observations() {
        let (reporter, receiver) = ProgressReporter::channel(4);
        let started = Instant::now();
        let mut telemetry = TransferTelemetry::new(reporter, Direction::Download, 1_000, 1, 1);
        telemetry.begin_at(started);
        telemetry.observe_at(600, started + Duration::from_millis(250), false);
        assert!(receiver.try_recv().is_ok());

        telemetry.observe_at(500, started + Duration::from_millis(500), true);
        telemetry.observe_at(1_001, started + Duration::from_millis(750), true);

        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn transfer_telemetry_suppresses_zero_duration_windows() {
        let (reporter, receiver) = ProgressReporter::channel(1);
        let started = Instant::now();
        let mut telemetry = TransferTelemetry::new(reporter, Direction::Download, 1_000, 1, 1);
        telemetry.begin_at(started);

        telemetry.observe_at(100, started, true);

        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn transfer_telemetry_drops_samples_when_the_channel_is_full_or_closed() {
        let (reporter, receiver) = ProgressReporter::channel(1);
        let started = Instant::now();
        reporter.emit(ProgressEvent::DirectionFinished {
            direction: Direction::Download,
        });
        let mut telemetry = TransferTelemetry::new(reporter, Direction::Download, 1_000, 1, 1);
        telemetry.begin_at(started);
        telemetry.observe_at(100, started + Duration::from_millis(10), true);
        assert!(matches!(
            receiver.try_recv(),
            Ok(ProgressEvent::DirectionFinished { .. })
        ));

        let (reporter, receiver) = ProgressReporter::channel(1);
        drop(receiver);
        let mut telemetry = TransferTelemetry::new(reporter, Direction::Download, 1_000, 1, 1);
        telemetry.begin_at(started);
        telemetry.observe_at(100, started + Duration::from_millis(10), true);
    }
}
