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
    Cancelled { payload_bytes: u64 },
    #[error("timed out waiting for response headers from endpoint {endpoint}")]
    HeaderTimeout {
        endpoint: String,
        payload_bytes: u64,
    },
    #[error("timed out while reading the response body from endpoint {endpoint}")]
    BodyTimeout {
        endpoint: String,
        payload_bytes: u64,
    },
    #[error("HTTP request failed for endpoint {endpoint}: {source}")]
    Request {
        endpoint: String,
        payload_bytes: u64,
        #[source]
        source: reqwest::Error,
    },
    #[error("endpoint {endpoint} returned HTTP status {status}")]
    HttpStatus {
        endpoint: String,
        status: u16,
        payload_bytes: u64,
    },
    #[error("response body stream failed for endpoint {endpoint}: {source}")]
    BodyStream {
        endpoint: String,
        payload_bytes: u64,
        #[source]
        source: reqwest::Error,
    },
    #[error(
        "download payload mismatch from endpoint {endpoint}: expected {expected} bytes, received {actual}"
    )]
    DownloadPayloadMismatch {
        endpoint: String,
        expected: u64,
        actual: u64,
    },
    #[error(
        "upload payload mismatch for endpoint {endpoint}: expected {expected} bytes, yielded {actual}"
    )]
    UploadPayloadMismatch {
        endpoint: String,
        expected: u64,
        actual: u64,
    },
    #[error("invalid transport base URL: {0}")]
    InvalidBaseUrl(String),
    #[error("could not build HTTP client: {0}")]
    ClientBuild(#[source] reqwest::Error),
}

impl TransportError {
    pub fn payload_bytes(&self) -> u64 {
        match self {
            Self::Cancelled { payload_bytes }
            | Self::HeaderTimeout { payload_bytes, .. }
            | Self::BodyTimeout { payload_bytes, .. }
            | Self::Request { payload_bytes, .. }
            | Self::HttpStatus { payload_bytes, .. }
            | Self::BodyStream { payload_bytes, .. } => *payload_bytes,
            Self::DownloadPayloadMismatch { actual, .. }
            | Self::UploadPayloadMismatch { actual, .. } => *actual,
            Self::InvalidBaseUrl(_) | Self::ClientBuild(_) => 0,
        }
    }

    pub(crate) fn with_payload(self, payload_bytes: u64) -> Self {
        match self {
            Self::Cancelled { .. } => Self::Cancelled { payload_bytes },
            Self::HeaderTimeout { endpoint, .. } => Self::HeaderTimeout {
                endpoint,
                payload_bytes,
            },
            Self::BodyTimeout { endpoint, .. } => Self::BodyTimeout {
                endpoint,
                payload_bytes,
            },
            Self::Request {
                endpoint, source, ..
            } => Self::Request {
                endpoint,
                payload_bytes,
                source,
            },
            Self::HttpStatus {
                endpoint, status, ..
            } => Self::HttpStatus {
                endpoint,
                status,
                payload_bytes,
            },
            Self::BodyStream {
                endpoint, source, ..
            } => Self::BodyStream {
                endpoint,
                payload_bytes,
                source,
            },
            error => error,
        }
    }
}
