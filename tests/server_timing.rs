use std::time::Duration;

use cfbench::transport::server_timing::server_duration;

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
        Duration::from_millis(42)
    );
}

#[test]
fn falls_back_for_missing_malformed_and_non_finite_values() {
    let fallback = Duration::from_millis(10);
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
        server_duration(Some(", processing;dur=6.25")),
        Duration::from_micros(6_250)
    );
}
