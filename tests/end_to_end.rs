mod support;

use cfbench::cancellation::CancellationToken;
use cfbench::config::RunConfig;
use cfbench::plan::{
    CLOUDFLARE_SPEEDTEST_COMMIT, CLOUDFLARE_SPEEDTEST_VERSION, MeasurementPlan, MeasurementStep,
};
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
        .run(&CancellationToken::new())
        .await;

    assert!(outcome.error.is_none());
    assert!(outcome.result.summary.unloaded_latency_ms.is_some());
    assert!(outcome.result.summary.download_bps.is_some());
    assert!(outcome.result.summary.upload_bps.is_some());
    assert_eq!(outcome.result.raw.latency.len(), 2);
    assert_eq!(outcome.result.raw.download.len(), 1);
    assert_eq!(outcome.result.raw.upload.len(), 1);
    assert_eq!(server.unexpected_requests(), 0);
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
