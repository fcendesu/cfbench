use std::time::Duration;

const FALLBACK_SERVER_DURATION: Duration = Duration::ZERO;
const MIN_SERVER_DURATION_MS: f64 = 0.01;

/// Extracts Cloudflare's server processing time from a `Server-Timing` value.
pub fn server_duration(header: Option<&str>) -> Duration {
    header
        .and_then(server_duration_ms)
        .and_then(|milliseconds| Duration::try_from_secs_f64(milliseconds / 1_000.0).ok())
        .unwrap_or(FALLBACK_SERVER_DURATION)
}

fn server_duration_ms(header: &str) -> Option<f64> {
    let mut request_duration = None;
    let mut speed_duration = 0.0;

    for entry in header.split(',').map(str::trim) {
        if request_duration.is_none() {
            request_duration = named_duration(entry, is_request_duration_metric);
        }
        if let Some(duration) = named_duration(entry, is_speed_duration_metric) {
            speed_duration += duration;
        }
    }

    if request_duration.is_some_and(|duration| duration > MIN_SERVER_DURATION_MS) {
        return request_duration;
    }

    (speed_duration.is_finite() && speed_duration > MIN_SERVER_DURATION_MS)
        .then_some(speed_duration)
}

fn named_duration(entry: &str, accepts_name: impl Fn(&str) -> bool) -> Option<f64> {
    let (name, parameters) = entry.split_once(';')?;
    if !accepts_name(name.trim()) {
        return None;
    }

    let parameters = parameters.trim_start();
    if !parameters
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("dur="))
    {
        return None;
    }
    let decimal = &parameters[4..];
    let captured_bytes = decimal
        .bytes()
        .take_while(|character| character.is_ascii_digit() || *character == b'.')
        .count();
    if captured_bytes == 0 {
        return None;
    }

    decimal[..captured_bytes]
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && !value.is_sign_negative())
}

fn is_request_duration_metric(name: &str) -> bool {
    [
        "cfReqDur",
        "cfRequestDur",
        "cfReqDuration",
        "cfRequestDuration",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn is_speed_duration_metric(name: &str) -> bool {
    name.get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("cfSpeed"))
        && name
            .get(7..)
            .is_some_and(|suffix| suffix.bytes().all(|byte| byte.is_ascii_alphabetic()))
}
