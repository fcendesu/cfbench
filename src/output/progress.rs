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

/// Stateful display data for the compact, single-line progress renderer.
#[derive(Debug, Default)]
pub struct CompactProgressState {
    previous_latency_ms: Option<f64>,
    jitter_delta_sum_ms: f64,
    jitter_pair_count: u64,
    loaded_download_ms: Option<f64>,
    loaded_upload_ms: Option<f64>,
    active_transfer: Option<ActiveTransferDisplay>,
}

#[derive(Debug)]
struct ActiveTransferDisplay {
    direction: Direction,
    requested_bytes: u64,
    current: Option<u16>,
    total: Option<u16>,
    provisional_mbps: Option<f64>,
    percentage: u8,
}

impl CompactProgressState {
    /// Applies one event and returns a replacement line when the display changes.
    pub fn render(&mut self, event: &ProgressEvent) -> Option<String> {
        match event {
            ProgressEvent::RequestStarted {
                stage: ProgressStage::Latency,
                current,
                total,
            } => Some(compact_stage(ProgressStage::Latency, *current, *total)),
            ProgressEvent::RequestStarted {
                stage:
                    ProgressStage::Transfer {
                        direction,
                        requested_bytes,
                    },
                current,
                total,
            } => {
                self.active_transfer = Some(ActiveTransferDisplay {
                    direction: *direction,
                    requested_bytes: *requested_bytes,
                    current: *current,
                    total: *total,
                    provisional_mbps: None,
                    percentage: 0,
                });
                self.render_active_transfer()
            }
            ProgressEvent::RequestStarted {
                stage: ProgressStage::LoadedLatency { .. },
                ..
            } => None,
            ProgressEvent::LatencyCompleted {
                current,
                total,
                latency_ms,
            } => self.render_latency(*current, *total, *latency_ms),
            ProgressEvent::TransferAdvanced {
                direction,
                requested_bytes,
                current,
                total,
                transferred_bytes,
                window_bytes,
                window_duration_ms,
            } => self.render_transfer_advance(
                *direction,
                *requested_bytes,
                *current,
                *total,
                *transferred_bytes,
                *window_bytes,
                *window_duration_ms,
            ),
            ProgressEvent::TransferCompleted {
                direction,
                requested_bytes,
                current,
                total,
                bps,
                ..
            } => {
                let completed = ActiveTransferDisplay {
                    direction: *direction,
                    requested_bytes: *requested_bytes,
                    current: Some(*current),
                    total: Some(*total),
                    provisional_mbps: Some(*bps as f64 / 1_000_000.0),
                    percentage: 100,
                };
                let message = self.render_transfer(&completed);
                self.active_transfer = None;
                Some(message)
            }
            ProgressEvent::LoadedLatencyCompleted {
                direction,
                latency_ms,
                ..
            } => {
                if !valid_nonnegative(*latency_ms) {
                    return None;
                }
                match direction {
                    Direction::Download => self.loaded_download_ms = Some(*latency_ms),
                    Direction::Upload => self.loaded_upload_ms = Some(*latency_ms),
                }
                self.active_transfer
                    .as_ref()
                    .filter(|active| active.direction == *direction)
                    .map(|active| self.render_transfer(active))
            }
            ProgressEvent::RequestFailed {
                stage: ProgressStage::LoadedLatency { .. },
                ..
            } => None,
            ProgressEvent::RequestFailed {
                stage,
                current,
                total,
                kind,
            } => {
                if let ProgressStage::Transfer {
                    direction,
                    requested_bytes,
                } = stage
                    && self.active_transfer.as_ref().is_some_and(|active| {
                        active.matches(*direction, *requested_bytes, *current, *total)
                    })
                {
                    self.active_transfer = None;
                }
                Some(format!(
                    "{} · failed: {}",
                    compact_stage(*stage, *current, *total),
                    failure_detail(*kind),
                ))
            }
            ProgressEvent::DirectionFinished { direction } => {
                if self
                    .active_transfer
                    .as_ref()
                    .is_some_and(|active| active.direction == *direction)
                {
                    self.active_transfer = None;
                }
                Some(format!("{} complete", title_direction(*direction)))
            }
        }
    }

    fn render_latency(&mut self, current: u16, total: u16, latency_ms: f64) -> Option<String> {
        if !valid_nonnegative(latency_ms) {
            return None;
        }

        if let Some(previous_latency_ms) = self.previous_latency_ms {
            let delta_ms = (latency_ms - previous_latency_ms).abs();
            if delta_ms.is_finite() {
                self.jitter_delta_sum_ms += delta_ms;
                self.jitter_pair_count = self.jitter_pair_count.saturating_add(1);
            }
        }
        self.previous_latency_ms = Some(latency_ms);

        let mut line = format!("Latency {current}/{total} · {latency_ms:.1} ms");
        if self.jitter_pair_count > 0 {
            let jitter_ms = self.jitter_delta_sum_ms / self.jitter_pair_count as f64;
            line.push_str(&format!(" · jitter {jitter_ms:.1} ms"));
        }
        Some(line)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_transfer_advance(
        &mut self,
        direction: Direction,
        requested_bytes: u64,
        current: u16,
        total: u16,
        transferred_bytes: u64,
        window_bytes: u64,
        window_duration_ms: f64,
    ) -> Option<String> {
        if requested_bytes == 0
            || transferred_bytes > requested_bytes
            || window_bytes == 0
            || window_bytes > transferred_bytes
            || !window_duration_ms.is_finite()
            || window_duration_ms <= 0.0
        {
            return None;
        }

        let active = self.active_transfer.as_mut()?;
        if !active.matches(direction, requested_bytes, Some(current), Some(total)) {
            return None;
        }

        let provisional_mbps = window_bytes as f64 * 8.0 / window_duration_ms / 1_000.0;
        if !valid_nonnegative(provisional_mbps) {
            return None;
        }
        let percentage = ((transferred_bytes as u128 * 100) / requested_bytes as u128).min(100);
        active.provisional_mbps = Some(provisional_mbps);
        active.percentage = percentage as u8;
        self.render_active_transfer()
    }

    fn render_active_transfer(&self) -> Option<String> {
        self.active_transfer
            .as_ref()
            .map(|active| self.render_transfer(active))
    }

    fn render_transfer(&self, active: &ActiveTransferDisplay) -> String {
        let mut line = compact_stage(
            ProgressStage::Transfer {
                direction: active.direction,
                requested_bytes: active.requested_bytes,
            },
            active.current,
            active.total,
        );
        if let Some(provisional_mbps) = active.provisional_mbps {
            line.push_str(&format!(" · {} Mbps", format_rate(provisional_mbps)));
        }
        line.push_str(&format!(" · {}%", active.percentage));
        if let Some(loaded_ms) = self.loaded_latency(active.direction) {
            line.push_str(&format!(" · loaded {loaded_ms:.1} ms"));
        }
        line
    }

    const fn loaded_latency(&self, direction: Direction) -> Option<f64> {
        match direction {
            Direction::Download => self.loaded_download_ms,
            Direction::Upload => self.loaded_upload_ms,
        }
    }
}

impl ActiveTransferDisplay {
    fn matches(
        &self,
        direction: Direction,
        requested_bytes: u64,
        current: Option<u16>,
        total: Option<u16>,
    ) -> bool {
        self.direction == direction
            && self.requested_bytes == requested_bytes
            && self.current == current
            && self.total == total
    }
}

fn format_rate(mbps: f64) -> String {
    if mbps >= 100.0 {
        format!("{mbps:.0}")
    } else {
        format!("{mbps:.1}")
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
