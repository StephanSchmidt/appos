//! The data and its rules: sessions, registrations, who has a seat and who
//! waits, and the checks of everything people type. Plain Rust, no IO.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::clock;
use crate::appos::time::{self, Date, Time};

/// The Zoom licence allows at most this many people.
pub const SEATS_MAX: u32 = 100;
pub const MINUTES_MAX: u32 = 480;
pub const TITLE_MAX: usize = 120;
pub const DESCRIPTION_MAX: usize = 600;
pub const NAME_MAX: usize = 80;
pub const COMPANY_MAX: usize = 120;
pub const EMAIL_MAX: usize = 254;
pub const LINK_MAX: usize = 500;
const TRACKING_MAX: usize = 80;

/// The sentence next to the checkbox. Every registration keeps the text
/// its person saw, so a later change here doesn't change old records.
pub const CONSENT_TEXT: &str =
    "Yes, send me Kitewell product news and event invites. You can unsubscribe at any time.";

macro_rules! id_type {
    ($name:ident) => {
        /// A UUID v7: it starts with the time, so ids sort in the order
        /// they were made.
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
        )]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> $name {
                $name(Uuid::now_v7())
            }

            /// An id from a URL; None if it isn't one (the handler answers
            /// 404, as for an id nobody has).
            pub fn parse(text: &str) -> Option<$name> {
                Uuid::try_parse(text).ok().map($name)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(SessionId);
id_type!(RegId);

/// A webinar. Stored data outlives the code: `#[serde(default)]` lets data
/// saved before a new field existed still load.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub id: SessionId,
    pub title: String,
    pub description: String,
    /// Berlin date and time as planned: "2026-12-04", "16:00".
    pub date: String,
    pub time: String,
    /// The same moment in Unix seconds, for sorting and the other zones.
    pub start: i64,
    pub minutes: u32,
    pub speaker: String,
    pub role: String,
    pub seats: u32,
    pub join_link: String,
    pub cancelled_at: Option<Time>,
    pub created_at: Time,
    pub updated_at: Time,
}

impl Session {
    pub fn new(input: SessionInput, now: Time) -> Session {
        let mut session = Session {
            id: SessionId::new(),
            created_at: now,
            ..Session::default()
        };
        session.apply(input, now);
        session
    }

    /// Takes over what the team typed into the session form.
    pub fn apply(&mut self, input: SessionInput, now: Time) {
        self.title = input.title;
        self.description = input.description;
        self.date = input.date;
        self.time = input.time;
        self.start = input.start;
        self.minutes = input.minutes;
        self.speaker = input.speaker;
        self.role = input.role;
        self.seats = input.seats;
        self.join_link = input.join_link;
        self.updated_at = now;
    }

    pub fn end(&self) -> i64 {
        self.start + i64::from(self.minutes) * clock::MINUTE
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled_at.is_some()
    }

    pub fn has_started(&self, now: i64) -> bool {
        now >= self.start
    }

    /// Open for registration: not cancelled and not started.
    pub fn is_open(&self, now: i64) -> bool {
        !self.is_cancelled() && !self.has_started(now)
    }
}

/// Who has a seat, who waits (first come, first served) and who cancelled.
/// One per session, in one key: every registration and cancellation reads
/// and writes it, so two people can't take the last seat at the same
/// moment (the shell runs one of them again, see STORAGE in appos guide).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Roster {
    pub seated: Vec<RegId>,
    pub waitlist: Vec<RegId>,
    pub cancelled: Vec<RegId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Seated,
    /// The place on the waitlist, from 1.
    Waitlist(usize),
    Cancelled,
}

impl Status {
    pub fn is_active(self) -> bool {
        !matches!(self, Status::Cancelled)
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Seated => "registered",
            Status::Waitlist(_) => "waitlist",
            Status::Cancelled => "cancelled",
        }
    }
}

impl Roster {
    pub fn status(&self, id: RegId) -> Option<Status> {
        if self.seated.contains(&id) {
            Some(Status::Seated)
        } else if let Some(i) = self.waitlist.iter().position(|w| *w == id) {
            Some(Status::Waitlist(i + 1))
        } else if self.cancelled.contains(&id) {
            Some(Status::Cancelled)
        } else {
            None
        }
    }

    pub fn seats_left(&self, seats: u32) -> u32 {
        seats.saturating_sub(self.seated.len() as u32)
    }

    /// A seat if there is one and nobody waits for it, else the end of the
    /// waitlist.
    pub fn add(&mut self, id: RegId, seats: u32) -> Status {
        if self.waitlist.is_empty() && self.seats_left(seats) > 0 {
            self.seated.push(id);
            Status::Seated
        } else {
            self.waitlist.push(id);
            Status::Waitlist(self.waitlist.len())
        }
    }

    /// Gives free seats to the first people on the waitlist; returns who
    /// moved up.
    pub fn promote(&mut self, seats: u32) -> Vec<RegId> {
        let n = (self.seats_left(seats) as usize).min(self.waitlist.len());
        let moved: Vec<RegId> = self.waitlist.drain(..n).collect();
        self.seated.extend(&moved);
        moved
    }

    /// The person gives up their seat or place; the waitlist moves up.
    /// False if they had none.
    pub fn cancel(&mut self, id: RegId, seats: u32) -> bool {
        if !self.take_out(id) {
            return false;
        }
        self.cancelled.push(id);
        self.promote(seats);
        true
    }

    /// Forgets the person entirely (their data is deleted); the waitlist
    /// moves up.
    pub fn remove(&mut self, id: RegId, seats: u32) {
        self.take_out(id);
        self.cancelled.retain(|c| *c != id);
        self.promote(seats);
    }

    pub fn is_empty(&self) -> bool {
        self.seated.is_empty() && self.waitlist.is_empty() && self.cancelled.is_empty()
    }

    fn take_out(&mut self, id: RegId) -> bool {
        let before = self.seated.len() + self.waitlist.len();
        self.seated.retain(|s| *s != id);
        self.waitlist.retain(|w| *w != id);
        before != self.seated.len() + self.waitlist.len()
    }
}

/// One person's registration for one session. Whether they have a seat is
/// the roster's business, so moving up the waitlist writes one key.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Registration {
    pub id: RegId,
    pub session: SessionId,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub company: String,
    pub job_title: String,
    pub consent: bool,
    /// The sentence next to the checkbox, as the person saw it.
    pub consent_text: String,
    /// When they ticked it: the time they registered, if they did.
    pub consent_at: Option<Time>,
    pub registered_at: Time,
    pub source: String,
    pub campaign: String,
    pub free_mail: bool,
    pub cancelled_at: Option<Time>,
}

impl Registration {
    pub fn new(session: SessionId, who: Applicant, tracking: Tracking, now: Time) -> Registration {
        Registration {
            id: RegId::new(),
            session,
            free_mail: who.email.is_free_mail(),
            first_name: who.first_name,
            last_name: who.last_name,
            email: who.email.typed,
            company: who.company,
            job_title: who.job_title,
            consent: who.consent,
            consent_text: CONSENT_TEXT.into(),
            consent_at: who.consent.then_some(now),
            registered_at: now,
            source: tracking.source,
            campaign: tracking.campaign,
            cancelled_at: None,
        }
    }

    /// The key for "same email, same session": lowercase.
    pub fn email_key(&self) -> String {
        self.email.to_lowercase()
    }
}

// ---- What people type --------------------------------------------------

/// Why a field didn't parse. The text is for the person typing.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum FieldError {
    #[error("Please fill this in.")]
    Missing,
    #[error("Please keep it to {max} characters.")]
    TooLong { max: usize },
    #[error("That doesn't look like an email address.")]
    NotAnEmail,
    #[error("Please enter a date.")]
    NotADate,
    #[error("Please enter a time, like 16:00.")]
    NotATime,
    #[error(
        "That time does not exist in Berlin when the clocks move forward. Please choose another time."
    )]
    MissingBerlinTime,
    #[error("Please enter a whole number between {min} and {max}.")]
    OutOfRange { min: u32, max: u32 },
    #[error("Please enter a link starting with https://")]
    NotALink,
}

/// The errors of a form, by field name, in the order of the form.
pub type FormErrors = crate::appos::FormErrors<FieldError>;

/// A line of text: trimmed, inner whitespace (line breaks too) collapsed.
pub fn line(input: &str, max: usize, required: bool) -> Result<String, FieldError> {
    let text = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if required && text.is_empty() {
        return Err(FieldError::Missing);
    }
    if text.chars().count() > max {
        return Err(FieldError::TooLong { max });
    }
    Ok(text)
}

/// Text with paragraphs: trimmed, line breaks kept.
fn paragraph(input: &str, max: usize) -> Result<String, FieldError> {
    let text = input.trim().replace("\r\n", "\n");
    if text.is_empty() {
        return Err(FieldError::Missing);
    }
    if text.chars().count() > max {
        return Err(FieldError::TooLong { max });
    }
    Ok(text)
}

fn whole_number(input: &str, min: u32, max: u32) -> Result<u32, FieldError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(FieldError::Missing);
    }
    match input.parse::<u32>() {
        Ok(n) if (min..=max).contains(&n) => Ok(n),
        _ => Err(FieldError::OutOfRange { min, max }),
    }
}

fn link(input: &str) -> Result<String, FieldError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(FieldError::Missing);
    }
    let rest = input.strip_prefix("https://").ok_or(FieldError::NotALink)?;
    if rest.is_empty()
        || rest.starts_with('/')
        || input.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(FieldError::NotALink);
    }
    if input.len() > LINK_MAX {
        return Err(FieldError::TooLong { max: LINK_MAX });
    }
    Ok(input.into())
}

/// An email address: one @, something before it, a domain with a dot after.
#[derive(Debug, Default, PartialEq)]
pub struct Email {
    typed: String,
}

impl Email {
    pub fn parse(input: &str) -> Result<Email, FieldError> {
        let typed = input.trim();
        if typed.is_empty() {
            return Err(FieldError::Missing);
        }
        if typed.len() > EMAIL_MAX {
            return Err(FieldError::TooLong { max: EMAIL_MAX });
        }
        let (local, domain) = typed.rsplit_once('@').ok_or(FieldError::NotAnEmail)?;
        let domain_ok = domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !domain.contains("..")
            && domain
                .chars()
                .all(|c| c.is_alphanumeric() || c == '.' || c == '-');
        let local_ok = !local.is_empty()
            && !local.contains('@')
            && !local
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "<>()[],;:\"\\".contains(c));
        if !domain_ok || !local_ok {
            return Err(FieldError::NotAnEmail);
        }
        Ok(Email {
            typed: typed.into(),
        })
    }

    pub fn key(&self) -> String {
        self.typed.to_lowercase()
    }

    /// A personal mailbox (Gmail, Outlook.com, GMX, ...): sales doesn't call
    /// those.
    pub fn is_free_mail(&self) -> bool {
        let key = self.key();
        let domain = key.rsplit_once('@').map(|(_, d)| d).unwrap_or("");
        is_free_mail_domain(domain)
    }
}

fn is_free_mail_domain(domain: &str) -> bool {
    const EXACT: &[&str] = &[
        "gmail.com",
        "googlemail.com",
        "web.de",
        "t-online.de",
        "freenet.de",
        "arcor.de",
        "posteo.de",
        "posteo.net",
        "mailbox.org",
        "online.de",
        "email.de",
        "icloud.com",
        "me.com",
        "mac.com",
        "aol.com",
        "aol.de",
        "msn.com",
        "proton.me",
        "protonmail.com",
        "pm.me",
        "mail.com",
        "zoho.com",
        "fastmail.com",
        "tutanota.com",
        "tuta.io",
        "hey.com",
        "btinternet.com",
        "sky.com",
        "virginmedia.com",
        "ntlworld.com",
        "talktalk.net",
        "comcast.net",
        "verizon.net",
        "att.net",
        "sbcglobal.net",
        "bellsouth.net",
        "cox.net",
        "charter.net",
        "mail.ru",
        "qq.com",
        "163.com",
        "seznam.cz",
        "wp.pl",
        "o2.pl",
        "libero.it",
        "orange.fr",
        "free.fr",
        "laposte.net",
        "bluewin.ch",
    ];
    // Brands with many country domains: gmx.de, gmx.net, yahoo.co.uk, ...
    const BRANDS: &[&str] = &[
        "gmx",
        "yahoo",
        "ymail",
        "rocketmail",
        "outlook",
        "hotmail",
        "live",
        "yandex",
    ];
    EXACT.contains(&domain)
        || BRANDS.iter().any(|b| {
            domain
                .strip_prefix(b)
                .is_some_and(|rest| rest.starts_with('.'))
        })
}

/// Where a visitor came from: utm_source and utm_campaign of the link
/// they followed. Never fails: odd characters are dropped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracking {
    pub source: String,
    pub campaign: String,
}

impl Tracking {
    pub fn parse(source: &str, campaign: &str) -> Tracking {
        Tracking {
            source: tag(source),
            campaign: tag(campaign),
        }
    }
}

fn tag(input: &str) -> String {
    input
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || "-_.+ ".contains(*c))
        .take(TRACKING_MAX)
        .collect()
}

/// The registration form, as typed.
#[derive(Clone, Debug, Default)]
pub struct ApplicantForm {
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    pub company: String,
    pub job_title: String,
    pub consent: bool,
}

/// The registration form, parsed.
#[derive(Debug, Default)]
pub struct Applicant {
    pub first_name: String,
    pub last_name: String,
    pub email: Email,
    pub company: String,
    pub job_title: String,
    pub consent: bool,
}

impl Applicant {
    pub fn parse(form: &ApplicantForm) -> Result<Applicant, FormErrors> {
        let mut errors = FormErrors::default();
        let applicant = Applicant {
            first_name: errors.take("first_name", line(&form.first_name, NAME_MAX, true)),
            last_name: errors.take("last_name", line(&form.last_name, NAME_MAX, true)),
            email: errors.take("email", Email::parse(&form.email)),
            company: errors.take("company", line(&form.company, COMPANY_MAX, true)),
            job_title: errors.take("job_title", line(&form.job_title, COMPANY_MAX, false)),
            consent: form.consent,
        };
        if errors.is_empty() {
            Ok(applicant)
        } else {
            Err(errors)
        }
    }
}

/// The session form, as typed (also to fill it for editing).
#[derive(Clone, Debug, Default)]
pub struct SessionForm {
    pub title: String,
    pub description: String,
    pub date: String,
    pub time: String,
    pub minutes: String,
    pub speaker: String,
    pub role: String,
    pub seats: String,
    pub join_link: String,
}

impl SessionForm {
    pub fn of(session: &Session) -> SessionForm {
        SessionForm {
            title: session.title.clone(),
            description: session.description.clone(),
            date: session.date.clone(),
            time: session.time.clone(),
            minutes: session.minutes.to_string(),
            speaker: session.speaker.clone(),
            role: session.role.clone(),
            seats: session.seats.to_string(),
            join_link: session.join_link.clone(),
        }
    }

    /// A new session: usually 50 seats.
    pub fn blank() -> SessionForm {
        SessionForm {
            minutes: "45".into(),
            seats: "50".into(),
            time: "16:00".into(),
            ..SessionForm::default()
        }
    }
}

/// The session form, parsed.
#[derive(Debug, Default)]
pub struct SessionInput {
    pub title: String,
    pub description: String,
    pub date: String,
    pub time: String,
    pub start: i64,
    pub minutes: u32,
    pub speaker: String,
    pub role: String,
    pub seats: u32,
    pub join_link: String,
}

impl SessionInput {
    pub fn parse(form: &SessionForm) -> Result<SessionInput, FormErrors> {
        let mut errors = FormErrors::default();
        let date = errors.take(
            "date",
            Date::parse(&form.date)
                .map(|date| date.to_string())
                .ok_or(FieldError::NotADate),
        );
        let time = errors.take(
            "time",
            time::parse_time(&form.time)
                .map(|(h, m, _)| format!("{h:02}:{m:02}"))
                .ok_or(FieldError::NotATime),
        );
        let start = if date.is_empty() || time.is_empty() {
            0 // The invalid field already has an error.
        } else {
            errors.take(
                "time",
                clock::berlin_to_utc(&date, &time).ok_or(FieldError::MissingBerlinTime),
            )
        };
        let input = SessionInput {
            title: errors.take("title", line(&form.title, TITLE_MAX, true)),
            description: errors.take("description", paragraph(&form.description, DESCRIPTION_MAX)),
            start,
            date,
            time,
            minutes: errors.take("minutes", whole_number(&form.minutes, 5, MINUTES_MAX)),
            speaker: errors.take("speaker", line(&form.speaker, NAME_MAX, true)),
            role: errors.take("role", line(&form.role, NAME_MAX, false)),
            seats: errors.take("seats", whole_number(&form.seats, 1, SEATS_MAX)),
            join_link: errors.take("join_link", link(&form.join_link)),
        };
        if errors.is_empty() {
            Ok(input)
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::NOW;

    fn ids(n: usize) -> Vec<RegId> {
        Vec::from_fn(n, |_| RegId::new())
    }

    #[test]
    fn seats_first_then_the_waitlist() {
        let mut roster = Roster::default();
        let [a, b, c] = ids(3)[..] else { panic!() };
        assert_eq!(roster.add(a, 2), Status::Seated);
        assert_eq!(roster.add(b, 2), Status::Seated);
        assert_eq!(roster.add(c, 2), Status::Waitlist(1));
        assert_eq!(roster.seats_left(2), 0);
    }

    #[test]
    fn a_cancelled_seat_goes_to_the_first_on_the_waitlist() {
        let mut roster = Roster::default();
        let [a, b, c, d] = ids(4)[..] else { panic!() };
        for id in [a, b, c, d] {
            roster.add(id, 1);
        }
        assert_eq!(roster.status(d), Some(Status::Waitlist(3)));
        assert!(roster.cancel(a, 1));
        assert_eq!(roster.status(a), Some(Status::Cancelled));
        assert_eq!(roster.status(b), Some(Status::Seated));
        assert_eq!(roster.status(c), Some(Status::Waitlist(1)));
        assert_eq!(roster.status(d), Some(Status::Waitlist(2)));
        assert!(!roster.cancel(a, 1), "cancelling twice changes nothing");
    }

    #[test]
    fn leaving_the_waitlist_moves_the_others_up() {
        let mut roster = Roster::default();
        let [a, b, c] = ids(3)[..] else { panic!() };
        for id in [a, b, c] {
            roster.add(id, 1);
        }
        roster.cancel(b, 1);
        assert_eq!(roster.status(a), Some(Status::Seated));
        assert_eq!(roster.status(c), Some(Status::Waitlist(1)));
    }

    #[test]
    fn more_seats_seat_the_waitlist() {
        let mut roster = Roster::default();
        let all = ids(5);
        for id in &all {
            roster.add(*id, 2);
        }
        assert_eq!(roster.promote(4), all[2..4].to_vec());
        assert_eq!(roster.status(all[4]), Some(Status::Waitlist(1)));
        assert_eq!(roster.promote(4), vec![]);
    }

    #[test]
    fn nobody_jumps_the_waitlist() {
        let mut roster = Roster::default();
        let [a, b, c] = ids(3)[..] else { panic!() };
        roster.add(a, 1);
        roster.add(b, 1);
        roster.seated.clear(); // a seat is free, but b waits for it
        assert_eq!(roster.add(c, 1), Status::Waitlist(2));
    }

    #[test]
    fn removing_forgets_and_moves_up() {
        let mut roster = Roster::default();
        let [a, b] = ids(2)[..] else { panic!() };
        roster.add(a, 1);
        roster.add(b, 1);
        roster.remove(a, 1);
        assert_eq!(roster.status(a), None);
        assert_eq!(roster.status(b), Some(Status::Seated));
        roster.cancel(b, 1);
        roster.remove(b, 1);
        assert!(roster.is_empty());
    }

    #[test]
    fn emails() {
        assert_eq!(
            Email::parse("  Ann@Acme.com ").unwrap().key(),
            "ann@acme.com"
        );
        assert_eq!(Email::parse(""), Err(FieldError::Missing));
        for bad in [
            "ann",
            "ann@",
            "@acme.com",
            "ann@acme",
            "ann@acme..com",
            "a b@acme.com",
            "ann@acme.com.",
        ] {
            assert_eq!(Email::parse(bad), Err(FieldError::NotAnEmail), "{bad}");
        }
    }

    #[test]
    fn free_mail_is_flagged() {
        for free in [
            "a@gmail.com",
            "a@GMX.de",
            "a@gmx.net",
            "a@web.de",
            "a@outlook.com",
            "a@hotmail.co.uk",
            "a@yahoo.com",
            "a@yahoo.co.uk",
            "a@icloud.com",
        ] {
            assert!(Email::parse(free).unwrap().is_free_mail(), "{free}");
        }
        for work in [
            "a@kitewell.example",
            "a@acme.de",
            "a@outlookagency.com",
            "a@gmxtra.de",
            "a@livestream.com",
        ] {
            assert!(!Email::parse(work).unwrap().is_free_mail(), "{work}");
        }
    }

    #[test]
    fn tracking_keeps_simple_tags() {
        let t = Tracking::parse(" LinkedIn ", "q4-agencies<script>");
        assert_eq!(t.source, "linkedin");
        assert_eq!(t.campaign, "q4-agenciesscript");
    }

    #[test]
    fn applicant_errors_name_each_field() {
        let form = ApplicantForm {
            first_name: " Ann ".into(),
            email: "nope".into(),
            ..ApplicantForm::default()
        };
        let errors = Applicant::parse(&form).unwrap_err();
        assert_eq!(errors.get("first_name"), None);
        assert_eq!(errors.get("last_name"), Some(&FieldError::Missing));
        assert_eq!(errors.get("email"), Some(&FieldError::NotAnEmail));
        assert_eq!(errors.get("company"), Some(&FieldError::Missing));
        assert_eq!(errors.get("job_title"), None, "optional");
    }

    #[test]
    fn consent_is_recorded_with_its_text_and_time() {
        let form = ApplicantForm {
            first_name: "Ann".into(),
            last_name: "Lee".into(),
            email: "ann@gmail.com".into(),
            company: "Acme".into(),
            consent: true,
            ..ApplicantForm::default()
        };
        let reg = Registration::new(
            SessionId::new(),
            Applicant::parse(&form).unwrap(),
            Tracking::default(),
            NOW,
        );
        assert!(reg.consent);
        assert_eq!(reg.consent_text, CONSENT_TEXT);
        assert_eq!(reg.consent_at, Some(NOW));
        assert!(reg.free_mail);
    }

    fn session_form() -> SessionForm {
        SessionForm {
            title: "Plan a quarter in 30 minutes".into(),
            description: "Mira walks through it.".into(),
            date: "2026-12-04".into(),
            time: "16:00".into(),
            minutes: "45".into(),
            speaker: "Mira Jensen".into(),
            role: "Head of Customer Education".into(),
            seats: "50".into(),
            join_link: "https://meet.kitewell.example/plan-a-quarter".into(),
        }
    }

    #[test]
    fn a_session_form_parses() {
        let input = SessionInput::parse(&session_form()).unwrap();
        assert_eq!(clock::utc_compact(input.start), "20261204T150000Z");
        assert_eq!(input.seats, 50);
    }

    #[test]
    fn a_session_cannot_be_scheduled_in_the_missing_spring_hour() {
        let form = SessionForm {
            date: "2026-03-29".into(),
            time: "02:30".into(),
            ..session_form()
        };
        let errors = SessionInput::parse(&form).unwrap_err();
        assert_eq!(errors.get("time"), Some(&FieldError::MissingBerlinTime));
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn seats_are_capped_by_the_zoom_licence() {
        let form = SessionForm {
            seats: "101".into(),
            join_link: "http://insecure.example".into(),
            date: "2026-02-30".into(),
            ..session_form()
        };
        let errors = SessionInput::parse(&form).unwrap_err();
        assert_eq!(
            errors.get("seats"),
            Some(&FieldError::OutOfRange { min: 1, max: 100 })
        );
        assert_eq!(errors.get("join_link"), Some(&FieldError::NotALink));
        assert_eq!(errors.get("date"), Some(&FieldError::NotADate));
        assert_eq!(errors.len(), 3);
    }
}
