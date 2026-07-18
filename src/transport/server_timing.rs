use std::time::Duration;

const FALLBACK_SERVER_DURATION: Duration = Duration::from_millis(10);

/// Extracts Cloudflare's server processing time from a `Server-Timing` value.
pub fn server_duration(header: Option<&str>) -> Duration {
    header
        .into_iter()
        .flat_map(|value| value.split(','))
        .filter_map(parse_metric)
        .next()
        .unwrap_or(FALLBACK_SERVER_DURATION)
}

fn parse_metric(metric: &str) -> Option<Duration> {
    let mut fields = metric.split(';').map(str::trim);
    if fields.next()? != "cfRequestDuration" {
        return None;
    }

    fields.find_map(|field| {
        let value = field.strip_prefix("dur=")?.trim().parse::<f64>().ok()?;
        if !value.is_finite() || value.is_sign_negative() {
            return None;
        }
        Duration::try_from_secs_f64(value / 1_000.0).ok()
    })
}
