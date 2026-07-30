use serde::Serialize;
use serde::ser::Serializer;

use crate::plan::Direction;

const MAX_LOADED_LATENCY_POINTS: usize = 20;

/// One finite native latency observation.
#[derive(Clone, Debug, Serialize)]
pub struct LatencyPoint {
    pub ping_ms: f64,
    pub ttfb_ms: f64,
    pub server_time_ms: f64,
    pub http_version: Option<String>,
    pub measured_at_unix_ms: i64,
}

/// One finite native transfer observation.
#[derive(Clone, Debug, Serialize)]
pub struct BandwidthPoint {
    #[serde(serialize_with = "serialize_direction")]
    pub direction: Direction,
    pub requested_bytes: u64,
    pub payload_bytes: u64,
    pub duration_ms: f64,
    pub adjusted_duration_ms: f64,
    pub ping_ms: f64,
    pub server_time_ms: f64,
    pub bps: u64,
    pub http_version: Option<String>,
    pub measured_at_unix_ms: i64,
}

fn serialize_direction<S>(direction: &Direction, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(match direction {
        Direction::Download => "download",
        Direction::Upload => "upload",
    })
}

/// A bounded, measurement-ordered collection of loaded-latency points.
///
/// Pushing beyond the compatibility limit evicts the oldest point, so every
/// consumer (including serialization) observes at most the latest 20 points.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(transparent)]
pub struct LoadedLatencyPoints {
    points: Vec<LatencyPoint>,
}

impl LoadedLatencyPoints {
    pub fn push(&mut self, point: LatencyPoint) {
        if self.points.len() == MAX_LOADED_LATENCY_POINTS {
            self.points.remove(0);
        }
        self.points.push(point);
    }

    pub fn extend<I>(&mut self, points: I)
    where
        I: IntoIterator<Item = LatencyPoint>,
    {
        for point in points {
            self.push(point);
        }
    }

    pub fn as_slice(&self) -> &[LatencyPoint] {
        &self.points
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

impl FromIterator<LatencyPoint> for LoadedLatencyPoints {
    fn from_iter<I>(points: I) -> Self
    where
        I: IntoIterator<Item = LatencyPoint>,
    {
        let mut retained = Self::default();
        retained.extend(points);
        retained
    }
}

/// Raw successful points retained by a run.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RawResults {
    pub latency: Vec<LatencyPoint>,
    pub download: Vec<BandwidthPoint>,
    pub upload: Vec<BandwidthPoint>,
    pub download_loaded_latency: LoadedLatencyPoints,
    pub upload_loaded_latency: LoadedLatencyPoints,
}
