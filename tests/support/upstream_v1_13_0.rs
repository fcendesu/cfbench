use cfbench::plan::MeasurementStep;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FixtureStep {
    Latency {
        packets: u32,
    },
    Download {
        bytes: u64,
        count: u32,
        bypass_finish: bool,
    },
    Upload {
        bytes: u64,
        count: u32,
        bypass_finish: bool,
    },
    PacketLossUnsupported {
        packets: u32,
        responses_wait_ms: u32,
    },
}

impl From<FixtureStep> for MeasurementStep {
    fn from(step: FixtureStep) -> Self {
        match step {
            FixtureStep::Latency { packets } => Self::Latency { packets },
            FixtureStep::Download {
                bytes,
                count,
                bypass_finish,
            } => Self::Download {
                bytes,
                count,
                bypass_finish,
            },
            FixtureStep::Upload {
                bytes,
                count,
                bypass_finish,
            } => Self::Upload {
                bytes,
                count,
                bypass_finish,
            },
            FixtureStep::PacketLossUnsupported {
                packets,
                responses_wait_ms,
            } => Self::PacketLossUnsupported {
                packets,
                responses_wait_ms,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpstreamFixture {
    pub upstream_version: String,
    pub upstream_commit: String,
    pub source: String,
    pub expected_idle_latency_points: usize,
    pub schedule: Vec<FixtureStep>,
    pub constants: FixtureConstants,
    pub server_timing_cases: Vec<ServerTimingCase>,
    pub reductions: ReductionCases,
}

#[derive(Debug, Deserialize)]
pub struct FixtureConstants {
    pub estimated_server_time_ms: f64,
    pub server_time_min_duration_ms: f64,
    pub transfer_overhead_factor: f64,
    pub latency_percentile: f64,
    pub bandwidth_percentile: f64,
    pub bandwidth_min_request_duration_ms: f64,
    pub loaded_request_min_duration_ms: f64,
    pub loaded_latency_throttle_ms: u64,
    pub loaded_latency_max_points: usize,
    pub bandwidth_finish_request_duration_ms: f64,
}

#[derive(Debug, Deserialize)]
pub struct ServerTimingCase {
    pub header: String,
    pub expected_ms: f64,
}

#[derive(Debug, Deserialize)]
pub struct ReductionCases {
    pub latency_points_ms: Vec<f64>,
    pub latency_p50_ms: f64,
    pub latency_jitter_ms: f64,
    pub bandwidth_points_bps: Vec<f64>,
    pub bandwidth_p90_bps: f64,
}

pub fn fixture() -> UpstreamFixture {
    serde_json::from_str(include_str!(
        "../fixtures/cloudflare-speedtest-v1.13.0.json"
    ))
    .expect("pinned Cloudflare Speedtest fixture is valid")
}
