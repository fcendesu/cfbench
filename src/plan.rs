use crate::config::RunConfig;

pub const CLOUDFLARE_SPEEDTEST_VERSION: &str = "v1.11.0";
pub const CLOUDFLARE_SPEEDTEST_COMMIT: &str = "cfc99a74fd8d5c2121d319aeb7894c6246202c65";

/// A bandwidth measurement direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Download,
    Upload,
}

/// One entry in the upstream-compatible measurement schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeasurementStep {
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

impl MeasurementStep {
    /// Returns the bandwidth direction for transfer steps.
    pub const fn direction(self) -> Option<Direction> {
        match self {
            Self::Download { .. } => Some(Direction::Download),
            Self::Upload { .. } => Some(Direction::Upload),
            Self::Latency { .. } | Self::PacketLossUnsupported { .. } => None,
        }
    }
}

/// A versioned measurement schedule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeasurementPlan {
    pub upstream_version: &'static str,
    pub upstream_commit: &'static str,
    pub steps: Vec<MeasurementStep>,
}

impl MeasurementPlan {
    /// Returns a plan with disabled transfer directions removed.
    ///
    /// Non-transfer entries, including packet-loss metadata, remain in their
    /// original source order.
    pub fn for_config(&self, config: &RunConfig) -> Self {
        let steps = self
            .steps
            .iter()
            .copied()
            .filter(|step| match step.direction() {
                Some(Direction::Download) => !config.no_download,
                Some(Direction::Upload) => !config.no_upload,
                None => true,
            })
            .collect();

        Self {
            upstream_version: self.upstream_version,
            upstream_commit: self.upstream_commit,
            steps,
        }
    }
}

const DEFAULT_CLOUDFLARE_STEPS: [MeasurementStep; 15] = [
    MeasurementStep::Latency { packets: 1 },
    MeasurementStep::Download {
        bytes: 100_000,
        count: 1,
        bypass_finish: true,
    },
    MeasurementStep::Latency { packets: 20 },
    MeasurementStep::Download {
        bytes: 100_000,
        count: 9,
        bypass_finish: false,
    },
    MeasurementStep::Download {
        bytes: 1_000_000,
        count: 8,
        bypass_finish: false,
    },
    MeasurementStep::Upload {
        bytes: 100_000,
        count: 8,
        bypass_finish: false,
    },
    MeasurementStep::PacketLossUnsupported {
        packets: 1_000,
        responses_wait_ms: 3_000,
    },
    MeasurementStep::Upload {
        bytes: 1_000_000,
        count: 6,
        bypass_finish: false,
    },
    MeasurementStep::Download {
        bytes: 10_000_000,
        count: 6,
        bypass_finish: false,
    },
    MeasurementStep::Upload {
        bytes: 10_000_000,
        count: 4,
        bypass_finish: false,
    },
    MeasurementStep::Download {
        bytes: 25_000_000,
        count: 4,
        bypass_finish: false,
    },
    MeasurementStep::Upload {
        bytes: 25_000_000,
        count: 4,
        bypass_finish: false,
    },
    MeasurementStep::Download {
        bytes: 100_000_000,
        count: 3,
        bypass_finish: false,
    },
    MeasurementStep::Upload {
        bytes: 50_000_000,
        count: 3,
        bypass_finish: false,
    },
    MeasurementStep::Download {
        bytes: 250_000_000,
        count: 2,
        bypass_finish: false,
    },
];

/// Returns the immutable upstream baseline copied from a compile-time fixture.
pub fn default_cloudflare_plan() -> MeasurementPlan {
    MeasurementPlan {
        upstream_version: CLOUDFLARE_SPEEDTEST_VERSION,
        upstream_commit: CLOUDFLARE_SPEEDTEST_COMMIT,
        steps: DEFAULT_CLOUDFLARE_STEPS.to_vec(),
    }
}
