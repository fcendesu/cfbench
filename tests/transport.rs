mod support;

use std::net::Ipv6Addr;
use std::time::Duration;

use cfbench::config::{IpMode, RunConfig};
use cfbench::error::TransportError;
use cfbench::transport::ReqwestTransport;
use support::{FixtureServer, ResponsePlan};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[test]
fn reqwest_http2_client_capability_is_compiled() {
    reqwest::Client::builder()
        .http2_adaptive_window(true)
        .build()
        .expect("HTTP/2-capable reqwest client");
}

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
    let error = transport_for(&mismatch, IpMode::V4Only, Duration::from_secs(1))
        .download(100_001, None, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        TransportError::PayloadMismatch {
            expected: 100_001,
            actual: 100_000,
            ..
        }
    ));
    let message = error.to_string();
    assert!(message.contains(&format!("{}/__down", mismatch.url())));
    assert!(!message.contains("bytes="));
}

#[tokio::test]
async fn later_server_timing_field_is_used_when_first_has_no_duration() {
    let server = FixtureServer::start(ResponsePlan::MultiServerTiming).await;

    let observation = transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
        .latency(&CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(observation.server_time, Duration::from_micros(2_500));
}

#[tokio::test]
async fn truncated_http_body_is_a_body_stream_failure() {
    let server = FixtureServer::start(ResponsePlan::DeclaredLength {
        declared_bytes: 100_001,
        body_bytes: 100_000,
    })
    .await;

    let error = transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
        .download(100_001, None, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(&error, TransportError::BodyStream { .. }));
    assert_eq!(error.payload_bytes(), 100_000);
    let message = error.to_string();
    assert!(message.contains(&format!("{}/__down", server.url())));
    assert!(!message.contains("bytes="));
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
        Err(TransportError::HttpStatus { status: 503, .. })
    ));
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn distinguishes_header_and_body_timeouts() {
    let headers = FixtureServer::start(ResponsePlan::DelayHeaders).await;
    assert!(matches!(
        transport_for(&headers, IpMode::V4Only, Duration::from_millis(30))
            .latency(&CancellationToken::new())
            .await,
        Err(TransportError::HeaderTimeout { .. })
    ));

    let body = FixtureServer::start(ResponsePlan::StallBody).await;
    assert!(matches!(
        transport_for(&body, IpMode::V4Only, Duration::from_millis(30))
            .download(1, None, &CancellationToken::new())
            .await,
        Err(TransportError::BodyTimeout { .. })
    ));
}

#[tokio::test]
async fn request_deadline_does_not_restart_for_each_body_chunk() {
    let server = FixtureServer::start(ResponsePlan::Trickle {
        chunks: 3,
        chunk_interval: Duration::from_millis(100),
    })
    .await;

    let error = transport_for(&server, IpMode::V4Only, Duration::from_millis(150))
        .download(3, None, &CancellationToken::new())
        .await
        .unwrap_err();

    assert!(matches!(error, TransportError::BodyTimeout { .. }));
    assert_eq!(error.payload_bytes(), 1);
}

#[tokio::test]
async fn full_upload_before_stalled_response_accounts_yielded_bytes() {
    let server = FixtureServer::start(ResponsePlan::StallUploadResponse).await;

    let error = transport_for(&server, IpMode::V4Only, Duration::from_millis(50))
        .upload(150_000, &CancellationToken::new())
        .await
        .unwrap_err();

    assert_eq!(error.payload_bytes(), 150_000);
}

#[tokio::test]
async fn cancelled_partial_download_reports_received_bytes() {
    let server = FixtureServer::start(ResponsePlan::Trickle {
        chunks: 3,
        chunk_interval: Duration::from_millis(100),
    })
    .await;
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let transport = transport_for(&server, IpMode::V4Only, Duration::from_secs(2));
    let task = tokio::spawn(async move { transport.download(3, None, &task_cancellation).await });
    tokio::time::sleep(Duration::from_millis(150)).await;
    cancellation.cancel();

    let error = task.await.unwrap().unwrap_err();
    assert!(matches!(error, TransportError::Cancelled { .. }));
    assert_eq!(error.payload_bytes(), 1);
}

#[tokio::test]
#[ignore = "streams 250 MB; run explicitly under a platform memory tool"]
async fn local_250_mb_download_streams_in_bounded_chunks() {
    let server = FixtureServer::start(ResponsePlan::Exact {
        status: 200,
        body_bytes: 250_000_000,
        chunk_bytes: 64 * 1024,
        server_timing: Some("cfRequestDuration;dur=0"),
    })
    .await;

    let observation = transport_for(&server, IpMode::V4Only, Duration::from_secs(30))
        .download(250_000_000, None, &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(observation.payload_bytes, 250_000_000);
}

#[tokio::test]
async fn cancellation_preempts_waiting_for_headers() {
    let server = FixtureServer::start(ResponsePlan::DelayHeaders).await;
    let token = CancellationToken::new();
    let transport = transport_for(&server, IpMode::V4Only, Duration::from_secs(30));
    let task_token = token.clone();
    let task = tokio::spawn(async move { transport.latency(&task_token).await });
    server.wait_until_stalled().await;
    token.cancel();

    let result = tokio::time::timeout(Duration::from_millis(250), task)
        .await
        .expect("header cancellation completes promptly")
        .expect("transport task joins");
    assert!(matches!(result, Err(TransportError::Cancelled { .. })));
}

#[tokio::test]
async fn cancellation_preempts_stalled_download_body() {
    let server = FixtureServer::start(ResponsePlan::StallBody).await;
    let token = CancellationToken::new();
    let transport = transport_for(&server, IpMode::V4Only, Duration::from_secs(30));
    let task_token = token.clone();
    let task = tokio::spawn(async move { transport.download(1, None, &task_token).await });
    server.wait_until_stalled().await;
    token.cancel();

    let result = tokio::time::timeout(Duration::from_millis(250), task)
        .await
        .expect("download cancellation completes promptly")
        .expect("transport task joins");
    assert!(matches!(result, Err(TransportError::Cancelled { .. })));
}

#[tokio::test]
async fn cancellation_preempts_stalled_upload_response_body() {
    let server = FixtureServer::start(ResponsePlan::StallUploadResponse).await;
    let token = CancellationToken::new();
    let transport = transport_for(&server, IpMode::V4Only, Duration::from_secs(30));
    let task_token = token.clone();
    let task = tokio::spawn(async move { transport.upload(150_000, &task_token).await });
    server.wait_until_stalled().await;
    token.cancel();

    let result = tokio::time::timeout(Duration::from_millis(250), task)
        .await
        .expect("upload cancellation completes promptly")
        .expect("transport task joins");
    assert!(matches!(result, Err(TransportError::Cancelled { .. })));
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
        Err(TransportError::Request { .. })
    ));
}

#[tokio::test]
async fn ipv6_only_reaches_ipv6_fixture_and_ipv4_only_cannot_fallback() {
    let Ok(listener) = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await else {
        return;
    };
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let url = format!("http://[{0}]:{1}", address.ip(), address.port());

    ReqwestTransport::with_base_url(config(IpMode::V6Only, Duration::from_secs(1)), &url)
        .unwrap()
        .latency(&CancellationToken::new())
        .await
        .expect("IPv6-only fixture connection");
    server.await.unwrap();

    assert!(matches!(
        ReqwestTransport::with_base_url(config(IpMode::V4Only, Duration::from_millis(100)), &url,)
            .unwrap()
            .latency(&CancellationToken::new())
            .await,
        Err(TransportError::Request { .. })
    ));
}
#[tokio::test]
async fn observation_records_contract_http_version_and_peer_ip_family() {
    let fixture = FixtureServer::start(ResponsePlan::Exact {
        status: 200,
        body_bytes: 0,
        chunk_bytes: 1,
        server_timing: None,
    })
    .await;
    let transport = ReqwestTransport::with_base_url(RunConfig::default(), fixture.url()).unwrap();

    let observation = transport.latency(&CancellationToken::new()).await.unwrap();

    assert_eq!(observation.http_version.as_deref(), Some("1.1"));
    assert_eq!(observation.ip_family.as_deref(), Some("ipv4"));
    assert_eq!(observation.endpoint, format!("{}/__down", fixture.url()));
    assert!(!observation.endpoint.contains('?'));
}

#[tokio::test]
async fn transport_errors_include_redacted_endpoint_context() {
    let headers = FixtureServer::start(ResponsePlan::DelayHeaders).await;
    let error = transport_for(&headers, IpMode::V4Only, Duration::from_millis(20))
        .download(123, Some("download"), &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(&format!("{}/__down", headers.url())));
    assert!(error.contains("response headers"));
    assert!(!error.contains("bytes=123"));
    assert!(!error.contains("during=download"));

    let body = FixtureServer::start(ResponsePlan::StallBody).await;
    let error = transport_for(&body, IpMode::V4Only, Duration::from_millis(20))
        .download(1, None, &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(&format!("{}/__down", body.url())));
    assert!(error.contains("response body"));
    assert!(!error.contains("bytes=1"));

    let status = FixtureServer::start(ResponsePlan::Exact {
        status: 503,
        body_bytes: 0,
        chunk_bytes: 1,
        server_timing: None,
    })
    .await;
    let error = transport_for(&status, IpMode::V4Only, Duration::from_secs(1))
        .latency(&CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(&format!("{}/__down", status.url())));
    assert!(error.contains("HTTP status 503"));
    assert!(!error.contains("bytes=0"));

    let unreachable = transport_for(&status, IpMode::V6Only, Duration::from_millis(50));
    let error = unreachable
        .download(987, Some("secret"), &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(&format!("{}/__down", status.url())));
    assert!(error.contains("HTTP request failed"));
    assert!(!error.contains("bytes=987"));
    assert!(!error.contains("secret"));
}
