use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("--ipv4 and --ipv6 cannot be used together")]
    ConflictingIpModes,
    #[error("timeout must be between 1 and 300 seconds, received {0}")]
    InvalidTimeout(u64),
}

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("could not serialize result as JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// Failures at a native HTTP measurement boundary.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("measurement was cancelled")]
    Cancelled,
    #[error("timed out waiting for response headers from endpoint {endpoint}")]
    HeaderTimeout { endpoint: String },
    #[error("timed out while reading the response body from endpoint {endpoint}")]
    BodyTimeout { endpoint: String },
    #[error("HTTP request failed for endpoint {endpoint}: {source}")]
    Request {
        endpoint: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("endpoint {endpoint} returned HTTP status {status}")]
    HttpStatus { endpoint: String, status: u16 },
    #[error("response body stream failed for endpoint {endpoint}: {source}")]
    BodyStream {
        endpoint: String,
        #[source]
        source: reqwest::Error,
    },
    #[error(
        "download payload mismatch from endpoint {endpoint}: expected {expected} bytes, received {actual}"
    )]
    PayloadMismatch {
        endpoint: String,
        expected: u64,
        actual: u64,
    },
    #[error("invalid transport base URL: {0}")]
    InvalidBaseUrl(String),
    #[error("could not build HTTP client: {0}")]
    ClientBuild(#[source] reqwest::Error),
}
