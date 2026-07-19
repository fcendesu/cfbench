use cfbench::output::{render_json, render_text};
use cfbench::results::RunResult;

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
