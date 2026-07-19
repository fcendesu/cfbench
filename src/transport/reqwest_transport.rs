use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Client, Response, Url, Version, redirect};
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
        let endpoint = redacted_endpoint(&url);

        let started = Instant::now();
        let response = self
            .send_headers(self.client.get(url), &endpoint, cancellation)
            .await?;
        let headers_received = started.elapsed();
        let (mut response, server_time, http_version, ip_family) =
            validate_response(response, &endpoint)?;

        let mut payload_bytes = 0_u64;
        while let Some(chunk) = self
            .next_chunk(&mut response, &endpoint, cancellation)
            .await?
        {
            payload_bytes = payload_bytes.checked_add(chunk.len() as u64).ok_or(
                TransportError::PayloadMismatch {
                    endpoint: endpoint.clone(),
                    expected: bytes,
                    actual: u64::MAX,
                },
            )?;
        }
        let total = started.elapsed();

        if payload_bytes != bytes {
            return Err(TransportError::PayloadMismatch {
                endpoint,
                expected: bytes,
                actual: payload_bytes,
            });
        }

        let observation = TimingObservation::new(
            headers_received,
            total,
            server_time,
            payload_bytes,
            http_version,
        );
        Ok(with_ip_family(observation, ip_family))
    }

    pub async fn upload(
        &self,
        bytes: u64,
        cancellation: &CancellationToken,
    ) -> Result<TimingObservation, TransportError> {
        let url = self.endpoint("__up")?;
        let endpoint = redacted_endpoint(&url);
        let (stream, content_length) = stream_upload(bytes);
        let request = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "text/plain;charset=UTF-8")
            .header(CONTENT_LENGTH, content_length)
            .body(reqwest::Body::wrap_stream(stream));

        let started = Instant::now();
        let response = self.send_headers(request, &endpoint, cancellation).await?;
        let headers_received = started.elapsed();
        let (mut response, server_time, http_version, ip_family) =
            validate_response(response, &endpoint)?;
        while self
            .next_chunk(&mut response, &endpoint, cancellation)
            .await?
            .is_some()
        {}

        let observation = TimingObservation::new(
            headers_received,
            started.elapsed(),
            server_time,
            bytes,
            http_version,
        );
        Ok(with_ip_family(observation, ip_family))
    }

    fn endpoint(&self, path: &str) -> Result<Url, TransportError> {
        self.base_url
            .join(path)
            .map_err(|error| TransportError::InvalidBaseUrl(error.to_string()))
    }

    async fn send_headers(
        &self,
        request: reqwest::RequestBuilder,
        endpoint: &str,
        cancellation: &CancellationToken,
    ) -> Result<Response, TransportError> {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(TransportError::Cancelled),
            result = tokio::time::timeout(self.request_timeout, request.send()) => {
                match result {
                    Err(_) => Err(TransportError::HeaderTimeout {
                        endpoint: endpoint.to_owned(),
                    }),
                    Ok(Err(source)) => Err(TransportError::Request {
                        endpoint: endpoint.to_owned(),
                        source: source.without_url(),
                    }),
                    Ok(Ok(response)) => Ok(response),
                }
            }
        }
    }

    async fn next_chunk(
        &self,
        response: &mut Response,
        endpoint: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<bytes::Bytes>, TransportError> {
        poll_body_chunk(
            response.chunk(),
            self.request_timeout,
            endpoint,
            cancellation,
        )
        .await
    }
}

async fn poll_body_chunk<F>(
    body_chunk: F,
    timeout: Duration,
    endpoint: &str,
    cancellation: &CancellationToken,
) -> Result<Option<bytes::Bytes>, TransportError>
where
    F: Future<Output = reqwest::Result<Option<bytes::Bytes>>>,
{
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(TransportError::Cancelled),
        result = tokio::time::timeout(timeout, body_chunk) => {
            match result {
                Err(_) => Err(TransportError::BodyTimeout {
                    endpoint: endpoint.to_owned(),
                }),
                Ok(Err(source)) => Err(TransportError::BodyStream {
                    endpoint: endpoint.to_owned(),
                    source: source.without_url(),
                }),
                Ok(Ok(chunk)) => Ok(chunk),
            }
        }
    }
}

fn validate_response(
    response: Response,
    endpoint: &str,
) -> Result<(Response, Duration, Option<String>, Option<String>), TransportError> {
    if !response.status().is_success() {
        return Err(TransportError::HttpStatus {
            endpoint: endpoint.to_owned(),
            status: response.status().as_u16(),
        });
    }

    let duration = server_duration(
        response
            .headers()
            .get(SERVER_TIMING)
            .and_then(|value| value.to_str().ok()),
    );
    let version = contract_http_version(response.version()).map(ToOwned::to_owned);
    let ip_family = response.remote_addr().map(|address| {
        if address.is_ipv4() {
            "ipv4".to_owned()
        } else {
            "ipv6".to_owned()
        }
    });
    Ok((response, duration, version, ip_family))
}

fn contract_http_version(version: Version) -> Option<&'static str> {
    match version {
        Version::HTTP_09 => Some("0.9"),
        Version::HTTP_10 => Some("1.0"),
        Version::HTTP_11 => Some("1.1"),
        Version::HTTP_2 => Some("2"),
        Version::HTTP_3 => Some("3"),
        _ => None,
    }
}

fn with_ip_family(observation: TimingObservation, ip_family: Option<String>) -> TimingObservation {
    match ip_family {
        Some(ip_family) => observation.with_ip_family(ip_family),
        None => observation,
    }
}

fn redacted_endpoint(url: &Url) -> String {
    let mut endpoint = url.clone();
    let _ = endpoint.set_username("");
    let _ = endpoint.set_password(None);
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    endpoint.to_string()
}

#[cfg(test)]
mod tests {
    use std::future;
    use std::sync::Arc;

    use tokio::sync::Notify;

    use super::*;

    #[tokio::test]
    async fn cancellation_preempts_an_actively_polled_body_future() {
        let entered_body_poll = Arc::new(Notify::new());
        let body_signal = entered_body_poll.clone();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let body_future = async move {
            body_signal.notify_one();
            future::pending::<reqwest::Result<Option<bytes::Bytes>>>().await
        };

        let task = tokio::spawn(async move {
            poll_body_chunk(
                body_future,
                Duration::from_secs(30),
                "https://speed.cloudflare.com/__down",
                &task_cancellation,
            )
            .await
        });
        entered_body_poll.notified().await;
        cancellation.cancel();

        let result = tokio::time::timeout(Duration::from_millis(250), task)
            .await
            .expect("active body cancellation completes promptly")
            .expect("body poll task joins");
        assert!(matches!(result, Err(TransportError::Cancelled)));
    }

    #[test]
    fn endpoint_redaction_removes_credentials_query_and_fragment() {
        let url = Url::parse(
            "https://user:password@speed.cloudflare.com/__down?bytes=250000000&secret=value#x",
        )
        .unwrap();

        let endpoint = redacted_endpoint(&url);

        assert_eq!(endpoint, "https://speed.cloudflare.com/__down");
        assert!(!endpoint.contains("user"));
        assert!(!endpoint.contains("password"));
        assert!(!endpoint.contains("secret"));
    }
}
