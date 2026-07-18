use cfbench::measurement::{TimingObservation, bandwidth_point, latency_point};
use cfbench::plan::Direction;
use cfbench::results::{BandwidthPoint, LatencyPoint, RawResults, reduce};

fn bandwidth(direction: Direction, duration_ms: f64, bps: u64) -> BandwidthPoint {
    BandwidthPoint {
        direction,
        requested_bytes: 1_000_000,
        payload_bytes: 1_000_000,
        duration_ms,
        adjusted_duration_ms: duration_ms,
        ping_ms: 5.0,
        server_time_ms: 10.0,
        bps,
        http_version: Some("HTTP/2".to_owned()),
    }
}

fn latency(ping_ms: f64) -> LatencyPoint {
    LatencyPoint {
        ping_ms,
        ttfb_ms: ping_ms + 10.0,
        server_time_ms: 10.0,
        http_version: Some("HTTP/2".to_owned()),
    }
}

#[test]
fn bandwidth_applies_server_adjustment_and_header_estimate() {
    let observation = TimingObservation::from_millis(30.0, 210.0, 10.0, 1_000_000, "HTTP/2");
    let point = bandwidth_point(Direction::Download, 1_000_000, observation).unwrap();

    assert_eq!(point.adjusted_duration_ms, 200.0);
    assert_eq!(point.bps, 40_200_000);
    assert_eq!(point.payload_bytes, 1_000_000);
}

#[test]
fn latency_subtracts_server_time_and_clamps_at_zero() {
    let observation = TimingObservation::from_millis(8.0, 9.0, 10.0, 0, "HTTP/1.1");
    let point = latency_point(observation).unwrap();

    assert_eq!(point.ping_ms, 0.0);
    assert_eq!(point.ttfb_ms, 8.0);
}

#[test]
fn reducer_filters_short_bandwidth_points() {
    let raw = RawResults {
        download: vec![
            bandwidth(Direction::Download, 9.99, 900_000_000),
            bandwidth(Direction::Download, 10.0, 100_000_000),
            bandwidth(Direction::Download, 20.0, 200_000_000),
        ],
        ..RawResults::default()
    };

    assert_eq!(reduce(&raw).download_bps, Some(190_000_000));
}

#[test]
fn reducer_uses_later_unloaded_phase_only() {
    let raw = RawResults {
        initial_latency: vec![latency(500.0)],
        latency: vec![latency(10.0), latency(20.0)],
        ..RawResults::default()
    };

    let summary = reduce(&raw);
    assert_eq!(summary.unloaded_latency_ms, Some(15.0));
    assert_eq!(summary.unloaded_jitter_ms, Some(10.0));
}

#[test]
fn reducer_keeps_loaded_directions_separate_and_latest_twenty() {
    let raw = RawResults {
        download_loaded_latency: (1..=21).map(|value| latency(value as f64)).collect(),
        upload_loaded_latency: vec![latency(100.0), latency(120.0)],
        ..RawResults::default()
    };

    let summary = reduce(&raw);
    assert_eq!(summary.download_loaded_latency_ms, Some(11.5));
    assert_eq!(summary.download_loaded_jitter_ms, Some(1.0));
    assert_eq!(summary.upload_loaded_latency_ms, Some(110.0));
    assert_eq!(summary.upload_loaded_jitter_ms, Some(20.0));
}

#[test]
fn invalid_points_do_not_escape_as_non_finite_summaries() {
    let raw = RawResults {
        latency: vec![latency(f64::NAN)],
        download: vec![bandwidth(Direction::Download, f64::INFINITY, u64::MAX)],
        ..RawResults::default()
    };

    let summary = reduce(&raw);
    assert_eq!(summary.unloaded_latency_ms, None);
    assert_eq!(summary.download_bps, None);
}

#[test]
fn conversion_rejects_non_finite_millisecond_inputs() {
    let observation = TimingObservation::from_millis(f64::NAN, 100.0, 10.0, 1_000, "HTTP/2");

    assert!(latency_point(observation).is_err());
}
