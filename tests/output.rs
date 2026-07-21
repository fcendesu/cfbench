use cfbench::output::{render_json, render_text};
use cfbench::results::{EdgeLocation, MetadataStatus, NetworkMetadata, RunResult};

const STARTED_AT: &str = "2026-07-19T09:02:59.123Z";

#[test]
fn render_json_is_one_document_with_nulls() {
    let rendered = render_json(&RunResult::empty()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();

    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["summary"]["download_bps"].is_null());
    assert!(parsed["target"]["ip_family"].is_null());
    assert_eq!(rendered.matches('{').count(), rendered.matches('}').count());
    assert!(!rendered.contains("\u{1b}["));
}

#[test]
fn empty_text_result_uses_stable_labels_and_unavailable_values() {
    let rendered = render_text(&RunResult::empty());

    assert_eq!(
        rendered,
        concat!(
            "cfbench 0.1.0\n",
            "Target: Cloudflare edge\n",
            "Protocol: unavailable\n",
            "Metadata: unavailable\n",
            "Measured at: unavailable\n",
            "\n",
            "Idle latency: unavailable\n",
            "Idle jitter: unavailable\n",
            "Download: unavailable\n",
            "Download latency: unavailable\n",
            "Download jitter: unavailable\n",
            "Upload: unavailable\n",
            "Upload latency: unavailable\n",
            "Upload jitter: unavailable\n",
            "Packet loss: unavailable\n",
            "\n",
            "Downloaded: 0.0 MB\n",
            "Uploaded: 0.0 MB\n",
            "Duration: 0.00 s\n",
        )
    );
    assert!(!rendered.contains("\u{1b}["));
}

#[test]
fn metadata_text_renders_complete_values_between_protocol_and_metrics() {
    let mut result = RunResult::empty();
    result.started_at = STARTED_AT.to_owned();
    result.target.metadata_status = MetadataStatus::Available;
    result.target.metadata = Some(NetworkMetadata {
        public_ip: Some("2a02:ff0::1".to_owned()),
        asn: Some(12_735),
        as_organization: Some("TurkNet Iletisim Hizmetleri A.S.".to_owned()),
        edge: EdgeLocation {
            colo: Some("IST".to_owned()),
            city: Some("Arnavutkoy".to_owned()),
            country_code: Some("TR".to_owned()),
            ..EdgeLocation::default()
        },
        ..NetworkMetadata::default()
    });

    let rendered = render_text(&result);

    assert!(rendered.contains(concat!(
        "Protocol: unavailable\n",
        "Edge: IST — Arnavutkoy, TR\n",
        "Network: TurkNet Iletisim Hizmetleri A.S. (AS12735)\n",
        "Public IP: 2a02:ff0::1\n",
        "Measured at: 2026-07-19T09:02:59.123Z\n",
        "\n",
        "Idle latency: unavailable\n",
    )));
    assert!(!rendered.contains("Metadata: unavailable"));
}

#[test]
fn partial_metadata_text_omits_missing_components_without_dangling_punctuation() {
    let cases = [
        (
            EdgeLocation {
                colo: Some("IST".to_owned()),
                country_code: Some("TR".to_owned()),
                ..EdgeLocation::default()
            },
            None,
            Some(12_735),
            "Edge: IST — TR\nNetwork: AS12735\n",
        ),
        (
            EdgeLocation {
                city: Some("Arnavutkoy".to_owned()),
                ..EdgeLocation::default()
            },
            Some("TurkNet"),
            None,
            "Edge: Arnavutkoy\nNetwork: TurkNet\n",
        ),
        (EdgeLocation::default(), None, None, ""),
    ];

    for (edge, organization, asn, expected_lines) in cases {
        let mut result = RunResult::empty();
        result.started_at = STARTED_AT.to_owned();
        result.target.metadata_status = MetadataStatus::Available;
        result.target.metadata = Some(NetworkMetadata {
            asn,
            as_organization: organization.map(ToOwned::to_owned),
            edge,
            ..NetworkMetadata::default()
        });

        let rendered = render_text(&result);

        assert!(rendered.contains(&format!(
            "Protocol: unavailable\n{expected_lines}Measured at: {STARTED_AT}\n"
        )));
        assert!(!rendered.contains("Edge: —"));
        assert!(!rendered.contains("Edge: ,"));
        assert!(!rendered.contains("Network: ()"));
    }
}

#[test]
fn unavailable_and_disabled_metadata_have_distinct_text_output() {
    let mut unavailable = RunResult::empty();
    unavailable.started_at = STARTED_AT.to_owned();
    unavailable.target.metadata_status = MetadataStatus::Unavailable;
    let unavailable_text = render_text(&unavailable);

    assert!(unavailable_text.contains("Metadata: unavailable\n"));
    assert!(unavailable_text.contains(&format!("Measured at: {STARTED_AT}\n")));

    let mut disabled = unavailable;
    disabled.target.metadata_status = MetadataStatus::Disabled;
    disabled.target.metadata = Some(NetworkMetadata {
        public_ip: Some("192.0.2.1".to_owned()),
        asn: Some(64_496),
        as_organization: Some("Example Network".to_owned()),
        edge: EdgeLocation {
            colo: Some("XYZ".to_owned()),
            ..EdgeLocation::default()
        },
        ..NetworkMetadata::default()
    });
    let disabled_text = render_text(&disabled);

    assert!(!disabled_text.contains("Edge:"));
    assert!(!disabled_text.contains("Network:"));
    assert!(!disabled_text.contains("Public IP:"));
    assert!(!disabled_text.contains("Metadata:"));
    assert!(disabled_text.contains(&format!("Measured at: {STARTED_AT}\n")));
}

#[test]
fn text_result_uses_decimal_units_and_protocol_metadata() {
    let mut result = RunResult::empty();
    result.target.ip_family = Some("ipv6".to_owned());
    result.target.http_version = Some("2".to_owned());
    result.summary.unloaded_latency_ms = Some(14.82);
    result.summary.unloaded_jitter_ms = Some(1.74);
    result.summary.download_bps = Some(842_160_000);
    result.summary.upload_bps = Some(47_620_000);
    result.usage.download_payload_bytes = 418_700_000;
    result.usage.upload_payload_bytes = 83_400_000;
    result.usage.duration_ms = 16_420.0;

    let rendered = render_text(&result);

    assert!(rendered.contains("Protocol: IPv6 / HTTP/2"));
    assert!(rendered.contains("Idle latency: 14.82 ms"));
    assert!(rendered.contains("Download: 842.16 Mbps"));
    assert!(rendered.contains("Upload: 47.62 Mbps"));
    assert!(rendered.contains("Downloaded: 418.7 MB"));
    assert!(rendered.contains("Uploaded: 83.4 MB"));
    assert!(rendered.contains("Duration: 16.42 s"));
}
