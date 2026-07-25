use cfbench::cancellation::CancellationToken;
use cfbench::config::RunConfig;
use cfbench::transport::ReqwestTransport;

const LIVE_TRANSFER_BYTES: u64 = 65_536;

fn live_transport() -> ReqwestTransport {
    ReqwestTransport::new(RunConfig::default()).expect("build live Cloudflare transport")
}

#[tokio::test]
#[ignore = "consumes external network resources"]
async fn live_cloudflare_zero_byte_probe_is_finite() {
    let observation = live_transport()
        .latency(&CancellationToken::new())
        .await
        .expect("Cloudflare latency endpoint responds");

    assert_eq!(observation.payload_bytes, 0);
    assert!(observation.ttfb.as_secs_f64().is_finite());
    assert!(observation.total.as_secs_f64().is_finite());
}

#[tokio::test]
#[ignore = "consumes external network resources"]
async fn live_cloudflare_download_returns_exact_requested_size() {
    let observation = live_transport()
        .download(LIVE_TRANSFER_BYTES, None, &CancellationToken::new())
        .await
        .expect("Cloudflare download endpoint responds");

    assert_eq!(observation.payload_bytes, LIVE_TRANSFER_BYTES);
    assert!(observation.total.as_secs_f64().is_finite());
}

#[tokio::test]
#[ignore = "consumes external network resources"]
async fn live_cloudflare_upload_succeeds_with_finite_timing() {
    let observation = live_transport()
        .upload(LIVE_TRANSFER_BYTES, &CancellationToken::new())
        .await
        .expect("Cloudflare upload endpoint responds");

    assert_eq!(observation.payload_bytes, LIVE_TRANSFER_BYTES);
    assert!(observation.ttfb.as_secs_f64().is_finite());
    assert!(observation.total.as_secs_f64().is_finite());
}

#[tokio::test]
#[ignore = "uses external network metadata"]
async fn live_cloudflare_metadata_has_broad_public_shape() {
    let metadata = live_transport()
        .metadata(&CancellationToken::new())
        .await
        .expect("Cloudflare metadata endpoint responds");

    assert!(
        metadata
            .public_ip
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    );
    assert!(metadata.asn.is_some_and(|asn| asn > 0));
    assert!(metadata.edge.colo.as_deref().is_some_and(|colo| {
        colo.len() == 3 && colo.bytes().all(|byte| byte.is_ascii_uppercase())
    }));
}
