use serde::Serialize;

use super::{RawResults, reduce};

pub const SCHEMA_VERSION: u32 = 1;

/// Deterministic reductions over raw points.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub unloaded_latency_ms: Option<f64>,
    pub unloaded_jitter_ms: Option<f64>,
    pub download_bps: Option<u64>,
    pub download_loaded_latency_ms: Option<f64>,
    pub download_loaded_jitter_ms: Option<f64>,
    pub upload_bps: Option<u64>,
    pub upload_loaded_latency_ms: Option<f64>,
    pub upload_loaded_jitter_ms: Option<f64>,
    pub packet_loss_ratio: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PacketLossResult {
    pub status: String,
    pub reason: String,
    pub ratio: Option<f64>,
}

impl PacketLossResult {
    pub fn unavailable() -> Self {
        Self {
            status: "unavailable".to_owned(),
            reason: "turn_not_implemented".to_owned(),
            ratio: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TargetInfo {
    pub provider: String,
    pub ip_family: Option<String>,
    pub http_version: Option<String>,
    pub timing_model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Usage {
    pub download_payload_bytes: u64,
    pub upload_payload_bytes: u64,
    pub duration_ms: f64,
}

/// Stable JSON result envelope.
#[derive(Clone, Debug, Serialize)]
pub struct RunResult {
    pub schema_version: u32,
    pub client: ClientInfo,
    pub target: TargetInfo,
    pub summary: Summary,
    pub usage: Usage,
    #[serde(rename = "points")]
    pub raw: RawResults,
    pub packet_loss: PacketLossResult,
    pub failures: Vec<String>,
    pub diagnostics: Vec<String>,
}

impl RunResult {
    pub fn empty() -> Self {
        let raw = RawResults::default();
        Self {
            schema_version: SCHEMA_VERSION,
            client: ClientInfo {
                name: "cfbench".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            target: TargetInfo {
                provider: "cloudflare".to_owned(),
                ip_family: None,
                http_version: None,
                timing_model: "native_reqwest_v1".to_owned(),
            },
            summary: reduce(&raw),
            usage: Usage::default(),
            raw,
            packet_loss: PacketLossResult::unavailable(),
            failures: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}
