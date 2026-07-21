mod support;

use cfbench::cancellation::CancellationToken;
use cfbench::config::RunConfig;
use cfbench::plan::{
    CLOUDFLARE_SPEEDTEST_COMMIT, CLOUDFLARE_SPEEDTEST_VERSION, MeasurementPlan, MeasurementStep,
};
use cfbench::results::MetadataStatus;
use cfbench::runner::Runner;
use cfbench::transport::ReqwestTransport;

use support::FixtureServer;

#[tokio::test]
async fn compact_fixture_run_produces_reducible_results() {
    let server = FixtureServer::cloudflare_compatible().await;
    let transport = ReqwestTransport::with_base_url(RunConfig::default(), server.url())
        .expect("build fixture transport");
    let outcome = Runner::new(transport, compact_plan())
        .with_loaded_latency(false)
        .with_metadata(true)
        .run(&CancellationToken::new())
        .await;

    assert!(outcome.error.is_none());
    assert!(outcome.result.summary.unloaded_latency_ms.is_some());
    assert!(outcome.result.summary.download_bps.is_some());
    assert!(outcome.result.summary.upload_bps.is_some());
    assert_eq!(outcome.result.raw.latency.len(), 2);
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(outcome.result.raw.upload.len(), 1);
    assert_eq!(
        outcome.result.target.metadata_status,
        MetadataStatus::Available
    );
    let metadata = outcome
        .result
        .target
        .metadata
        .as_ref()
        .expect("fixture metadata is retained");
    assert_eq!(metadata.public_ip.as_deref(), Some("192.0.2.1"));
    assert_eq!(metadata.edge.colo.as_deref(), Some("TEST"));
    assert_eq!(outcome.result.usage.download_payload_bytes, 1_024);
    assert_eq!(outcome.result.usage.upload_payload_bytes, 1_024);
    assert_eq!(server.unexpected_requests(), 0);
    assert_eq!(server.request_count(), 5);
    let requests = server.requests().await;
    assert_eq!(
        requests
            .iter()
            .map(|request| request.path.as_str())
            .collect::<Vec<_>>(),
        [
            "/__down?bytes=0",
            "/__down?bytes=0",
            "/__down?bytes=1024",
            "/__up",
            "/meta",
        ]
    );
    let uploads = server.uploads().await;
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].body_bytes, 1_024);
    assert_eq!(
        uploads[0].content_type.as_deref(),
        Some("text/plain;charset=UTF-8")
    );
    assert_eq!(uploads[0].accept_encoding.as_deref(), Some("identity"));
}

fn compact_plan() -> MeasurementPlan {
    MeasurementPlan {
        upstream_version: CLOUDFLARE_SPEEDTEST_VERSION,
        upstream_commit: CLOUDFLARE_SPEEDTEST_COMMIT,
        steps: vec![
            MeasurementStep::Latency { packets: 2 },
            MeasurementStep::Download {
                bytes: 1_024,
                count: 1,
                bypass_finish: true,
            },
            MeasurementStep::Upload {
                bytes: 1_024,
                count: 1,
                bypass_finish: false,
            },
        ],
    }
}
