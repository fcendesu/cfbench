mod support;

use std::net::Ipv6Addr;
use std::time::Duration;

use cfbench::config::{IpMode, RunConfig};
use cfbench::error::TransportError;
use cfbench::plan::{MeasurementPlan, MeasurementStep};
use cfbench::progress::{ProgressEvent, ProgressReporter};
use cfbench::results::RpkiReachabilityStatus;
use cfbench::runner::{Runner, RunnerError};
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
async fn download_stream_reports_live_transfer_snapshots() {
    let server = FixtureServer::start(ResponsePlan::Trickle {
        chunks: 3,
        chunk_interval: Duration::from_millis(300),
    })
    .await;
    let runner = Runner::new(
        transport_for(&server, IpMode::V4Only, Duration::from_secs(2)),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Download {
                bytes: 3,
                count: 1,
                bypass_finish: true,
            }],
        },
    )
    .with_loaded_latency(false);
    let (progress, receiver) = ProgressReporter::channel(16);

    let outcome = runner
        .run_with_progress(&CancellationToken::new(), progress)
        .await;
    let events = receiver.into_iter().collect::<Vec<_>>();

    assert!(outcome.error.is_none());
    assert!(events.iter().any(|event| {
        matches!(
            event,
            ProgressEvent::TransferAdvanced {
                transferred_bytes,
                window_bytes,
                window_duration_ms,
                ..
            } if *transferred_bytes > 0
                && *transferred_bytes < 3
                && *window_bytes > 0
                && *window_duration_ms > 0.0
        )
    }));

    let point = &outcome.result.raw.download[0];
    assert_eq!(point.payload_bytes, 3);
    assert_eq!(point.server_time_ms, 0.0);
    assert_eq!(point.http_version.as_deref(), Some("1.1"));
    assert!(point.duration_ms >= 500.0);
    assert!(point.adjusted_duration_ms >= 500.0);
}

#[tokio::test]
async fn upload_stream_reports_live_transfer_snapshots() {
    let server = FixtureServer::start(ResponsePlan::UploadEcho).await;
    let runner = Runner::new(
        transport_for(&server, IpMode::V4Only, Duration::from_secs(2)),
        MeasurementPlan {
            upstream_version: "test",
            upstream_commit: "test",
            steps: vec![MeasurementStep::Upload {
                bytes: 150_000,
                count: 1,
                bypass_finish: true,
            }],
        },
    )
    .with_loaded_latency(false);
    let (progress, receiver) = ProgressReporter::channel(16);

    let outcome = runner
        .run_with_progress(&CancellationToken::new(), progress)
        .await;
    let events = receiver.into_iter().collect::<Vec<_>>();

    assert!(outcome.error.is_none());
    assert!(events.iter().any(|event| {
        matches!(
            event,
            ProgressEvent::TransferAdvanced {
                direction: cfbench::plan::Direction::Upload,
                requested_bytes: 150_000,
                transferred_bytes: 150_000,
                current: 1,
                total: 1,
                ..
            }
        )
    }));
}

#[tokio::test]
async fn rpki_request_obeys_the_selected_ip_family() {
    let server = FixtureServer::start(ResponsePlan::Exact {
        status: 200,
        body_bytes: 0,
        chunk_bytes: 1,
        server_timing: None,
    })
    .await;

    let reachable = transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
        .rpki_reachability(&CancellationToken::new())
        .await
        .unwrap();
    let unreachable = transport_for(&server, IpMode::V6Only, Duration::from_secs(1))
        .rpki_reachability(&CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(reachable.status, RpkiReachabilityStatus::Reachable);
    assert_eq!(unreachable.status, RpkiReachabilityStatus::Unreachable);
    assert_eq!(server.request_count(), 1);
    let requests = server.requests().await;
    assert_eq!(requests[0].path, "/rpki-invalid");
    assert_eq!(requests[0].accept_encoding.as_deref(), Some("identity"));
    assert!(requests[0].authorization.is_none());
}

#[tokio::test(start_paused = true)]
async fn rpki_timeout_is_informationally_unreachable() {
    let server = FixtureServer::start(ResponsePlan::DelayHeaders).await;

    let result = transport_for(&server, IpMode::V4Only, Duration::from_secs(300))
        .rpki_reachability(&CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(result.status, RpkiReachabilityStatus::Unreachable);
    assert_eq!(result.host.as_deref(), Some("127.0.0.1"));
    assert!(
        result
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("timed out"))
    );
}

#[tokio::test]
async fn rpki_http_failure_is_an_informational_error() {
    let server = FixtureServer::start(ResponsePlan::Exact {
        status: 503,
        body_bytes: 0,
        chunk_bytes: 1,
        server_timing: None,
    })
    .await;

    let result = transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
        .rpki_reachability(&CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(result.status, RpkiReachabilityStatus::Error);
    assert!(
        result
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("HTTP status 503"))
    );
}

#[tokio::test]
async fn cancellation_preempts_rpki_waiting_for_headers() {
    let server = FixtureServer::start(ResponsePlan::DelayHeaders).await;
    let token = CancellationToken::new();
    let transport = transport_for(&server, IpMode::V4Only, Duration::from_secs(30));
    let task_token = token.clone();
    let task = tokio::spawn(async move { transport.rpki_reachability(&task_token).await });
    server.wait_until_stalled().await;
    token.cancel();

    let result = tokio::time::timeout(Duration::from_millis(250), task)
        .await
        .expect("RPKI cancellation completes promptly")
        .expect("transport task joins");
    assert!(matches!(
        result,
        Err(TransportError::Cancelled { payload_bytes: 0 })
    ));
}

#[tokio::test]
async fn measurement_requests_send_safe_same_origin_context() {
    let fixture = FixtureServer::cloudflare_compatible().await;
    let transport =
        ReqwestTransport::with_base_url(RunConfig::default(), fixture.url_with_test_context())
            .unwrap();
    let cancel = CancellationToken::new();

    transport.download(100_000, None, &cancel).await.unwrap();
    transport.upload(100_000, &cancel).await.unwrap();

    let requests = fixture.requests().await;
    let expected_referer = format!("{}/", fixture.url());
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/__down?bytes=100000");
    assert_eq!(
        requests[0].referer.as_deref(),
        Some(expected_referer.as_str())
    );
    assert_eq!(requests[0].origin, None);
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/__up");
    assert_eq!(requests[1].referer, requests[0].referer);
    assert_eq!(requests[1].origin.as_deref(), Some(fixture.url().as_str()));
    assert!(
        requests
            .iter()
            .all(|request| request.authorization.is_none())
    );
    assert!(
        !requests
            .iter()
            .any(|request| format!("{request:?}").contains("secret"))
    );
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
        TransportError::DownloadPayloadMismatch {
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
async fn combined_server_timing_headers_use_cloudflare_metrics() {
    let server = FixtureServer::start(ResponsePlan::MultiServerTiming).await;

    let observation = transport_for(&server, IpMode::V4Only, Duration::from_secs(1))
        .latency(&CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(observation.server_time, Duration::from_millis(4));
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
    assert!((1..3).contains(&error.payload_bytes()));
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
async fn early_success_response_rejects_partially_yielded_upload() {
    let server = FixtureServer::start(ResponsePlan::EarlyUploadSuccess).await;
    let expected = 50_000_000;

    let error = transport_for(&server, IpMode::V4Only, Duration::from_secs(2))
        .upload(expected, &CancellationToken::new())
        .await
        .unwrap_err();

    let TransportError::UploadPayloadMismatch {
        endpoint,
        expected: mismatch_expected,
        actual,
    } = &error
    else {
        panic!("expected upload payload mismatch, got {error:?}");
    };
    assert_eq!(*mismatch_expected, expected);
    assert!(*actual < expected);
    assert_eq!(error.payload_bytes(), *actual);
    assert_eq!(endpoint, &format!("{}/__up", server.url()));
    assert!(!endpoint.contains('?'));
}

#[tokio::test]
async fn runner_keeps_partial_early_upload_usage_without_a_point() {
    let server = FixtureServer::start(ResponsePlan::EarlyUploadSuccess).await;
    let expected = 50_000_000;
    let transport = transport_for(&server, IpMode::V4Only, Duration::from_secs(2));
    let plan = MeasurementPlan {
        upstream_version: "test",
        upstream_commit: "test",
        steps: vec![MeasurementStep::Upload {
            bytes: expected,
            count: 1,
            bypass_finish: true,
        }],
    };

    let outcome = Runner::new(transport, plan)
        .with_loaded_latency(false)
        .run(&CancellationToken::new())
        .await;
    let Some(RunnerError::Transport { source, .. }) = outcome.error.as_ref() else {
        panic!("expected runner transport failure");
    };

    assert!(matches!(
        source,
        TransportError::UploadPayloadMismatch { .. }
    ));
    assert!(source.payload_bytes() < expected);
    assert_eq!(
        outcome.result.usage.upload_payload_bytes,
        source.payload_bytes()
    );
    assert!(outcome.result.raw.upload.is_empty());
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
    server.wait_until_first_body_chunk().await;
    cancellation.cancel();

    let error = task.await.unwrap().unwrap_err();
    assert!(matches!(error, TransportError::Cancelled { .. }));
    assert!(error.payload_bytes() < 3);
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
