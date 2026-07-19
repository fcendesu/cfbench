use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use reqwest::header::{
    ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue, ORIGIN, REFERER,
};
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
    referer: HeaderValue,
    origin: HeaderValue,
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
        let mut base_url = Url::parse(base_url.as_ref())
            .map_err(|error| TransportError::InvalidBaseUrl(error.to_string()))?;
        base_url
            .set_username("")
            .map_err(|_| TransportError::InvalidRequestContext)?;
        base_url
            .set_password(None)
            .map_err(|_| TransportError::InvalidRequestContext)?;
        let (referer, origin) = request_context(&base_url)?;
        let client = build_measurement_client(&config).map_err(TransportError::ClientBuild)?;
        Ok(Self {
            client,
            base_url,
            referer,
            origin,
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
        let (request, endpoint) = self.download_request(bytes, during)?;

        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + self.request_timeout;
        let response = self
            .send_headers(request, &endpoint, deadline, cancellation)
            .await?;
        let headers_received = started.elapsed();
        let (mut response, server_time, http_version, ip_family) =
            validate_response(response, &endpoint)?;

        let mut payload_bytes = 0_u64;
        loop {
            let chunk = match self
                .next_chunk(
                    &mut response,
                    &endpoint,
                    deadline,
                    payload_bytes,
                    cancellation,
                )
                .await
            {
                Ok(chunk) => chunk,
                Err(error) => return Err(error),
            };
            let Some(chunk) = chunk else { break };
            payload_bytes = payload_bytes.checked_add(chunk.len() as u64).ok_or(
                TransportError::DownloadPayloadMismatch {
                    endpoint: endpoint.clone(),
                    expected: bytes,
                    actual: u64::MAX,
                },
            )?;
        }
        let total = started.elapsed();

        if payload_bytes != bytes {
            return Err(TransportError::DownloadPayloadMismatch {
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
        )
        .with_endpoint(endpoint);
        Ok(with_ip_family(observation, ip_family))
    }

    pub async fn upload(
        &self,
        bytes: u64,
        cancellation: &CancellationToken,
    ) -> Result<TimingObservation, TransportError> {
        let url = self.endpoint("__up")?;
        let endpoint = redacted_endpoint(&url);
        let (stream, content_length, yielded_bytes) = stream_upload(bytes);
        let request = self
            .client
            .post(url)
            .header(CONTENT_TYPE, "text/plain;charset=UTF-8")
            .header(CONTENT_LENGTH, content_length)
            .header(REFERER, self.referer.clone())
            .header(ORIGIN, self.origin.clone())
            .body(reqwest::Body::wrap_stream(stream));

        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + self.request_timeout;
        let response = self
            .send_headers(request, &endpoint, deadline, cancellation)
            .await
            .map_err(|error| error.with_payload(yielded_bytes.load(Ordering::Relaxed)))?;
        let headers_received = started.elapsed();
        let (mut response, server_time, http_version, ip_family) =
            validate_response(response, &endpoint)
                .map_err(|error| error.with_payload(yielded_bytes.load(Ordering::Relaxed)))?;
        while self
            .next_chunk(
                &mut response,
                &endpoint,
                deadline,
                yielded_bytes.load(Ordering::Relaxed),
                cancellation,
            )
            .await
            .map_err(|error| error.with_payload(yielded_bytes.load(Ordering::Relaxed)))?
            .is_some()
        {}

        let payload_bytes = yielded_bytes.load(Ordering::Relaxed);
        if payload_bytes != bytes {
            return Err(TransportError::UploadPayloadMismatch {
                endpoint,
                expected: bytes,
                actual: payload_bytes,
            });
        }

        let observation = TimingObservation::new(
            headers_received,
            started.elapsed(),
            server_time,
            payload_bytes,
            http_version,
        )
        .with_endpoint(endpoint);
        Ok(with_ip_family(observation, ip_family))
    }

    fn endpoint(&self, path: &str) -> Result<Url, TransportError> {
        self.base_url
            .join(path)
            .map_err(|error| TransportError::InvalidBaseUrl(error.to_string()))
    }

    fn download_request(
        &self,
        bytes: u64,
        during: Option<&str>,
    ) -> Result<(reqwest::RequestBuilder, String), TransportError> {
        let mut url = self.endpoint("__down")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("bytes", &bytes.to_string());
            if let Some(during) = during {
                query.append_pair("during", during);
            }
        }
        let endpoint = redacted_endpoint(&url);
        let request = self.client.get(url).header(REFERER, self.referer.clone());
        Ok((request, endpoint))
    }

    async fn send_headers(
        &self,
        request: reqwest::RequestBuilder,
        endpoint: &str,
        deadline: tokio::time::Instant,
        cancellation: &CancellationToken,
    ) -> Result<Response, TransportError> {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(TransportError::Cancelled { payload_bytes: 0 }),
            result = tokio::time::timeout_at(deadline, request.send()) => {
                match result {
                    Err(_) => Err(TransportError::HeaderTimeout {
                        endpoint: endpoint.to_owned(),
                        payload_bytes: 0,
                    }),
                    Ok(Err(source)) => Err(TransportError::Request {
                        endpoint: endpoint.to_owned(),
                        payload_bytes: 0,
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
        deadline: tokio::time::Instant,
        payload_bytes: u64,
        cancellation: &CancellationToken,
    ) -> Result<Option<bytes::Bytes>, TransportError> {
        poll_body_chunk(
            response.chunk(),
            deadline,
            endpoint,
            payload_bytes,
            cancellation,
        )
        .await
    }
}

fn build_measurement_client(config: &RunConfig) -> Result<Client, reqwest::Error> {
    let mut default_headers = HeaderMap::new();
    default_headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));

    let mut builder = Client::builder()
        .use_rustls_tls()
        .redirect(redirect::Policy::none())
        .retry(reqwest::retry::never())
        .user_agent(concat!("cfbench/", env!("CARGO_PKG_VERSION")))
        .default_headers(default_headers)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd();

    builder = configure_ip_mode(builder, config.ip_mode);

    builder.build()
}

fn configure_ip_mode(builder: reqwest::ClientBuilder, ip_mode: IpMode) -> reqwest::ClientBuilder {
    match ip_mode {
        IpMode::Auto => builder,
        IpMode::V4Only => builder
            .no_proxy()
            .local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        IpMode::V6Only => builder
            .no_proxy()
            .local_address(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
    }
}

async fn poll_body_chunk<F>(
    body_chunk: F,
    deadline: tokio::time::Instant,
    endpoint: &str,
    payload_bytes: u64,
    cancellation: &CancellationToken,
) -> Result<Option<bytes::Bytes>, TransportError>
where
    F: Future<Output = reqwest::Result<Option<bytes::Bytes>>>,
{
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(TransportError::Cancelled { payload_bytes }),
        result = tokio::time::timeout_at(deadline, body_chunk) => {
            match result {
                Err(_) => Err(TransportError::BodyTimeout {
                    endpoint: endpoint.to_owned(),
                    payload_bytes,
                }),
                Ok(Err(source)) => Err(TransportError::BodyStream {
                    endpoint: endpoint.to_owned(),
                    payload_bytes,
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
            payload_bytes: 0,
        });
    }

    let combined_server_timing = response
        .headers()
        .get_all(SERVER_TIMING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(",");
    let duration = server_duration(
        (!combined_server_timing.is_empty()).then_some(combined_server_timing.as_str()),
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

fn request_context(base_url: &Url) -> Result<(HeaderValue, HeaderValue), TransportError> {
    let mut referer = base_url.clone();
    referer
        .set_username("")
        .map_err(|_| TransportError::InvalidRequestContext)?;
    referer
        .set_password(None)
        .map_err(|_| TransportError::InvalidRequestContext)?;
    referer.set_path("/");
    referer.set_query(None);
    referer.set_fragment(None);
    let origin = referer.origin().ascii_serialization();
    Ok((
        HeaderValue::from_str(referer.as_str())
            .map_err(|_| TransportError::InvalidRequestContext)?,
        HeaderValue::from_str(&origin).map_err(|_| TransportError::InvalidRequestContext)?,
    ))
}

#[cfg(test)]
mod tests {
    use std::future;
    use std::sync::Arc;

    use tokio::sync::Notify;

    use super::*;

    async fn probe_download_headers(
        transport: &ReqwestTransport,
        bytes: u64,
    ) -> Result<reqwest::StatusCode, TransportError> {
        let (request, endpoint) = transport.download_request(bytes, None)?;
        let cancellation = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + transport.request_timeout;
        let response = transport
            .send_headers(request, &endpoint, deadline, &cancellation)
            .await?;
        let status = response.status();
        drop(response);
        Ok(status)
    }

    #[tokio::test]
    #[ignore = "uses the live Cloudflare endpoint"]
    async fn live_large_download_accepts_browser_request_context() {
        let transport =
            ReqwestTransport::new(RunConfig::default()).expect("build live Cloudflare transport");
        let status = probe_download_headers(&transport, 100_000_000)
            .await
            .expect("Cloudflare large download endpoint returns response headers");

        assert!(status.is_success());
    }

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
                tokio::time::Instant::now() + Duration::from_secs(30),
                "https://speed.cloudflare.com/__down",
                0,
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
        assert!(matches!(result, Err(TransportError::Cancelled { .. })));
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

    #[test]
    fn production_client_builder_contains_exactly_one_explicit_no_retry_policy() {
        let source = include_str!("reqwest_transport.rs");
        let builder_body = source
            .split_once("fn build_measurement_client")
            .and_then(|(_, rest)| rest.split_once("\nasync fn poll_body_chunk"))
            .map(|(body, _)| body)
            .expect("locate only the production measurement-client builder body");
        let no_retry_call = [".retry(reqwest::", "retry::never())"].concat();

        assert_eq!(builder_body.matches(&no_retry_call).count(), 1);
    }

    #[test]
    fn forced_family_builder_contains_no_proxy_in_both_strict_arms() {
        let source = include_str!("reqwest_transport.rs");
        let family_body = source
            .split_once("fn configure_ip_mode")
            .and_then(|(_, rest)| rest.split_once("\nasync fn poll_body_chunk"))
            .map(|(body, _)| body)
            .expect("locate only strict-family client construction");
        let no_proxy_call = [".no_", "proxy()"].concat();

        assert_eq!(family_body.matches(&no_proxy_call).count(), 2);
    }

    #[test]
    fn download_request_is_constructed_before_measurement_timing_starts() {
        let source = include_str!("reqwest_transport.rs");
        let download_body = source
            .split_once("pub async fn download(")
            .and_then(|(_, rest)| rest.split_once("\n    pub async fn upload("))
            .map(|(body, _)| body)
            .expect("locate download implementation");
        let request_builder = download_body
            .find("let (request, endpoint) = self.download_request(bytes, during)?;")
            .expect("shared download request construction");
        let started = download_body
            .find("let started = Instant::now()")
            .expect("download timing start");

        assert!(request_builder < started);
    }
}
