//! The three time zones the audience lives in. Times are Unix seconds (UTC);
//! parsing and calendar math are `appos::time`'s. The rules are today's: the
//! EU switches on the last Sundays of March and October at 01:00 UTC, the US
//! on the second Sunday of March and the first Sunday of November at 02:00
//! local time.

pub use crate::appos::time::MINUTE;
use crate::appos::time::{self, DAY, Date, HOUR, Time};

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Berlin,
    London,
    NewYork,
}

impl Zone {
    pub const ALL: [Zone; 3] = [Zone::Berlin, Zone::London, Zone::NewYork];

    pub fn name(self) -> &'static str {
        match self {
            Zone::Berlin => "Berlin",
            Zone::London => "London",
            Zone::NewYork => "New York",
        }
    }

    /// The offset from UTC at that moment, in seconds.
    pub fn offset(self, utc: i64) -> i64 {
        match self {
            Zone::Berlin if eu_summer(utc) => 2 * HOUR,
            Zone::Berlin => HOUR,
            Zone::London if eu_summer(utc) => HOUR,
            Zone::London => 0,
            Zone::NewYork if us_summer(utc) => -4 * HOUR,
            Zone::NewYork => -5 * HOUR,
        }
    }

    /// The wall clock in this zone at that moment.
    pub fn local(self, utc: i64) -> Local {
        Local(Time::utc(utc).at_offset(self.offset(utc)))
    }
}

/// A wall-clock time: what a clock in some zone shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local(Time);

impl Local {
    pub fn date(&self) -> Date {
        self.0.date()
    }

    /// Days since 1970-01-01, to compare dates.
    pub fn days(&self) -> i64 {
        self.date().days()
    }

    /// "16:00"
    pub fn time(&self) -> String {
        let (hour, minute, _) = self.0.clock();
        format!("{hour:02}:{minute:02}")
    }

    /// "Thursday"
    pub fn weekday(&self) -> &'static str {
        WEEKDAYS[self.date().weekday() as usize]
    }

    /// "Thu"
    pub fn weekday_short(&self) -> &'static str {
        &self.weekday()[..3]
    }

    /// "December"
    pub fn month_name(&self) -> &'static str {
        MONTHS[(self.date().month() - 1) as usize]
    }

    /// "Dec"
    pub fn month_short(&self) -> &'static str {
        &self.month_name()[..3]
    }

    /// "4", the day of the month.
    pub fn day(&self) -> u32 {
        self.date().day()
    }

    /// "Thursday 4 December 2026"
    pub fn long_date(&self) -> String {
        let date = self.date();
        format!(
            "{} {} {} {}",
            self.weekday(),
            date.day(),
            self.month_name(),
            date.year()
        )
    }

    /// "2026-12-04"
    pub fn iso_date(&self) -> String {
        self.date().to_string()
    }

    /// "2026-12-04 16:00"
    pub fn stamp(&self) -> String {
        format!("{} {}", self.iso_date(), self.time())
    }
}

/// Berlin wall-clock time ("2026-12-04", "16:00") as a moment. None if the
/// date or the time isn't one, including the missing spring-forward hour.
/// The repeated autumn hour uses its first occurrence.
pub fn berlin_to_utc(date: &str, time: &str) -> Option<i64> {
    let date = Date::parse(date)?;
    let (hour, minute, _) = time::parse_time(time)?;
    let wall = date.at(hour, minute, 0, 0).unix();
    [2 * HOUR, HOUR]
        .into_iter()
        .map(|offset| wall - offset)
        .find(|&utc| utc + Zone::Berlin.offset(utc) == wall)
}

/// "20261204T150000Z", for calendar files and links.
pub fn utc_compact(utc: i64) -> String {
    let t = Time::utc(utc);
    let date = t.date();
    let (hour, minute, _) = t.clock();
    format!(
        "{:04}{:02}{:02}T{hour:02}{minute:02}00Z",
        date.year(),
        date.month(),
        date.day()
    )
}

/// "2026-12-04T15:00:00Z"
pub fn utc_iso(utc: i64) -> String {
    let t = Time::utc(utc);
    let (hour, minute, _) = t.clock();
    format!("{}T{hour:02}:{minute:02}:00Z", t.date())
}

fn eu_summer(utc: i64) -> bool {
    let year = Time::utc(utc).date().year();
    let start = last_sunday(year, 3) * DAY + HOUR;
    let end = last_sunday(year, 10) * DAY + HOUR;
    (start..end).contains(&utc)
}

fn us_summer(utc: i64) -> bool {
    let year = Time::utc(utc).date().year();
    let start = nth_sunday(year, 3, 2) * DAY + 7 * HOUR;
    let end = nth_sunday(year, 11, 1) * DAY + 6 * HOUR;
    (start..end).contains(&utc)
}

fn last_sunday(year: i64, month: u32) -> i64 {
    let last =
        Date::new(year, month, time::days_in_month(year, month)).expect("the month's last day");
    // Monday is 0, Sunday 6: step back to the Sunday.
    last.days() - i64::from((last.weekday() + 1) % 7)
}

fn nth_sunday(year: i64, month: u32, n: i64) -> i64 {
    let first = Date::new(year, month, 1).expect("the month's first day");
    first.days() + i64::from(6 - first.weekday()) + 7 * (n - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> i64 {
        Time::parse(text).unwrap().unix()
    }

    fn date(days: i64) -> String {
        Date::from_days(days).to_string()
    }

    #[test]
    fn berlin_conversion_never_silently_changes_the_wall_clock() {
        assert_eq!(berlin_to_utc("2026-03-29", "02:30"), None);
        assert_eq!(berlin_to_utc("2026-02-29", "16:00"), None);
        assert_eq!(berlin_to_utc("2026-12-04", "4pm"), None);
        for (date, time, expected) in [
            ("2026-03-29", "01:59", "2026-03-29T00:59:00Z"),
            ("2026-03-29", "03:00", "2026-03-29T01:00:00Z"),
            // In autumn, retain the first occurrence of the repeated hour.
            ("2026-10-25", "02:30", "2026-10-25T00:30:00Z"),
            ("2026-10-25", "03:00", "2026-10-25T02:00:00Z"),
        ] {
            let utc = berlin_to_utc(date, time).unwrap();
            assert_eq!(utc, at(expected));
            let local = Zone::Berlin.local(utc);
            assert_eq!(local.iso_date(), date);
            assert_eq!(local.time(), time);
        }
    }

    #[test]
    fn sundays_of_the_switch() {
        // 2026: EU 29 March and 25 October, US 8 March and 1 November.
        assert_eq!(date(last_sunday(2026, 3)), "2026-03-29");
        assert_eq!(date(last_sunday(2026, 10)), "2026-10-25");
        assert_eq!(date(nth_sunday(2026, 3, 2)), "2026-03-08");
        assert_eq!(date(nth_sunday(2026, 11, 1)), "2026-11-01");
        // A month that ends on a Sunday: 31 May 2026.
        assert_eq!(date(last_sunday(2026, 5)), "2026-05-31");
    }

    #[test]
    fn winter_session_in_three_zones() {
        let start = berlin_to_utc("2026-12-04", "16:00").unwrap();
        assert_eq!(utc_compact(start), "20261204T150000Z");
        assert_eq!(Zone::Berlin.local(start).time(), "16:00");
        assert_eq!(Zone::London.local(start).time(), "15:00");
        assert_eq!(Zone::NewYork.local(start).time(), "10:00");
        // 4 December 2026 is a Friday.
        assert_eq!(
            Zone::Berlin.local(start).long_date(),
            "Friday 4 December 2026"
        );
    }

    #[test]
    fn summer_and_the_weeks_between_the_switches() {
        let july = berlin_to_utc("2027-07-01", "16:00").unwrap();
        assert_eq!(utc_iso(july), "2027-07-01T14:00:00Z");
        assert_eq!(Zone::London.local(july).time(), "15:00");
        assert_eq!(Zone::NewYork.local(july).time(), "10:00");
        // US already on summer time, Europe not yet: only 5 hours apart.
        let march = berlin_to_utc("2027-03-20", "16:00").unwrap();
        assert_eq!(Zone::NewYork.local(march).time(), "11:00");
    }

    #[test]
    fn late_berlin_is_the_day_before_nowhere_but_early_is() {
        let early = berlin_to_utc("2027-01-14", "03:00").unwrap();
        let ny = Zone::NewYork.local(early);
        assert_eq!(ny.time(), "21:00");
        assert_eq!(ny.weekday(), "Wednesday");
        assert_eq!(ny.days() + 1, Zone::Berlin.local(early).days());
    }

    #[test]
    fn weekdays_and_months() {
        let t = Zone::Berlin.local(berlin_to_utc("2027-01-14", "15:00").unwrap());
        assert_eq!(t.long_date(), "Thursday 14 January 2027");
        assert_eq!(t.weekday_short(), "Thu");
        assert_eq!(t.month_short(), "Jan");
        assert_eq!(t.stamp(), "2027-01-14 15:00");
    }
}
