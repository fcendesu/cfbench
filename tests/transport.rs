mod support;

use std::time::Duration;

use cfbench::config::{IpMode, RunConfig};
use cfbench::error::TransportError;
use cfbench::transport::ReqwestTransport;
use support::{FixtureServer, ResponsePlan};
use tokio_util::sync::CancellationToken;

fn config(ip_mode: IpMode, timeout: Duration) -> RunConfig {
    RunConfig {
        ip_mode,
        request_timeout: timeout,
        ..RunConfig::default()
    }
}

fn transport_for(server: &FixtureServer, ip_mode: IpMode, timeout: Duration) -> ReqwestTransport {
    ReqwestTransport::with_base_url(config(ip_mode, timeout), server.url())
        .expect("fixture transport")
}

#[tokio::test]
async fn download_counts_streamed_bytes_and_rejects_payload_mismatch() {
    let exact = FixtureServer::start(ResponsePlan::Exact {
        status: 200,
        body_bytes: 100_000,
        chunk_bytes: 8_192,
        server_timing: Some("cfRequestDuration;dur=1.5"),
    })
    .await;
    let point = transport_for(&exact, IpMode::V4Only, Duration::from_secs(1))
        .download(100_000, None, &CancellationToken::new())
        .await
        .expect("exact download");
    assert_eq!(point.payload_bytes, 100_000);
    assert_eq!(point.server_time, Duration::from_micros(1_500));

    let mismatch = FixtureServer::start(ResponsePlan::Exact {
        status: 200,
        body_bytes: 100_000,
        chunk_bytes: 8_192,
        server_timing: None,
    })
    .await;
    assert!(matches!(
        transport_for(&mismatch, IpMode::V4Only, Duration::from_secs(1))
            .download(100_001, None, &CancellationToken::new())
            .await,
        Err(TransportError::PayloadMismatch {
            expected: 100_001,
            actual: 100_000
        })
    ));
}

#[tokio::test]
async fn truncated_http_body_is_a_body_stream_failure() {
    let server = FixtureServer::start(ResponsePlan::DeclaredLength {
        declared_bytes: 100_001,
        body_bytes: 100_000,
    })
    .await;

    assert!(matches!(
        transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
            .download(100_001, None, &CancellationToken::new())
            .await,
        Err(TransportError::BodyStream(_))
    ));
}

#[tokio::test]
async fn rejects_error_status_without_retrying() {
    let server = FixtureServer::start(ResponsePlan::Exact {
        status: 503,
        body_bytes: 0,
        chunk_bytes: 1,
        server_timing: None,
    })
    .await;
    assert!(matches!(
        transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
            .latency(&CancellationToken::new())
            .await,
        Err(TransportError::HttpStatus(503))
    ));
}

#[tokio::test]
async fn distinguishes_header_and_body_timeouts() {
    let headers = FixtureServer::start(ResponsePlan::DelayHeaders).await;
    assert!(matches!(
        transport_for(&headers, IpMode::V4Only, Duration::from_millis(30))
            .latency(&CancellationToken::new())
            .await,
        Err(TransportError::HeaderTimeout)
    ));

    let body = FixtureServer::start(ResponsePlan::StallBody).await;
    assert!(matches!(
        transport_for(&body, IpMode::V4Only, Duration::from_millis(30))
            .download(1, None, &CancellationToken::new())
            .await,
        Err(TransportError::BodyTimeout)
    ));
}

#[tokio::test]
async fn cancellation_preempts_waiting_for_headers() {
    let server = FixtureServer::start(ResponsePlan::DelayHeaders).await;
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
            .latency(&token)
            .await,
        Err(TransportError::Cancelled)
    ));
}

#[tokio::test]
async fn upload_streams_exact_bytes_and_required_headers() {
    let server = FixtureServer::start(ResponsePlan::UploadEcho).await;
    let point = transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
        .upload(150_000, &CancellationToken::new())
        .await
        .expect("upload");
    assert_eq!(point.payload_bytes, 150_000);
    let uploads = server.uploads().await;
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].body_bytes, 150_000);
    assert_eq!(
        uploads[0].content_type.as_deref(),
        Some("text/plain;charset=UTF-8")
    );
    assert_eq!(uploads[0].accept_encoding.as_deref(), Some("identity"));
}

#[tokio::test]
async fn ipv4_only_reaches_ipv4_fixture_and_ipv6_only_cannot_fallback() {
    let server = FixtureServer::start(ResponsePlan::Exact {
        status: 200,
        body_bytes: 0,
        chunk_bytes: 1,
        server_timing: None,
    })
    .await;
    transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
        .latency(&CancellationToken::new())
        .await
        .expect("IPv4-only fixture connection");

    assert!(matches!(
        transport_for(&server, IpMode::V6Only, Duration::from_millis(100))
            .latency(&CancellationToken::new())
            .await,
        Err(TransportError::Request(_))
    ));
}
