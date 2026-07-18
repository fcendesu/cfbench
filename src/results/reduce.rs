use crate::plan::Direction;
use crate::statistics::{jitter, percentile};

use super::{BandwidthPoint, LatencyPoint, RawResults, Summary};

const MIN_BANDWIDTH_DURATION_MS: f64 = 10.0;
const MAX_LOADED_LATENCY_POINTS: usize = 20;

/// Reduces raw points according to the pinned Cloudflare-compatible rules.
pub fn reduce(raw: &RawResults) -> Summary {
    let unloaded = latency_values(&raw.latency);
    let download_loaded = latest_latency_values(&raw.download_loaded_latency);
    let upload_loaded = latest_latency_values(&raw.upload_loaded_latency);

    Summary {
        unloaded_latency_ms: percentile(&unloaded, 0.5),
        unloaded_jitter_ms: jitter(&unloaded),
        download_bps: bandwidth(&raw.download, Direction::Download),
        download_loaded_latency_ms: percentile(&download_loaded, 0.5),
        download_loaded_jitter_ms: jitter(&download_loaded),
        upload_bps: bandwidth(&raw.upload, Direction::Upload),
        upload_loaded_latency_ms: percentile(&upload_loaded, 0.5),
        upload_loaded_jitter_ms: jitter(&upload_loaded),
        packet_loss_ratio: None,
    }
}

fn latency_values(points: &[LatencyPoint]) -> Vec<f64> {
    if points.iter().any(|point| {
        !point.ping_ms.is_finite()
            || !point.ttfb_ms.is_finite()
            || !point.server_time_ms.is_finite()
    }) {
        return vec![f64::NAN];
    }

    points.iter().map(|point| point.ping_ms).collect()
}

fn latest_latency_values(points: &[LatencyPoint]) -> Vec<f64> {
    let start = points.len().saturating_sub(MAX_LOADED_LATENCY_POINTS);
    latency_values(&points[start..])
}

fn bandwidth(points: &[BandwidthPoint], direction: Direction) -> Option<u64> {
    if points.iter().any(|point| {
        point.direction != direction
            || !point.duration_ms.is_finite()
            || !point.adjusted_duration_ms.is_finite()
            || !point.ping_ms.is_finite()
            || !point.server_time_ms.is_finite()
    }) {
        return None;
    }

    let eligible = points
        .iter()
        .filter(|point| point.adjusted_duration_ms >= MIN_BANDWIDTH_DURATION_MS)
        .map(|point| point.bps as f64)
        .collect::<Vec<_>>();
    let reduced = percentile(&eligible, 0.9)?.round();

    (reduced.is_finite() && (0.0..=u64::MAX as f64).contains(&reduced)).then_some(reduced as u64)
}
