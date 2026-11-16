//! The public handlers: the list of sessions, a session with its form, and
//! each person's own registration page. They parse the input, call the
//! usecases and turn what they return, errors included, into the page.

use askama::Template;

use super::cancel_usecase::{CancelError, cancel_uc};
use super::domain::{
    self, Applicant, ApplicantForm, CONSENT_TEXT, FormErrors, RegId, Registration, SessionId,
    Status, Tracking,
};
use super::export;
use super::register_usecase::{RegisterError, register_uc};
use super::team::team_key;
use super::ticket_usecase::{TicketError, ticket_uc};
use super::upcoming_usecase::{SessionError, session_uc, upcoming_uc};
use super::view::SessionView;
use crate::appos::{self, Request, html, page};

pub const CSS: &str = include_str!("../../templates/style.css");

#[derive(Template)]
#[template(path = "events/list.html")]
struct ListPage {
    base: String,
    /// "?utm_source=...&utm_campaign=..." to carry along, or "".
    query: String,
    sessions: Vec<SessionView>,
}

#[derive(Template)]
#[template(path = "events/session.html")]
struct SessionPage {
    base: String,
    query: String,
    s: SessionView,
    tracking: Tracking,
    form: ApplicantForm,
    errors: FormErrors,
    /// Why the registration didn't go through (already registered, ...).
    problem: Option<String>,
    consent_text: &'static str,
}

#[derive(Template)]
#[template(path = "events/registration.html")]
struct RegistrationPage {
    base: String,
    query: String,
    s: SessionView,
    reg: Registration,
    status: Status,
    seated: bool,
    position: usize,
    join_link: String,
    google: String,
    outlook: String,
    ics: String,
    ics_name: String,
}

pub fn index(req: &Request) {
    // The first visit makes the team's key and logs its address.
    team_key(req);
    let now = req.now().unix();
    let tracking = tracking_of(req.query("utm_source"), req.query("utm_campaign"));
    let t = ListPage {
        base: req.base.clone(),
        query: carry(&tracking),
        sessions: upcoming_uc(now)
            .iter()
            .map(|(s, r)| SessionView::new(s, r, now))
            .collect(),
    };
    head("Kitewell webinars");
    page::set_html("page", &html(&t));
}

pub fn session(req: &Request) {
    let Some(id) = req.param("session").and_then(SessionId::parse) else {
        return appos::not_found();
    };
    let tracking = tracking_of(req.query("utm_source"), req.query("utm_campaign"));
    show_session(
        req,
        id,
        tracking,
        ApplicantForm::default(),
        FormErrors::default(),
        None,
    );
}

pub fn register(req: &Request) {
    let Some(id) = req.param("session").and_then(SessionId::parse) else {
        return appos::not_found();
    };
    let tracking = tracking_of(req.form("utm_source"), req.form("utm_campaign"));
    let form = ApplicantForm {
        first_name: req.form("first_name").unwrap_or("").into(),
        last_name: req.form("last_name").unwrap_or("").into(),
        email: req.form("email").unwrap_or("").into(),
        company: req.form("company").unwrap_or("").into(),
        job_title: req.form("job_title").unwrap_or("").into(),
        consent: req.form("consent") == Some("yes"),
    };
    let who = match Applicant::parse(&form) {
        Ok(who) => who,
        Err(errors) => {
            page::set_status(422);
            return show_session(req, id, tracking, form, errors, None);
        }
    };
    let problem = match register_uc(id, who, tracking.clone(), req.now()) {
        Ok((reg, _)) => {
            return page::redirect(&req.url(&format!("/registration/{}/{}", reg.session, reg.id)));
        }
        Err(RegisterError::NotFound) => return appos::not_found(),
        Err(RegisterError::Unreadable(e)) => return appos::unreadable(&e),
        Err(RegisterError::AlreadyRegistered) => "You're already registered for this session.",
        Err(RegisterError::Closed) => "Registration has closed: this session has started.",
        Err(RegisterError::Cancelled) => "This session was cancelled.",
    };
    page::set_status(409);
    show_session(
        req,
        id,
        tracking,
        form,
        FormErrors::default(),
        Some(problem.into()),
    );
}

pub fn registration(req: &Request) {
    let (Some(session), Some(id)) = (
        req.param("session").and_then(SessionId::parse),
        req.param("reg").and_then(RegId::parse),
    ) else {
        return appos::not_found();
    };
    let ticket = match ticket_uc(session, id) {
        Ok(ticket) => ticket,
        Err(TicketError::NotFound) => return appos::not_found(),
        Err(TicketError::Unreadable(e)) => return appos::unreadable(&e),
    };
    let now = req.now().unix();
    let s = SessionView::new(&ticket.session, &ticket.roster, now);
    let t = RegistrationPage {
        base: req.base.clone(),
        query: String::new(),
        seated: ticket.status == Status::Seated,
        position: match ticket.status {
            Status::Waitlist(n) => n,
            Status::Seated | Status::Cancelled => 0,
        },
        join_link: ticket.session.join_link.clone(),
        google: export::google_calendar(&ticket.session),
        outlook: export::outlook_calendar(&ticket.session),
        ics: export::data_url(
            "text/calendar",
            &export::ics(&ticket.session, &ticket.reg, now),
        ),
        ics_name: format!("kitewell-webinar-{}.ics", ticket.session.date),
        status: ticket.status,
        reg: ticket.reg,
        s,
    };
    head(&format!("Your registration: {}", ticket.session.title));
    // The page holds the Zoom link: keep it out of search engines.
    page::add_header(r#"<meta name="robots" content="noindex">"#);
    page::set_html("page", &html(&t));
}

pub fn cancel(req: &Request) {
    let (Some(session), Some(id)) = (
        req.param("session").and_then(SessionId::parse),
        req.param("reg").and_then(RegId::parse),
    ) else {
        return appos::not_found();
    };
    match cancel_uc(session, id, req.now()) {
        // The page shows what happened, the session's start too.
        Ok(()) | Err(CancelError::Started) => {
            page::redirect(&req.url(&format!("/registration/{session}/{id}")))
        }
        Err(CancelError::NotFound) => appos::not_found(),
        Err(CancelError::Unreadable(e)) => appos::unreadable(&e),
    }
}

fn show_session(
    req: &Request,
    id: SessionId,
    tracking: Tracking,
    form: ApplicantForm,
    errors: FormErrors,
    problem: Option<String>,
) {
    let (session, roster) = match session_uc(id) {
        Ok(found) => found,
        Err(SessionError::NotFound) => return appos::not_found(),
        Err(SessionError::Unreadable(e)) => return appos::unreadable(&e),
    };
    let t = SessionPage {
        base: req.base.clone(),
        query: carry(&tracking),
        s: SessionView::new(&session, &roster, req.now().unix()),
        tracking,
        form,
        errors,
        problem,
        consent_text: CONSENT_TEXT,
    };
    head(&format!("{}: Kitewell webinar", session.title));
    page::set_html("page", &html(&t));
}

/// The design and the title of a page.
pub fn head(title: &str) {
    appos::style(CSS);
    appos::title(title);
}

fn tracking_of(source: Option<&str>, campaign: Option<&str>) -> Tracking {
    Tracking::parse(source.unwrap_or(""), campaign.unwrap_or(""))
}

/// The query string that keeps source and campaign on the next page.
fn carry(tracking: &Tracking) -> String {
    let mut out = String::new();
    for (key, value) in [
        ("utm_source", &tracking.source),
        ("utm_campaign", &tracking.campaign),
    ] {
        if !value.is_empty() {
            out.push(if out.is_empty() { '?' } else { '&' });
            out.push_str(key);
            out.push('=');
            appos::encode_into(value, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::{Roster, Session};
    use crate::events::testing::{NOW, applicant, session_input};

    fn view(seats_taken: usize) -> SessionView {
        let session = Session::new(session_input(), NOW);
        let mut roster = Roster::default();
        for _ in 0..seats_taken {
            roster.add(RegId::new(), session.seats);
        }
        SessionView::new(&session, &roster, NOW.unix())
    }

    fn session_page(s: SessionView, errors: FormErrors, problem: Option<String>) -> String {
        SessionPage {
            base: "/webinars".into(),
            query: "?utm_source=linkedin".into(),
            s,
            tracking: Tracking::parse("linkedin", "q4-agencies"),
            form: ApplicantForm {
                email: "ann@acme.com".into(),
                ..ApplicantForm::default()
            },
            errors,
            problem,
            consent_text: CONSENT_TEXT,
        }
        .render()
        .unwrap()
    }

    #[test]
    fn the_list_shows_seats_and_three_cities_but_no_join_link() {
        let page = ListPage {
            base: "/webinars".into(),
            query: String::new(),
            sessions: vec![view(38), view(50)],
        }
        .render()
        .unwrap();
        assert!(page.contains("12 seats left"));
        assert!(page.contains("Full: join the waitlist"));
        assert!(page.contains("16:00") && page.contains("15:00") && page.contains("10:00"));
        assert!(page.contains("New York"));
        assert!(!page.contains("meet.kitewell.example"));
    }

    #[test]
    fn the_form_keeps_tracking_and_never_ticks_consent() {
        let page = session_page(view(0), FormErrors::default(), None);
        assert!(page.contains(r#"name="utm_source" value="linkedin""#));
        assert!(page.contains(r#"name="utm_campaign" value="q4-agencies""#));
        assert!(page.contains(CONSENT_TEXT));
        assert!(!page.contains("checked"));
        assert!(!page.contains("meet.kitewell.example"));
    }

    #[test]
    fn errors_show_next_to_the_fields_with_what_was_typed() {
        let errors = Applicant::parse(&ApplicantForm {
            email: "nope".into(),
            ..ApplicantForm::default()
        })
        .unwrap_err();
        let page = session_page(
            view(0),
            errors,
            Some("You're already registered for this session.".into()),
        );
        assert!(page.contains("look like an email address."));
        assert!(page.contains(r#"value="ann@acme.com""#));
        assert!(page.contains("already registered for this session."));
    }

    fn registration_page(status: Status) -> String {
        let session = Session::new(session_input(), NOW);
        let reg = Registration::new(
            session.id,
            applicant("ann@acme.com"),
            Tracking::default(),
            NOW,
        );
        RegistrationPage {
            base: "/webinars".into(),
            query: String::new(),
            s: SessionView::new(&session, &Roster::default(), NOW.unix()),
            seated: status == Status::Seated,
            position: if let Status::Waitlist(n) = status {
                n
            } else {
                0
            },
            status,
            join_link: session.join_link.clone(),
            google: export::google_calendar(&session),
            outlook: export::outlook_calendar(&session),
            ics: export::data_url("text/calendar", &export::ics(&session, &reg, 0)),
            ics_name: "x.ics".into(),
            reg,
        }
        .render()
        .unwrap()
    }

    #[test]
    fn a_seat_shows_the_join_link_and_calendars() {
        let page = registration_page(Status::Seated);
        assert!(page.contains("You're in"));
        assert!(page.contains("https://meet.kitewell.example/plan-a-quarter"));
        assert!(page.contains("calendar.google.com"));
        assert!(page.contains("outlook.office.com"));
        assert!(page.contains("data:text/calendar"));
        assert!(page.contains("Cancel your seat"));
    }

    #[test]
    fn the_waitlist_shows_the_place_and_no_join_link() {
        let page = registration_page(Status::Waitlist(3));
        assert!(page.contains("number 3 on the waitlist"));
        assert!(!page.contains("meet.kitewell.example"));
        assert!(page.contains("Leave the waitlist"));
    }

    #[test]
    fn carry_encodes() {
        assert_eq!(
            carry(&Tracking::parse("linkedin", "q4 agencies")),
            "?utm_source=linkedin&utm_campaign=q4%20agencies"
        );
        assert_eq!(carry(&Tracking::default()), "");
    }
}
