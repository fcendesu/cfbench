mod support;

use cfbench::compatibility::{SPEEDTEST_COMMIT, SPEEDTEST_VERSION};
use cfbench::statistics::{jitter, percentile};

use support::upstream_v1_12_1::fixture;

#[test]
fn pinned_reduction_vectors_match_cfbench_statistics() {
    let fixture = fixture();
    let reductions = fixture.reductions;

    assert_eq!(
        percentile(
            &reductions.latency_points_ms,
            fixture.constants.latency_percentile,
        ),
        Some(reductions.latency_p50_ms),
    );
    assert_eq!(
        jitter(&reductions.latency_points_ms),
        Some(reductions.latency_jitter_ms),
    );
    assert_eq!(
        percentile(
            &reductions.bandwidth_points_bps,
            fixture.constants.bandwidth_percentile,
        ),
        Some(reductions.bandwidth_p90_bps),
    );
}

#[test]
fn pinned_constants_record_the_v1_12_1_runtime_contract() {
    let constants = fixture().constants;

    assert_eq!(constants.estimated_server_time_ms, 0.0);
    assert_eq!(constants.server_time_min_duration_ms, 0.01);
    assert_eq!(constants.transfer_overhead_factor, 1.005);
    assert_eq!(constants.bandwidth_min_request_duration_ms, 10.0);
    assert_eq!(constants.loaded_request_min_duration_ms, 250.0);
    assert_eq!(constants.loaded_latency_throttle_ms, 400);
    assert_eq!(constants.loaded_latency_max_points, 20);
    assert_eq!(constants.bandwidth_finish_request_duration_ms, 1000.0);
}

#[test]
fn fixture_records_a_reviewable_upstream_source() {
    let fixture = fixture();

    assert_eq!(
        fixture.source,
        "https://github.com/cloudflare/speedtest/tree/v1.12.1"
    );
    assert_eq!(fixture.upstream_version, SPEEDTEST_VERSION);
    assert_eq!(fixture.upstream_commit, SPEEDTEST_COMMIT);
}
