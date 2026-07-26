use std::time::Duration;

const FALLBACK_SERVER_DURATION: Duration = Duration::ZERO;

/// Extracts Cloudflare's server processing time from a `Server-Timing` value.
pub fn server_duration(header: Option<&str>) -> Duration {
    header
        .and_then(first_decimal_duration)
        .unwrap_or(FALLBACK_SERVER_DURATION)
}

fn first_decimal_duration(header: &str) -> Option<Duration> {
    let starts = std::iter::once(0).chain(
        header
            .match_indices(';')
            .map(|(index, separator)| index + separator.len()),
    );

    for start in starts {
        let field = header[start..].trim_start();
        let Some(decimal) = field.strip_prefix("dur=") else {
            continue;
        };
        let captured_bytes = decimal
            .bytes()
            .take_while(|character| character.is_ascii_digit() || *character == b'.')
            .count();
        if captured_bytes == 0 {
            continue;
        }

        let value = decimal[..captured_bytes].parse::<f64>().ok()?;
        if !value.is_finite() || value.is_sign_negative() {
            return None;
        }
        return Duration::try_from_secs_f64(value / 1_000.0).ok();
    }

    None
}
