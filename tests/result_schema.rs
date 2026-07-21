use cfbench::measurement::{TimingObservation, bandwidth_point};
use cfbench::plan::Direction;
use cfbench::results::{
    ClientLocation, EdgeLocation, LatencyPoint, MetadataStatus, NetworkMetadata, RawResults,
    RunResult,
};

const FIXED_UNIX_MS: i64 = 1_784_451_779_123;

fn latency(ping_ms: f64) -> LatencyPoint {
    LatencyPoint {
        ping_ms,
        ttfb_ms: ping_ms + 10.0,
        server_time_ms: 10.0,
        http_version: Some("HTTP/2".to_owned()),
        measured_at_unix_ms: FIXED_UNIX_MS,
    }
}

fn metadata_fixture() -> NetworkMetadata {
    NetworkMetadata {
        public_ip: Some("2a02:ff0::1".to_owned()),
        asn: Some(12_735),
        as_organization: Some("TurkNet Iletisim Hizmetleri A.S.".to_owned()),
        client_location: ClientLocation {
            country_code: Some("TR".to_owned()),
            city: Some("Istanbul".to_owned()),
            region: Some("Istanbul".to_owned()),
            postal_code: Some("34096".to_owned()),
            latitude: Some(41.01384),
            longitude: Some(28.94966),
        },
        edge: EdgeLocation {
            colo: Some("IST".to_owned()),
            country_code: Some("TR".to_owned()),
            region: Some("Europe".to_owned()),
            city: Some("Arnavutkoy".to_owned()),
            latitude: Some(41.262222),
            longitude: Some(28.727778),
        },
    }
}

#[test]
fn result_serializes_network_metadata_and_point_timestamps() {
    let mut result = RunResult::empty();
    result.started_at = "2026-07-19T09:02:59.123Z".to_owned();
    result.target.metadata_status = MetadataStatus::Available;
    result.target.metadata = Some(metadata_fixture());
    result.raw.latency.push(latency(21.6));

    let value = serde_json::to_value(result).unwrap();

    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["started_at"], "2026-07-19T09:02:59.123Z");
    assert_eq!(value["target"]["metadata_status"], "available");
    assert_eq!(value["target"]["metadata"]["edge"]["colo"], "IST");
    assert_eq!(value["target"]["metadata"]["asn"], 12_735);
    assert_eq!(
        value["points"]["latency"][0]["measured_at_unix_ms"],
        FIXED_UNIX_MS
    );
}

#[test]
fn unavailable_and_disabled_metadata_have_distinct_statuses_and_null_values() {
    for (status, expected) in [
        (MetadataStatus::Unavailable, "unavailable"),
        (MetadataStatus::Disabled, "disabled"),
    ] {
        let mut result = RunResult::empty();
        result.target.metadata_status = status;
        result.target.metadata = None;

        let value = serde_json::to_value(result).unwrap();

        assert_eq!(value["target"]["metadata_status"], expected);
        assert_eq!(value["target"]["metadata"], serde_json::Value::Null);
    }
}

#[test]
fn metadata_leaves_are_nullable_without_omitting_the_stable_shape() {
    let mut result = RunResult::empty();
    result.target.metadata_status = MetadataStatus::Available;
    result.target.metadata = Some(NetworkMetadata::default());

    let value = serde_json::to_value(result).unwrap();
    let metadata = &value["target"]["metadata"];

    assert_eq!(metadata["public_ip"], serde_json::Value::Null);
    assert_eq!(metadata["asn"], serde_json::Value::Null);
    assert_eq!(
        metadata["client_location"]["latitude"],
        serde_json::Value::Null
    );
    assert_eq!(metadata["edge"]["longitude"], serde_json::Value::Null);
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
    let point =
        bandwidth_point(Direction::Download, 1_000_000, observation, FIXED_UNIX_MS).unwrap();
    let value = serde_json::to_value(point).unwrap();

    assert_eq!(value["requested_bytes"], 1_000_000);
    assert_eq!(value["payload_bytes"], 999_983);
    assert_eq!(value["bps"], 80_398_633);
    assert_eq!(value["measured_at_unix_ms"], FIXED_UNIX_MS);
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
