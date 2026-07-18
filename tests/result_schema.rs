use cfbench::measurement::{TimingObservation, bandwidth_point};
use cfbench::plan::Direction;
use cfbench::results::RunResult;

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
