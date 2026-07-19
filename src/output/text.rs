use std::fmt::Write;

use crate::results::RunResult;

const UNAVAILABLE: &str = "unavailable";

/// Renders a line-oriented summary without terminal control sequences.
pub fn render_text(result: &RunResult) -> String {
    let mut output = String::new();
    let protocol = protocol(result);

    let _ = writeln!(output, "cfbench {}", result.client.version);
    let _ = writeln!(output, "Target: Cloudflare edge");
    let _ = writeln!(output, "Protocol: {protocol}");
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
