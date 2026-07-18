use thiserror::Error;

/// Failures at a native HTTP measurement boundary.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("measurement was cancelled")]
    Cancelled,
    #[error("timed out waiting for response headers")]
    HeaderTimeout,
    #[error("timed out while reading the response body")]
    BodyTimeout,
    #[error("HTTP request failed: {0}")]
    Request(#[source] reqwest::Error),
    #[error("endpoint returned HTTP status {0}")]
    HttpStatus(u16),
    #[error("response body stream failed: {0}")]
    BodyStream(#[source] reqwest::Error),
    #[error("download payload mismatch: expected {expected} bytes, received {actual}")]
    PayloadMismatch { expected: u64, actual: u64 },
    #[error("invalid transport base URL: {0}")]
    InvalidBaseUrl(String),
    #[error("could not build HTTP client: {0}")]
    ClientBuild(#[source] reqwest::Error),
}
