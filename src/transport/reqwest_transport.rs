use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Client, Response, Url, redirect};
use tokio_util::sync::CancellationToken;

use crate::config::{IpMode, RunConfig};
use crate::error::TransportError;
use crate::measurement::TimingObservation;

use super::server_timing::server_duration;
use super::upload_body::stream_upload;

const CLOUDFLARE_BASE_URL: &str = "https://speed.cloudflare.com";
const SERVER_TIMING: &str = "server-timing";

/// Reqwest-backed transport shared by every measurement in one run.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: Client,
    base_url: Url,
    request_timeout: Duration,
}

impl ReqwestTransport {
    pub fn new(config: RunConfig) -> Result<Self, TransportError> {
        Self::with_base_url(config, CLOUDFLARE_BASE_URL)
    }

    /// Constructs a transport for a compatible endpoint, primarily local fixtures.
    pub fn with_base_url(
        config: RunConfig,
        base_url: impl AsRef<str>,
    ) -> Result<Self, TransportError> {
        let base_url = Url::parse(base_url.as_ref())
            .map_err(|error| TransportError::InvalidBaseUrl(error.to_string()))?;
        let mut default_headers = HeaderMap::new();
        default_headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));

        let mut builder = Client::builder()
            .use_rustls_tls()
            .redirect(redirect::Policy::none())
            .user_agent(concat!("cfbench/", env!("CARGO_PKG_VERSION")))
            .default_headers(default_headers)
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd();

        builder = match config.ip_mode {
            IpMode::Auto => builder,
            IpMode::V4Only => builder.local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            IpMode::V6Only => builder.local_address(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
        };

        let client = builder.build().map_err(TransportError::ClientBuild)?;
        Ok(Self {
            client,
            base_url,
            request_timeout: config.request_timeout,
        })
    }

    pub async fn latency(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<TimingObservation, TransportError> {
        self.download(0, None, cancellation).await
    }

    pub async fn download(
        &self,
        bytes: u64,
        during: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<TimingObservation, TransportError> {
        let mut url = self.endpoint("__down")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("bytes", &bytes.to_string());
            if let Some(during) = during {
                query.append_pair("during", during);
            }
        }

        let started = Instant::now();
        let response = self
            .send_headers(self.client.get(url), cancellation)
            .await?;
        let headers_received = started.elapsed();
        let (mut response, server_time, http_version) = validate_response(response)?;

        let mut payload_bytes = 0_u64;
        while let Some(chunk) = self.next_chunk(&mut response, cancellation).await? {
            payload_bytes = payload_bytes.checked_add(chunk.len() as u64).ok_or(
                TransportError::PayloadMismatch {
                    expected: bytes,
                    actual: u64::MAX,
                },
            )?;
        }
        let total = started.elapsed();

        if payload_bytes != bytes {
            return Err(TransportError::PayloadMismatch {
                expected: bytes,
                actual: payload_bytes,
            });
        }

        Ok(TimingObservation::new(
            headers_received,
            total,
            server_time,
            payload_bytes,
            Some(http_version),
        ))
    }

    pub async fn upload(
        &self,
        bytes: u64,
        cancellation: &CancellationToken,
    ) -> Result<TimingObservation, TransportError> {
        let url = self.endpoint("__up")?;
        let (stream, content_length) = stream_upload(bytes);
        let request = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "text/plain;charset=UTF-8")
            .header(CONTENT_LENGTH, content_length)
            .body(reqwest::Body::wrap_stream(stream));

        let started = Instant::now();
        let response = self.send_headers(request, cancellation).await?;
        let headers_received = started.elapsed();
        let (mut response, server_time, http_version) = validate_response(response)?;
        while self
            .next_chunk(&mut response, cancellation)
            .await?
            .is_some()
        {}

        Ok(TimingObservation::new(
            headers_received,
            started.elapsed(),
            server_time,
            bytes,
            Some(http_version),
        ))
    }

    fn endpoint(&self, path: &str) -> Result<Url, TransportError> {
        self.base_url
            .join(path)
            .map_err(|error| TransportError::InvalidBaseUrl(error.to_string()))
    }

    async fn send_headers(
        &self,
        request: reqwest::RequestBuilder,
        cancellation: &CancellationToken,
    ) -> Result<Response, TransportError> {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(TransportError::Cancelled),
            result = tokio::time::timeout(self.request_timeout, request.send()) => {
                match result {
                    Err(_) => Err(TransportError::HeaderTimeout),
                    Ok(Err(error)) => Err(TransportError::Request(error)),
                    Ok(Ok(response)) => Ok(response),
                }
            }
        }
    }

    async fn next_chunk(
        &self,
        response: &mut Response,
        cancellation: &CancellationToken,
    ) -> Result<Option<bytes::Bytes>, TransportError> {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(TransportError::Cancelled),
            result = tokio::time::timeout(self.request_timeout, response.chunk()) => {
                match result {
                    Err(_) => Err(TransportError::BodyTimeout),
                    Ok(Err(error)) => Err(TransportError::BodyStream(error)),
                    Ok(Ok(chunk)) => Ok(chunk),
                }
            }
        }
    }
}

fn validate_response(response: Response) -> Result<(Response, Duration, String), TransportError> {
    if !response.status().is_success() {
        return Err(TransportError::HttpStatus(response.status().as_u16()));
    }

    let duration = server_duration(
        response
            .headers()
            .get(SERVER_TIMING)
            .and_then(|value| value.to_str().ok()),
    );
    let version = format!("{:?}", response.version());
    Ok((response, duration, version))
}
