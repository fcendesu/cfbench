use crate::plan::Direction;
use crate::progress::{ProgressEvent, ProgressFailureKind, ProgressStage};

const UNAVAILABLE: &str = "unavailable";

/// Renders one stable, line-oriented progress event without terminal control.
pub fn render_progress(event: &ProgressEvent) -> String {
    match event {
        ProgressEvent::RequestStarted {
            stage,
            current,
            total,
        } => format!("[{}] started", failure_label(*stage, *current, *total)),
        ProgressEvent::LatencyCompleted {
            current,
            total,
            latency_ms,
        } => {
            let label = format!("latency {current}/{total}");
            format_measurement(&label, *latency_ms, format_latency)
        }
        ProgressEvent::TransferCompleted {
            direction,
            requested_bytes,
            current,
            total,
            bps,
            adjusted_duration_ms,
        } => {
            let label = format!(
                "{} {} {current}/{total}",
                direction_name(*direction),
                payload_label(*requested_bytes)
            );
            if !adjusted_duration_ms.is_finite() || *adjusted_duration_ms < 0.0 {
                return unavailable(&label);
            }
            format!(
                "[{label}] {:.2} Mbps — {}",
                *bps as f64 / 1_000_000.0,
                format_duration(*adjusted_duration_ms)
            )
        }
        ProgressEvent::TransferAdvanced {
            direction,
            requested_bytes,
            current,
            total,
            window_bytes,
            window_duration_ms,
            ..
        } => {
            let label = format!(
                "{} {} {current}/{total}",
                direction_name(*direction),
                payload_label(*requested_bytes)
            );
            if !window_duration_ms.is_finite() || *window_duration_ms <= 0.0 {
                return unavailable(&label);
            }
            format!(
                "[{label}] {} transferred — {:.2} Mbps",
                payload_label(*window_bytes),
                *window_bytes as f64 * 8.0 / *window_duration_ms / 1_000.0,
            )
        }
        ProgressEvent::LoadedLatencyCompleted {
            direction,
            sequence,
            latency_ms,
        } => {
            let label = format!("loaded/{} {sequence}", direction_name(*direction));
            format_measurement(&label, *latency_ms, format_latency)
        }
        ProgressEvent::RequestFailed {
            stage,
            current,
            total,
            kind,
        } => format!(
            "[{}] failed — {}",
            failure_label(*stage, *current, *total),
            failure_detail(*kind)
        ),
        ProgressEvent::DirectionFinished { direction } => format!(
            "[{}] larger payload groups skipped — request duration threshold reached",
            direction_name(*direction)
        ),
    }
}

/// Renders the active terminal status for a progress event.
pub fn render_compact_progress(event: &ProgressEvent) -> Option<String> {
    match event {
        ProgressEvent::RequestStarted {
            stage,
            current,
            total,
        } => Some(compact_stage(*stage, *current, *total)),
        ProgressEvent::LatencyCompleted {
            current,
            total,
            latency_ms,
        } => valid_nonnegative(*latency_ms)
            .then(|| format!("Latency {current}/{total} - {latency_ms:.2} ms")),
        ProgressEvent::TransferCompleted {
            direction,
            requested_bytes,
            current,
            total,
            bps,
            ..
        } => Some(format!(
            "{} {} {current}/{total} - {:.2} Mbps",
            title_direction(*direction),
            payload_label(*requested_bytes),
            *bps as f64 / 1_000_000.0,
        )),
        ProgressEvent::TransferAdvanced { .. } => None,
        ProgressEvent::LoadedLatencyCompleted { .. } => None,
        ProgressEvent::RequestFailed {
            stage: ProgressStage::LoadedLatency { .. },
            ..
        } => None,
        ProgressEvent::RequestFailed {
            stage,
            current,
            total,
            kind,
        } => Some(format!(
            "{} - failed: {}",
            compact_stage(*stage, *current, *total),
            failure_detail(*kind),
        )),
        ProgressEvent::DirectionFinished { direction } => {
            Some(format!("{} complete", title_direction(*direction)))
        }
    }
}

fn valid_nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

const fn title_direction(direction: Direction) -> &'static str {
    match direction {
        Direction::Download => "Download",
        Direction::Upload => "Upload",
    }
}

fn compact_stage(stage: ProgressStage, current: Option<u16>, total: Option<u16>) -> String {
    let label = match stage {
        ProgressStage::Latency => "Latency".to_owned(),
        ProgressStage::Transfer {
            direction,
            requested_bytes,
        } => format!(
            "{} {}",
            title_direction(direction),
            payload_label(requested_bytes)
        ),
        ProgressStage::LoadedLatency { direction } => {
            format!("{} loaded latency", title_direction(direction))
        }
    };
    match (current, total) {
        (Some(current), Some(total)) => format!("{label} {current}/{total}"),
        (Some(current), None) => format!("{label} {current}"),
        (None, Some(total)) => format!("{label} ?/{total}"),
        (None, None) => label,
    }
}

fn format_measurement(label: &str, value: f64, formatter: fn(f64) -> String) -> String {
    if !value.is_finite() || value < 0.0 {
        unavailable(label)
    } else {
        format!("[{label}] {}", formatter(value))
    }
}

fn unavailable(label: &str) -> String {
    format!("[{label}] {UNAVAILABLE}")
}

fn format_latency(latency_ms: f64) -> String {
    format!("{latency_ms:.2} ms")
}

fn format_duration(duration_ms: f64) -> String {
    if duration_ms < 1_000.0 {
        format!("{duration_ms:.1} ms")
    } else {
        format!("{:.2} s", duration_ms / 1_000.0)
    }
}

fn failure_label(stage: ProgressStage, current: Option<u16>, total: Option<u16>) -> String {
    let stage = stage_label(stage);
    match (current, total) {
        (Some(current), Some(total)) => format!("{stage} {current}/{total}"),
        (Some(current), None) => format!("{stage} {current}"),
        (None, Some(total)) => format!("{stage} ?/{total}"),
        (None, None) => stage,
    }
}

fn stage_label(stage: ProgressStage) -> String {
    match stage {
        ProgressStage::Latency => "latency".to_owned(),
        ProgressStage::Transfer {
            direction,
            requested_bytes,
        } => format!(
            "{} {}",
            direction_name(direction),
            payload_label(requested_bytes)
        ),
        ProgressStage::LoadedLatency { direction } => {
            format!("loaded/{}", direction_name(direction))
        }
    }
}

const fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Download => "download",
        Direction::Upload => "upload",
    }
}

fn payload_label(bytes: u64) -> String {
    if bytes >= 1_000_000 && bytes.is_multiple_of(1_000_000) {
        format!("{} MB", bytes / 1_000_000)
    } else if bytes >= 1_000 && bytes.is_multiple_of(1_000) {
        format!("{} KB", bytes / 1_000)
    } else {
        format!("{bytes} B")
    }
}

fn failure_detail(kind: ProgressFailureKind) -> String {
    match kind {
        ProgressFailureKind::HttpStatus(status) => format!("HTTP {status}"),
        ProgressFailureKind::Timeout => "timeout".to_owned(),
        ProgressFailureKind::Cancelled => "cancelled".to_owned(),
        ProgressFailureKind::BodyStream => "body stream".to_owned(),
        ProgressFailureKind::PayloadMismatch => "payload mismatch".to_owned(),
        ProgressFailureKind::InvalidMeasurement => "invalid measurement".to_owned(),
        ProgressFailureKind::Request => "request failed".to_owned(),
    }
}
