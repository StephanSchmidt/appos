//! Times as the desk stores them (RFC 3339 text, "2026-11-11T14:05:09+01:00")
//! and the little arithmetic the desk needs: hours waiting, 30 days of
//! validity, quarters. Parsing and calendar math are appos::time's. Plain
//! Rust, no IO.

pub use crate::appos::time::{DAY, HOUR};
use crate::appos::time::{Date, Time};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Seconds since 1970, or 0 for text that isn't a time (old or broken data
/// then sorts first instead of taking a page down).
pub fn secs(text: &str) -> i64 {
    Time::parse(text).map_or(0, Time::unix)
}

/// `text` moved by `secs` seconds, in the same offset.
pub fn add(text: &str, secs: i64) -> String {
    Time::parse(text).unwrap_or_default().plus(secs).to_string()
}

/// The local date of a time, "2026-11-11".
pub fn date(text: &str) -> &str {
    text.get(..10).unwrap_or("")
}

/// "15 Dec 2026" for "2026-12-15" (or a time on that day).
pub fn day(text: &str) -> String {
    match Date::parse(date(text)) {
        Some(d) => format!("{} {}", day_month(d), d.year()),
        None => text.to_string(),
    }
}

/// "13 Nov 2026, 17:41".
pub fn when(text: &str) -> String {
    format!("{}, {}", day(text), text.get(11..16).unwrap_or(""))
}

/// "13 Nov, 17:41": for lists, where the year is clear.
pub fn when_short(text: &str) -> String {
    match Date::parse(date(text)) {
        Some(d) => format!("{}, {}", day_month(d), text.get(11..16).unwrap_or("")),
        None => text.to_string(),
    }
}

fn day_month(d: Date) -> String {
    format!("{} {}", d.day(), MONTHS[d.month() as usize - 1])
}

/// A duration for people: "45 min", "5 h", "31 h", "3 days".
pub fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < HOUR {
        format!("{} min", (secs / 60).max(1))
    } else if secs < 48 * HOUR {
        format!("{} h", secs / HOUR)
    } else {
        format!("{} days", secs / DAY)
    }
}

/// The calendar quarter of a time's (or a date's) local date: (2026, 4).
pub fn quarter(text: &str) -> Option<(i64, i64)> {
    let d = Date::parse(date(text))?;
    Some((d.year(), i64::from((d.month() - 1) / 3 + 1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_that_isnt_a_time_sorts_first() {
        assert_eq!(secs("1970-01-01T00:00:00Z"), 0);
        assert_eq!(
            secs("2026-11-11T13:05:09Z"),
            secs("2026-11-11T14:05:09+01:00")
        );
        assert_eq!(secs("soon"), 0);
    }

    #[test]
    fn adds_across_months_and_years() {
        assert_eq!(
            add("2026-12-15T10:00:00+01:00", 30 * DAY),
            "2027-01-14T10:00:00+01:00"
        );
        assert_eq!(
            add("2026-03-01T00:30:00-05:00", -HOUR),
            "2026-02-28T23:30:00-05:00"
        );
    }

    #[test]
    fn dates() {
        assert_eq!(day("2026-12-15"), "15 Dec 2026");
        assert_eq!(day("15.12.2026"), "15.12.2026");
        assert_eq!(when("2026-11-13T17:41:09+01:00"), "13 Nov 2026, 17:41");
        assert_eq!(when_short("2026-11-13T07:41:09+01:00"), "13 Nov, 07:41");
    }

    #[test]
    fn durations_and_quarters() {
        assert_eq!(duration(5), "1 min");
        assert_eq!(duration(31 * HOUR + 59 * 60), "31 h");
        assert_eq!(duration(3 * DAY + HOUR), "3 days");
        assert_eq!(quarter("2026-11-13T17:41:09+01:00"), Some((2026, 4)));
        assert_eq!(quarter("2027-03-31T23:59:59Z"), Some((2027, 1)));
        assert_eq!(quarter("2026-11-13"), Some((2026, 4)));
    }
}
