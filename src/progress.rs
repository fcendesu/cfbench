//! Reqwest-independent progress events for measurement orchestration.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use crate::plan::Direction;

/// A completed or informational measurement event for line-oriented progress.
#[derive(Clone, Debug, PartialEq)]
pub enum ProgressEvent {
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
    /// The unsupported packet-loss stage was reached.
    PacketLossUnavailable,
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
