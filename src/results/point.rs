use serde::Serialize;
use serde::ser::Serializer;

use crate::plan::Direction;

/// One finite native latency observation.
#[derive(Clone, Debug, Serialize)]
pub struct LatencyPoint {
    pub ping_ms: f64,
    pub ttfb_ms: f64,
    pub server_time_ms: f64,
    pub http_version: Option<String>,
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

/// Raw successful points retained by a run.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RawResults {
    /// The first estimate is private to orchestration and not part of public points.
    #[serde(skip_serializing)]
    pub initial_latency: Vec<LatencyPoint>,
    pub latency: Vec<LatencyPoint>,
    pub download: Vec<BandwidthPoint>,
    pub upload: Vec<BandwidthPoint>,
    pub download_loaded_latency: Vec<LatencyPoint>,
    pub upload_loaded_latency: Vec<LatencyPoint>,
}
