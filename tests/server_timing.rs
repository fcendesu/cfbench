use std::time::Duration;

use cfbench::transport::server_timing::server_duration;

mod support;

use support::upstream_v1_13_0::fixture;

#[test]
fn parses_cloudflare_server_duration() {
    assert_eq!(
        server_duration(Some("cfRequestDuration;dur=15.999794")),
        Duration::from_secs_f64(0.015999794)
    );
    assert_eq!(
        server_duration(Some("edge;desc=x, cfRequestDuration;dur=8.5")),
        Duration::from_secs_f64(0.0085)
    );
    assert_eq!(
        server_duration(Some("processing;dur=42, cfRequestDuration;dur=157.000065")),
        Duration::from_secs_f64(0.157000065)
    );
}

#[test]
fn matches_pinned_v1_13_0_server_timing_cases() {
    for case in fixture().server_timing_cases {
        let actual_ms = server_duration(Some(&case.header)).as_secs_f64() * 1_000.0;
        assert!(
            (actual_ms - case.expected_ms).abs() <= 1e-9,
            "header: {}; expected {} ms, got {actual_ms} ms",
            case.header,
            case.expected_ms,
        );
    }
}

#[test]
fn request_duration_wins_and_speed_phases_are_summed() {
    assert_eq!(
        server_duration(Some("cfSpeedDownload;dur=1.25, cfSpeedEdge;dur=2.75")),
        Duration::from_millis(4)
    );
    assert_eq!(
        server_duration(Some(
            "cfSpeedDownload;dur=1.25, cfRequestDuration;dur=8.5, cfSpeedEdge;dur=2.75"
        )),
        Duration::from_micros(8_500)
    );
}

#[test]
fn unrelated_and_too_small_metrics_fall_back_to_zero() {
    assert_eq!(
        server_duration(Some("processing;dur=99, cache;dur=12")),
        Duration::ZERO
    );
    assert_eq!(
        server_duration(Some("cfRequestDuration;dur=0.01, cfSpeedEdge;dur=0.01")),
        Duration::ZERO
    );
}

#[test]
fn falls_back_for_missing_malformed_and_non_finite_values() {
    let fallback = Duration::ZERO;
    assert_eq!(server_duration(None), fallback);
    assert_eq!(server_duration(Some("cfRequestDuration;dur=NaN")), fallback);
    assert_eq!(server_duration(Some("cfRequestDuration;dur=-1")), fallback);
    assert_eq!(server_duration(Some("processing;dur=.")), fallback);
    assert_eq!(
        server_duration(Some(concat!(
            "processing;dur=",
            "999999999999999999999999999999999999999999999999999999999999999999999999",
            "999999999999999999999999999999999999999999999999999999999999999999999999",
            "999999999999999999999999999999999999999999999999999999999999999999999999",
            "999999999999999999999999999999999999999999999999999999999999999999999999",
            "999999999999999999999999999999999999999999999999999999999999999999999999"
        ))),
        fallback
    );
    assert_eq!(server_duration(Some("cfRequestDuration;desc=x")), fallback);
}

#[test]
fn skips_invalid_duplicate_before_valid_cloudflare_metric() {
    assert_eq!(
        server_duration(Some(
            "cfRequestDuration;dur=bogus, cfRequestDuration;dur=2.25"
        )),
        Duration::from_secs_f64(0.00225)
    );
}

#[test]
fn matches_upstream_decimal_prefix_and_combined_header_search() {
    assert_eq!(
        server_duration(Some("cfRequestDuration;dur=1e3")),
        Duration::from_millis(1)
    );
    assert_eq!(
        server_duration(Some(", cfSpeedEdge;dur=6.25")),
        Duration::from_micros(6_250)
    );
}
