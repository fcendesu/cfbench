use std::fmt;
use std::time::Duration;

use crate::plan::Direction;
use crate::results::{BandwidthPoint, LatencyPoint};

const TRANSFER_OVERHEAD_FACTOR: f64 = 1.005;
const MIN_ADJUSTED_DURATION_MS: f64 = 0.01;

/// Monotonic timing boundaries and payload accounting returned by a transport.
#[derive(Clone, Debug)]
pub struct TimingObservation {
    pub ttfb: Duration,
    pub total: Duration,
    pub server_time: Duration,
    pub payload_bytes: u64,
    pub http_version: Option<String>,
    valid: bool,
}

impl TimingObservation {
    /// Creates a valid observation from monotonic durations.
    pub fn new(
        ttfb: Duration,
        total: Duration,
        server_time: Duration,
        payload_bytes: u64,
        http_version: Option<String>,
    ) -> Self {
        Self {
            ttfb,
            total,
            server_time,
            payload_bytes,
            http_version,
            valid: true,
        }
    }

    /// Test and fixture convenience constructor using millisecond values.
    ///
    /// Invalid floating-point inputs are retained as an invalid observation so
    /// conversion can return a normal error instead of panicking.
    pub fn from_millis(
        ttfb_ms: f64,
        total_ms: f64,
        server_time_ms: f64,
        payload_bytes: u64,
        http_version: impl Into<String>,
    ) -> Self {
        let ttfb = duration_from_millis(ttfb_ms);
        let total = duration_from_millis(total_ms);
        let server_time = duration_from_millis(server_time_ms);
        let valid = ttfb.is_some() && total.is_some() && server_time.is_some();

        Self {
            ttfb: ttfb.unwrap_or_default(),
            total: total.unwrap_or_default(),
            server_time: server_time.unwrap_or_default(),
            payload_bytes,
            http_version: Some(http_version.into()),
            valid,
        }
    }
}

fn duration_from_millis(milliseconds: f64) -> Option<Duration> {
    if !milliseconds.is_finite() || milliseconds.is_sign_negative() {
        return None;
    }

    Duration::try_from_secs_f64(milliseconds / 1_000.0).ok()
}

/// A raw observation could not be represented as a finite result point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeasurementConversionError {
    InvalidTiming,
    InvalidBandwidth,
}

impl fmt::Display for MeasurementConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTiming => formatter.write_str("timing observation is invalid"),
            Self::InvalidBandwidth => formatter.write_str("computed bandwidth is invalid"),
        }
    }
}

impl std::error::Error for MeasurementConversionError {}

/// Converts a transport observation into a validated latency point.
pub fn latency_point(
    observation: TimingObservation,
) -> Result<LatencyPoint, MeasurementConversionError> {
    validate_observation(&observation)?;

    let ttfb_ms = duration_millis(observation.ttfb);
    let server_time_ms = observation.server_time.as_secs_f64() * 1_000.0;
    let ping_ms = duration_millis(observation.ttfb.saturating_sub(observation.server_time));
    ensure_finite(&[ttfb_ms, server_time_ms, ping_ms])?;

    Ok(LatencyPoint {
        ping_ms,
        ttfb_ms,
        server_time_ms,
        http_version: observation.http_version,
    })
}

/// Converts a transport observation into a validated bandwidth point.
pub fn bandwidth_point(
    direction: Direction,
    requested_bytes: u64,
    observation: TimingObservation,
) -> Result<BandwidthPoint, MeasurementConversionError> {
    validate_observation(&observation)?;

    let server_time = observation.server_time.as_secs_f64();
    let minimum_adjusted = Duration::from_micros((MIN_ADJUSTED_DURATION_MS * 1_000.0) as u64);
    let adjusted_duration = observation
        .total
        .saturating_sub(observation.server_time)
        .max(minimum_adjusted);
    let adjusted = adjusted_duration.as_secs_f64();
    let bps =
        ((observation.payload_bytes as f64 * TRANSFER_OVERHEAD_FACTOR * 8.0) / adjusted).round();
    if !bps.is_finite() || !(0.0..=u64::MAX as f64).contains(&bps) {
        return Err(MeasurementConversionError::InvalidBandwidth);
    }

    let duration_ms = duration_millis(observation.total);
    let adjusted_duration_ms = duration_millis(adjusted_duration);
    let server_time_ms = server_time * 1_000.0;
    let ping_ms = duration_millis(observation.ttfb.saturating_sub(observation.server_time));
    ensure_finite(&[duration_ms, adjusted_duration_ms, ping_ms, server_time_ms])?;

    Ok(BandwidthPoint {
        direction,
        requested_bytes,
        payload_bytes: observation.payload_bytes,
        duration_ms,
        adjusted_duration_ms,
        ping_ms,
        server_time_ms,
        bps: bps as u64,
        http_version: observation.http_version,
    })
}

fn validate_observation(observation: &TimingObservation) -> Result<(), MeasurementConversionError> {
    if observation.valid {
        Ok(())
    } else {
        Err(MeasurementConversionError::InvalidTiming)
    }
}

fn ensure_finite(values: &[f64]) -> Result<(), MeasurementConversionError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(MeasurementConversionError::InvalidTiming)
    }
}

fn duration_millis(duration: Duration) -> f64 {
    duration.as_secs() as f64 * 1_000.0 + f64::from(duration.subsec_nanos()) / 1_000_000.0
}
