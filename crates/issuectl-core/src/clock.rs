//! Injectable source of wall-clock time for domain operations.

use chrono::{DateTime, Local, NaiveDate, Utc};

/// Source of the current time used by time-dependent core logic.
///
/// Callers use [`SystemClock`] in normal operation and [`FixedClock`] in
/// deterministic tests. Dates intentionally use the local calendar because
/// persisted `closed:` and `updated:` values historically do too.
pub trait Clock {
    /// The current instant in UTC.
    fn now_utc(&self) -> DateTime<Utc>;

    /// Today's local calendar date.
    fn today(&self) -> NaiveDate {
        self.now_utc().with_timezone(&Local).date_naive()
    }

    /// Today's local calendar date in the persisted frontmatter format.
    fn today_string(&self) -> String {
        self.today().format("%Y-%m-%d").to_string()
    }
}

/// The production wall-clock implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock pinned to one instant, for deterministic tests and callers.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    now: DateTime<Utc>,
}

impl FixedClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self { now }
    }
}

impl Clock for FixedClock {
    fn now_utc(&self) -> DateTime<Utc> {
        self.now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn fixed_clock_pins_the_instant_and_date() {
        let clock = FixedClock::new(Utc.with_ymd_and_hms(2026, 2, 28, 12, 0, 0).unwrap());
        assert_eq!(clock.now_utc().to_rfc3339(), "2026-02-28T12:00:00+00:00");
        assert_eq!(clock.today_string(), "2026-02-28");
    }
}
