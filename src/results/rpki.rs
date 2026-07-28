use serde::Serialize;

/// Result of the optional RPKI-invalid-route reachability diagnostic.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct RpkiReachability {
    pub status: RpkiReachabilityStatus,
    pub host: Option<String>,
    pub detail: Option<String>,
}

/// Informational classification for the RPKI-invalid-route diagnostic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RpkiReachabilityStatus {
    #[default]
    NotRequested,
    Reachable,
    Unreachable,
    Error,
}
