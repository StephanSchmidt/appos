//! How a session reads on a page: its date and the time in Berlin, London
//! and New York, its length and the seats left. Shared by the public pages
//! and the team's. Plain Rust, no IO.

use super::clock::Zone;
use super::domain::{Roster, Session};

/// The time in one zone; `other_day` when the date there isn't Berlin's.
pub struct ZoneTime {
    pub name: &'static str,
    pub time: String,
    pub other_day: Option<String>,
}

pub struct SessionView {
    pub id: String,
    pub title: String,
    pub paragraphs: Vec<String>,
    pub speaker: String,
    pub role: String,
    /// "Friday 4 December 2026", the Berlin date.
    pub long_date: String,
    pub day: u32,
    pub month: &'static str,
    pub weekday: &'static str,
    pub zones: Vec<ZoneTime>,
    /// "16:00 to 16:45 Berlin time"
    pub berlin_span: String,
    pub length: String,
    pub seats: u32,
    pub seats_left: u32,
    pub seated: usize,
    pub waiting: usize,
    pub cancellations: usize,
    pub cancelled: bool,
    pub started: bool,
    pub ended: bool,
}

impl SessionView {
    pub fn new(session: &Session, roster: &Roster, now: i64) -> SessionView {
        let berlin = Zone::Berlin.local(session.start);
        let zones = Zone::ALL
            .iter()
            .map(|zone| {
                let local = zone.local(session.start);
                ZoneTime {
                    name: zone.name(),
                    time: local.time(),
                    other_day: (local.days() != berlin.days()).then(|| {
                        format!(
                            "{} {} {}",
                            local.weekday_short(),
                            local.day(),
                            local.month_short()
                        )
                    }),
                }
            })
            .collect();
        SessionView {
            id: session.id.to_string(),
            title: session.title.clone(),
            paragraphs: session
                .description
                .split("\n")
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(String::from)
                .collect(),
            speaker: session.speaker.clone(),
            role: session.role.clone(),
            long_date: berlin.long_date(),
            day: berlin.day(),
            month: berlin.month_short(),
            weekday: berlin.weekday_short(),
            zones,
            berlin_span: format!(
                "{} to {} Berlin time",
                berlin.time(),
                Zone::Berlin.local(session.end()).time()
            ),
            length: length(session.minutes),
            seats: session.seats,
            seats_left: roster.seats_left(session.seats),
            seated: roster.seated.len(),
            waiting: roster.waitlist.len(),
            cancellations: roster.cancelled.len(),
            cancelled: session.is_cancelled(),
            started: session.has_started(now),
            ended: now >= session.end(),
        }
    }

    /// Full when no seat is left or people already wait for one.
    pub fn is_full(&self) -> bool {
        self.seats_left == 0 || self.waiting > 0
    }

    pub fn is_open(&self) -> bool {
        !self.cancelled && !self.started
    }

    /// "12 seats left", "1 seat left", "Full: join the waitlist"
    pub fn seats_label(&self) -> String {
        match self.seats_left {
            _ if self.is_full() => "Full: join the waitlist".into(),
            1 => "1 seat left".into(),
            n => format!("{n} seats left"),
        }
    }

    /// "MJ" for Mira Jensen: the speaker's avatar.
    pub fn initials(&self) -> String {
        self.speaker
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .filter(|c| c.is_alphabetic())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect()
    }

    /// How full, 0 to 100, for the bar on the team's page.
    pub fn percent_taken(&self) -> u32 {
        (self.seated as u32 * 100)
            .checked_div(self.seats)
            .unwrap_or(0)
            .min(100)
    }
}

/// "45 minutes", "1 hour", "90 minutes"
pub fn length(minutes: u32) -> String {
    match minutes {
        60 => "1 hour".into(),
        m if m > 60 && m % 60 == 0 => format!("{} hours", m / 60),
        m => format!("{m} minutes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::clock;
    use crate::events::domain::RegId;

    fn session(date: &str, time: &str, minutes: u32, seats: u32) -> Session {
        Session {
            title: "Capacity planning for agencies".into(),
            description: "How agencies plan.\n\nLive demo.".into(),
            start: clock::berlin_to_utc(date, time).unwrap(),
            minutes,
            seats,
            ..Session::default()
        }
    }

    const NOW: i64 = 1_794_000_000; // November 2026

    #[test]
    fn three_zones_and_the_berlin_span() {
        let v = SessionView::new(
            &session("2026-12-09", "17:00", 60, 50),
            &Roster::default(),
            NOW,
        );
        assert_eq!(v.long_date, "Wednesday 9 December 2026");
        let times: Vec<String> = v
            .zones
            .iter()
            .map(|z| format!("{} {}", z.time, z.name))
            .collect();
        assert_eq!(times, ["17:00 Berlin", "16:00 London", "11:00 New York"]);
        assert_eq!(v.berlin_span, "17:00 to 18:00 Berlin time");
        assert_eq!(v.length, "1 hour");
        assert_eq!(v.paragraphs, ["How agencies plan.", "Live demo."]);
        assert!(v.zones.iter().all(|z| z.other_day.is_none()));
    }

    #[test]
    fn early_berlin_is_yesterday_in_new_york() {
        let v = SessionView::new(
            &session("2026-12-09", "05:00", 30, 50),
            &Roster::default(),
            NOW,
        );
        assert_eq!(v.zones[2].time, "23:00");
        assert_eq!(v.zones[2].other_day.as_deref(), Some("Tue 8 Dec"));
    }

    #[test]
    fn seats_left_reads_well() {
        let mut roster = Roster::default();
        let s = session("2026-12-09", "17:00", 60, 2);
        assert_eq!(
            SessionView::new(&s, &roster, NOW).seats_label(),
            "2 seats left"
        );
        roster.add(RegId::new(), 2);
        assert_eq!(
            SessionView::new(&s, &roster, NOW).seats_label(),
            "1 seat left"
        );
        roster.add(RegId::new(), 2);
        assert_eq!(
            SessionView::new(&s, &roster, NOW).seats_label(),
            "Full: join the waitlist"
        );
        assert_eq!(SessionView::new(&s, &roster, NOW).percent_taken(), 100);
    }

    #[test]
    fn lengths() {
        assert_eq!(length(45), "45 minutes");
        assert_eq!(length(120), "2 hours");
        assert_eq!(length(90), "90 minutes");
    }
}
