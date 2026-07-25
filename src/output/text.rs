use std::borrow::Cow;
use std::fmt::Write;

use crate::results::{EdgeLocation, MetadataStatus, NetworkMetadata, RunResult};

const UNAVAILABLE: &str = "unavailable";

/// Renders a line-oriented summary without terminal control sequences.
pub fn render_text(result: &RunResult) -> String {
    let mut output = String::new();
    let protocol = protocol(result);

    let _ = writeln!(output, "cfbench {}", result.client.version);
    let _ = writeln!(output, "Target: Cloudflare edge");
    let _ = writeln!(output, "Protocol: {protocol}");
    metadata_lines(&mut output, result);
    let measured_at = nonempty(&result.started_at).unwrap_or(UNAVAILABLE);
    let _ = writeln!(output, "Measured at: {measured_at}");
    output.push('\n');
    metric_line(
        &mut output,
        "Idle latency",
        result.summary.unloaded_latency_ms,
        "ms",
    );
    metric_line(
        &mut output,
        "Idle jitter",
        result.summary.unloaded_jitter_ms,
        "ms",
    );
    bandwidth_line(&mut output, "Download", result.summary.download_bps);
    metric_line(
        &mut output,
        "Download latency",
        result.summary.download_loaded_latency_ms,
        "ms",
    );
    metric_line(
        &mut output,
        "Download jitter",
        result.summary.download_loaded_jitter_ms,
        "ms",
    );
    bandwidth_line(&mut output, "Upload", result.summary.upload_bps);
    metric_line(
        &mut output,
        "Upload latency",
        result.summary.upload_loaded_latency_ms,
        "ms",
    );
    metric_line(
        &mut output,
        "Upload jitter",
        result.summary.upload_loaded_jitter_ms,
        "ms",
    );
    let _ = writeln!(output, "Packet loss: {UNAVAILABLE}");
    output.push('\n');
    let _ = writeln!(
        output,
        "Downloaded: {:.1} MB",
        result.usage.download_payload_bytes as f64 / 1_000_000.0
    );
    let _ = writeln!(
        output,
        "Uploaded: {:.1} MB",
        result.usage.upload_payload_bytes as f64 / 1_000_000.0
    );
    let _ = writeln!(
        output,
        "Duration: {:.2} s",
        finite_or_zero(result.usage.duration_ms) / 1_000.0
    );
    output
}

fn metadata_lines(output: &mut String, result: &RunResult) {
    match result.target.metadata_status {
        MetadataStatus::Available => {
            if let Some(metadata) = result.target.metadata.as_ref() {
                available_metadata_lines(output, metadata);
            }
        }
        MetadataStatus::Unavailable => {
            let _ = writeln!(output, "Metadata: {UNAVAILABLE}");
        }
        MetadataStatus::Disabled => {}
    }
}

fn available_metadata_lines(output: &mut String, metadata: &NetworkMetadata) {
    if let Some(edge) = edge_label(&metadata.edge) {
        let edge = escape_metadata_controls(&edge);
        let _ = writeln!(output, "Edge (informational): {edge}");
    }
    if let Some(network) = network_label(metadata) {
        let network = escape_metadata_controls(&network);
        let _ = writeln!(output, "Network: {network}");
    }
    if let Some(public_ip) = metadata.public_ip.as_deref().and_then(nonempty) {
        let public_ip = escape_metadata_controls(public_ip);
        let _ = writeln!(output, "Public IP: {public_ip}");
    }
}

fn escape_metadata_controls(value: &str) -> Cow<'_, str> {
    if !value.chars().any(char::is_control) {
        return Cow::Borrowed(value);
    }

    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
    }
    Cow::Owned(escaped)
}

fn edge_label(edge: &EdgeLocation) -> Option<String> {
    let colo = edge.colo.as_deref().and_then(nonempty);
    let city = edge.city.as_deref().and_then(nonempty);
    let country = edge.country_code.as_deref().and_then(nonempty);
    let locality = join_components(city, country, ", ");

    join_components(colo, locality.as_deref(), " — ")
}

fn network_label(metadata: &NetworkMetadata) -> Option<String> {
    let organization = metadata.as_organization.as_deref().and_then(nonempty);
    let asn = metadata.asn.map(|asn| format!("AS{asn}"));

    match (organization, asn) {
        (Some(organization), Some(asn)) => Some(format!("{organization} ({asn})")),
        (Some(organization), None) => Some(organization.to_owned()),
        (None, Some(asn)) => Some(asn),
        (None, None) => None,
    }
}

fn join_components(left: Option<&str>, right: Option<&str>, separator: &str) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(format!("{left}{separator}{right}")),
        (Some(component), None) | (None, Some(component)) => Some(component.to_owned()),
        (None, None) => None,
    }
}

fn nonempty(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then_some(value)
}

fn protocol(result: &RunResult) -> String {
    match (
        result.target.ip_family.as_deref(),
        result.target.http_version.as_deref(),
    ) {
        (None, None) => UNAVAILABLE.to_owned(),
        (Some(family), None) => display_ip_family(family),
        (None, Some(version)) => display_http_version(version),
        (Some(family), Some(version)) => format!(
            "{} / {}",
            display_ip_family(family),
            display_http_version(version)
        ),
    }
}

fn display_http_version(version: &str) -> String {
    if version.starts_with("HTTP/") {
        version.to_owned()
    } else {
        format!("HTTP/{version}")
    }
}

fn display_ip_family(family: &str) -> String {
    match family {
        "ipv4" => "IPv4".to_owned(),
        "ipv6" => "IPv6".to_owned(),
        other => other.to_owned(),
    }
}

fn metric_line(output: &mut String, label: &str, value: Option<f64>, unit: &str) {
    match value.filter(|value| value.is_finite()) {
        Some(value) => {
            let _ = writeln!(output, "{label}: {value:.2} {unit}");
        }
        None => {
            let _ = writeln!(output, "{label}: {UNAVAILABLE}");
        }
    }
}

fn bandwidth_line(output: &mut String, label: &str, bps: Option<u64>) {
    match bps {
        Some(bps) => {
            let _ = writeln!(output, "{label}: {:.2} Mbps", bps as f64 / 1_000_000.0);
        }
        None => {
            let _ = writeln!(output, "{label}: {UNAVAILABLE}");
        }
    }
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}
