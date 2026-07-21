use std::time::{Duration, Instant, SystemTime};

const MAX_RFC3339_DURATION: Duration = Duration::new(253_402_300_799, 999_999_999);

/// One wall-clock/monotonic anchor shared by every point in a run.
#[derive(Clone, Debug)]
pub struct RunClock {
    started_instant: Instant,
    started_system: SystemTime,
    started_at: String,
}

impl RunClock {
    /// Captures adjacent monotonic and wall-clock anchors for a measurement run.
    pub fn start() -> Self {
        let started_instant = Instant::now();
        let started_system = SystemTime::now();
        let started_at = format_started_at(started_system);

        Self {
            started_instant,
            started_system,
            started_at,
        }
    }

    /// Returns the RFC 3339 UTC timestamp captured at run start.
    pub fn started_at(&self) -> &str {
        &self.started_at
    }

    /// Returns epoch milliseconds derived from the wall anchor and monotonic elapsed time.
    pub fn now_unix_ms(&self) -> i64 {
        system_time_unix_ms(self.started_system)
            .saturating_add(positive_duration_millis(self.started_instant.elapsed()))
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.started_instant.elapsed()
    }
}

fn system_time_unix_ms(time: SystemTime) -> i64 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => positive_duration_millis(duration),
        Err(error) => negative_duration_millis(error.duration()),
    }
}

fn positive_duration_millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn negative_duration_millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).map_or(i64::MIN, |milliseconds| -milliseconds)
}

fn format_started_at(time: SystemTime) -> String {
    let bounded_duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(Duration::ZERO, |duration| {
            duration.min(MAX_RFC3339_DURATION)
        });
    let bounded_time = SystemTime::UNIX_EPOCH
        .checked_add(bounded_duration)
        .unwrap_or(SystemTime::UNIX_EPOCH);

    humantime::format_rfc3339_millis(bounded_time).to_string()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{
        format_started_at, negative_duration_millis, positive_duration_millis, system_time_unix_ms,
    };

    #[test]
    fn epoch_milliseconds_support_pre_epoch_values() {
        let before_epoch = SystemTime::UNIX_EPOCH
            .checked_sub(Duration::from_millis(1_500))
            .expect("fixture is representable");

        assert_eq!(system_time_unix_ms(before_epoch), -1_500);
    }

    #[test]
    fn epoch_millisecond_components_saturate_at_signed_bounds() {
        assert_eq!(positive_duration_millis(Duration::MAX), i64::MAX);
        assert_eq!(negative_duration_millis(Duration::MAX), i64::MIN);
    }

    #[test]
    fn rfc3339_formatting_clamps_unsupported_system_times_without_panicking() {
        let before_epoch = SystemTime::UNIX_EPOCH
            .checked_sub(Duration::from_millis(1))
            .expect("fixture is representable");
        let after_year_9999 = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(253_402_300_800))
            .expect("fixture is representable");

        assert_eq!(format_started_at(before_epoch), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_started_at(after_year_9999),
            "9999-12-31T23:59:59.999Z"
        );
    }
}
