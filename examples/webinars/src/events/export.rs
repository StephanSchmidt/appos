//! What leaves the app: calendar links, the .ics file and the attendee
//! list as CSV and as text for a spreadsheet. Plain Rust, no IO. The shell
//! sends only HTML pages, so files go out as data: links the browser
//! downloads.

use super::clock::{self, Zone};
use super::domain::{Registration, Session, Status};
use crate::appos::encode;
use crate::appos::time::Time;

/// What a calendar entry says: the description and, for people with a
/// seat, the join link.
fn details(session: &Session) -> String {
    format!(
        "{}\n\nSpeaker: {}{}\n\nJoin on Zoom: {}",
        session.description,
        session.speaker,
        if session.role.is_empty() {
            String::new()
        } else {
            format!(", {}", session.role)
        },
        session.join_link
    )
}

pub fn google_calendar(session: &Session) -> String {
    format!(
        "https://calendar.google.com/calendar/render?action=TEMPLATE&text={}&dates={}/{}&details={}&location={}",
        encode(&session.title),
        clock::utc_compact(session.start),
        clock::utc_compact(session.end()),
        encode(&details(session)),
        encode(&session.join_link),
    )
}

/// Outlook on the web with a work account (Microsoft 365). Outlook on the
/// desktop and personal Outlook.com accounts take the .ics file.
pub fn outlook_calendar(session: &Session) -> String {
    format!(
        "https://outlook.office.com/calendar/0/deeplink/compose?path=%2Fcalendar%2Faction%2Fcompose&rru=addevent&subject={}&startdt={}&enddt={}&body={}&location={}",
        encode(&session.title),
        encode(&clock::utc_iso(session.start)),
        encode(&clock::utc_iso(session.end())),
        encode(&details(session)),
        encode(&session.join_link),
    )
}

/// The calendar file (RFC 5545) for Apple Calendar and anything else.
pub fn ics(session: &Session, reg: &Registration, now: i64) -> String {
    let lines = [
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//Kitewell//Webinars//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{}@webinars.kitewell", reg.id),
        format!("DTSTAMP:{}", clock::utc_compact(now)),
        format!("DTSTART:{}", clock::utc_compact(session.start)),
        format!("DTEND:{}", clock::utc_compact(session.end())),
        format!("SUMMARY:{}", ics_text(&session.title)),
        format!("DESCRIPTION:{}", ics_text(&details(session))),
        format!("LOCATION:{}", ics_text(&session.join_link)),
        format!("URL:{}", ics_text(&session.join_link)),
        "END:VEVENT".into(),
        "END:VCALENDAR".into(),
    ];
    lines
        .iter()
        .map(|l| fold(l))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

/// A link that downloads the text as a file.
pub fn data_url(mime: &str, text: &str) -> String {
    format!("data:{mime};charset=utf-8,{}", encode(text))
}

fn ics_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
        .replace('\r', "")
}

/// Lines of at most 75 bytes, continued with a space (RFC 5545 3.1).
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut width = 0;
    for c in line.chars() {
        if width + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += c.len_utf8();
    }
    out
}

/// One attendee row, as the team sees it and HubSpot imports it.
pub struct Row<'a> {
    pub reg: &'a Registration,
    pub status: Status,
}

pub const COLUMNS: [&str; 15] = [
    "First name",
    "Last name",
    "Email",
    "Company",
    "Job title",
    "Status",
    "Waitlist position",
    "Marketing consent",
    "Consent given at (Berlin)",
    "Consent text shown",
    "Registered at (Berlin)",
    "Source",
    "Campaign",
    "Free mail",
    "Cancelled at (Berlin)",
];

/// "2026-11-13 17:40", Berlin time.
pub fn berlin_stamp(at: Time) -> String {
    Zone::Berlin.local(at.unix()).stamp()
}

fn cells(row: &Row) -> [String; 15] {
    let r = row.reg;
    [
        r.first_name.clone(),
        r.last_name.clone(),
        r.email.clone(),
        r.company.clone(),
        r.job_title.clone(),
        row.status.label().into(),
        match row.status {
            Status::Waitlist(n) => n.to_string(),
            Status::Seated | Status::Cancelled => String::new(),
        },
        if r.consent { "yes" } else { "no" }.into(),
        r.consent_at.map(berlin_stamp).unwrap_or_default(),
        r.consent_text.clone(),
        berlin_stamp(r.registered_at),
        r.source.clone(),
        r.campaign.clone(),
        if r.free_mail { "yes" } else { "no" }.into(),
        r.cancelled_at.map(berlin_stamp).unwrap_or_default(),
    ]
}

/// RFC 4180 CSV. Cells that a spreadsheet would run as a formula get a
/// leading apostrophe.
pub fn csv(rows: &[Row]) -> String {
    let mut out = csv_line(COLUMNS.iter().map(|c| c.to_string()));
    for row in rows {
        out.push_str(&csv_line(cells(row).into_iter()));
    }
    out
}

fn csv_line(cells: impl Iterator<Item = String>) -> String {
    let cells: Vec<String> = cells
        .map(|c| {
            let c = spreadsheet_text(c);
            if c.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", c.replace('"', "\"\""))
            } else {
                c
            }
        })
        .collect();
    cells.join(",") + "\r\n"
}

fn spreadsheet_text(text: String) -> String {
    if text.starts_with(['=', '+', '-', '@', '\t', '\r', '\n']) {
        format!("'{text}")
    } else {
        text
    }
}

/// Tab-separated, to select and paste into a spreadsheet.
pub fn tsv(rows: &[Row]) -> String {
    let clean = |c: String| spreadsheet_text(c).replace(['\t', '\n', '\r'], " ");
    let mut lines = vec![COLUMNS.join("\t")];
    for row in rows {
        lines.push(
            cells(row)
                .into_iter()
                .map(clean)
                .collect::<Vec<_>>()
                .join("\t"),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::{RegId, SessionId};

    fn session() -> Session {
        Session {
            id: SessionId::new(),
            title: "Plan a quarter, fast".into(),
            description: "Mira walks through it.".into(),
            date: "2026-12-04".into(),
            time: "16:00".into(),
            start: clock::berlin_to_utc("2026-12-04", "16:00").unwrap(),
            minutes: 45,
            speaker: "Mira Jensen".into(),
            role: "Head of Customer Education".into(),
            seats: 50,
            join_link: "https://meet.kitewell.example/plan-a-quarter".into(),
            ..Session::default()
        }
    }

    fn reg() -> Registration {
        Registration {
            id: RegId::new(),
            first_name: "=Ann".into(),
            last_name: "Lee, Jr.".into(),
            email: "ann@gmail.com".into(),
            company: "Acme".into(),
            consent: true,
            consent_text: "Yes, send me news.".into(),
            consent_at: Time::parse("2026-11-13T16:40:00Z"),
            registered_at: Time::parse("2026-11-13T16:40:00Z").unwrap(),
            source: "linkedin".into(),
            free_mail: true,
            ..Registration::default()
        }
    }

    #[test]
    fn google_link_has_utc_times() {
        let link = google_calendar(&session());
        assert!(link.contains("dates=20261204T150000Z/20261204T154500Z"));
        assert!(link.contains("text=Plan%20a%20quarter%2C%20fast"));
    }

    #[test]
    fn outlook_link_has_iso_times() {
        let link = outlook_calendar(&session());
        assert!(link.contains("startdt=2026-12-04T15%3A00%3A00Z"));
        assert!(link.contains("enddt=2026-12-04T15%3A45%3A00Z"));
    }

    #[test]
    fn ics_is_escaped_and_folded() {
        let file = ics(&session(), &reg(), 0);
        assert!(file.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(file.contains("DTSTART:20261204T150000Z\r\n"));
        assert!(file.contains("SUMMARY:Plan a quarter\\, fast\r\n"));
        assert!(file.lines().all(|l| l.len() <= 75));
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let r = reg();
        let rows = [Row {
            reg: &r,
            status: Status::Waitlist(2),
        }];
        let text = csv(&rows);
        let line = text.lines().nth(1).unwrap();
        assert!(line.starts_with(
            "'=Ann,\"Lee, Jr.\",ann@gmail.com,Acme,,waitlist,2,yes,2026-11-13 17:40,"
        ));
        assert!(line.ends_with(",linkedin,,yes,"));
    }

    #[test]
    fn tsv_has_a_line_per_person() {
        let r = reg();
        let rows = [Row {
            reg: &r,
            status: Status::Seated,
        }];
        let text = tsv(&rows);
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\tregistered\t"));
    }

    #[test]
    fn pasted_attendee_data_cannot_become_spreadsheet_formulas() {
        use crate::events::domain::{Applicant, ApplicantForm, Tracking};

        let applicant = Applicant::parse(&ApplicantForm {
            first_name: "=1+1".into(),
            last_name: "+2+2".into(),
            email: "-3+3@example.com".into(),
            company: "@SUM(1)".into(),
            ..ApplicantForm::default()
        })
        .unwrap();
        let r = Registration::new(
            SessionId::new(),
            applicant,
            Tracking::default(),
            Time::parse("2026-11-13T16:40:00Z").unwrap(),
        );
        let text = tsv(&[Row {
            reg: &r,
            status: Status::Seated,
        }]);
        let columns: Vec<_> = text.lines().nth(1).unwrap().split('\t').collect();
        assert_eq!(
            &columns[..4],
            &["'=1+1", "'+2+2", "'-3+3@example.com", "'@SUM(1)"]
        );
    }
}
