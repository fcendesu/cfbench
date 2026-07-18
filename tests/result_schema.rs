use cfbench::measurement::{TimingObservation, bandwidth_point};
use cfbench::plan::Direction;
use cfbench::results::{LatencyPoint, RawResults, RunResult};

fn latency(ping_ms: f64) -> LatencyPoint {
    LatencyPoint {
        ping_ms,
        ttfb_ms: ping_ms + 10.0,
        server_time_ms: 10.0,
        http_version: Some("HTTP/2".to_owned()),
    }
}

#[test]
fn unavailable_packet_loss_is_explicit() {
    let value = serde_json::to_value(RunResult::empty()).unwrap();

    assert_eq!(value["schema_version"], 1);
    assert_eq!(
        value["summary"]["packet_loss_ratio"],
        serde_json::Value::Null
    );
    assert_eq!(value["packet_loss"]["status"], "unavailable");
    assert_eq!(value["packet_loss"]["reason"], "turn_not_implemented");
    assert_eq!(value["packet_loss"]["ratio"], serde_json::Value::Null);
}

#[test]
fn empty_result_uses_stable_arrays_and_null_summary_values() {
    let value = serde_json::to_value(RunResult::empty()).unwrap();

    assert_eq!(value["points"]["latency"], serde_json::json!([]));
    assert_eq!(value["failures"], serde_json::json!([]));
    assert_eq!(value["diagnostics"], serde_json::json!([]));
    assert_eq!(
        value["summary"]["unloaded_latency_ms"],
        serde_json::Value::Null
    );
    assert_eq!(value["summary"]["download_bps"], serde_json::Value::Null);
}

#[test]
fn bandwidth_json_preserves_actual_payload_bytes() {
    let observation = TimingObservation::from_millis(20.0, 110.0, 10.0, 999_983, "HTTP/2");
    let point = bandwidth_point(Direction::Download, 1_000_000, observation).unwrap();
    let value = serde_json::to_value(point).unwrap();

    assert_eq!(value["requested_bytes"], 1_000_000);
    assert_eq!(value["payload_bytes"], 999_983);
    assert_eq!(value["bps"], 80_398_633);
}

#[test]
fn loaded_latency_serialization_retains_only_latest_twenty_points() {
    let raw = RawResults {
        download_loaded_latency: (1..=21).map(|value| latency(value as f64)).collect(),
        ..RawResults::default()
    };

    let value = serde_json::to_value(raw).unwrap();
    let retained = value["download_loaded_latency"]
        .as_array()
        .unwrap()
        .iter()
        .map(|point| point["ping_ms"].as_f64().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(
        retained,
        (2..=21).map(|value| value as f64).collect::<Vec<_>>()
    );
}
