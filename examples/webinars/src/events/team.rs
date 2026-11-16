//! The events team's handlers: sessions (add, edit, cancel), their numbers
//! and the attendee list. There is no sign-in yet, so these pages live
//! under a secret address, /team/<key>; any other key is a 404. The key
//! is made on the app's first request and kept in kv, never in the source.

use askama::Template;

use super::cancel_session_usecase::{CancelSessionError, cancel_session_uc};
use super::delete_usecase::{
    DeleteRegistrationError, DeleteSessionError, delete_registration_uc, delete_session_uc,
};
use super::domain::{self, FormErrors, RegId, SessionForm, SessionId, SessionInput, Status};
use super::export::{self, Row};
use super::report_usecase::{Count, ReportError, overview_uc, report_uc};
use super::save_session_usecase::{SaveError, create_uc, update_uc};
use super::team_key_usecase::team_key_uc;
use super::upcoming_usecase::{SessionError, session_uc};
use super::view::SessionView;
use super::web::head;
use crate::appos::{self, Request, html, log, page};

/// The secret part of the team's address. When it is made, its address is
/// logged, for whoever runs the shell to pass on to the events team.
pub fn team_key(req: &Request) -> String {
    let (key, made) = team_key_uc();
    if made {
        log::info(&format!(
            "the events team's page: {}",
            req.absolute_url(&format!("/team/{key}"))
        ));
    }
    key
}

#[derive(Template)]
#[template(path = "events/team/home.html")]
struct HomePage {
    base: String,
    team: String,
    origin: String,
    sessions: Vec<SessionView>,
    notice: Option<String>,
}

#[derive(Template)]
#[template(path = "events/team/form.html")]
struct FormPage {
    team: String,
    action: String,
    editing: bool,
    form: SessionForm,
    errors: FormErrors,
    problem: Option<String>,
}

/// One attendee, ready to show.
struct AttendeeRow {
    id: RegId,
    status: Status,
    name: String,
    email: String,
    free_mail: bool,
    company: String,
    job_title: String,
    consent: bool,
    consent_at: String,
    registered_at: String,
    source: String,
    campaign: String,
}

#[derive(Template)]
#[template(path = "events/team/report.html")]
struct ReportPage {
    base: String,
    team: String,
    s: SessionView,
    notice: Option<String>,
    free_mail: usize,
    by_source: Vec<Count>,
    by_campaign: Vec<Count>,
    rows: Vec<AttendeeRow>,
    csv: String,
    csv_name: String,
    tsv: String,
}

/// The team's base address, or None (and a 404) for a wrong key.
fn team(req: &Request) -> Option<String> {
    let key = team_key(req);
    if req.param("key") == Some(key.as_str()) {
        Some(req.url(&format!("/team/{key}")))
    } else {
        appos::not_found();
        None
    }
}

fn private_head(title: &str) {
    head(title);
    page::add_header(r#"<meta name="robots" content="noindex">"#);
}

pub fn home(req: &Request) {
    let Some(team) = team(req) else { return };
    let now = req.now().unix();
    let notice = match req.query("done") {
        Some("deleted") => Some("The session was deleted.".to_string()),
        _ => None,
    };
    let t = HomePage {
        base: req.base.clone(),
        origin: req.origin.clone(),
        team,
        sessions: overview_uc(now)
            .iter()
            .map(|(s, r)| SessionView::new(s, r, now))
            .collect(),
        notice,
    };
    private_head("Webinars: events team");
    page::set_html("page", &html(&t));
}

pub fn new_session(req: &Request) {
    let Some(team) = team(req) else { return };
    show_form(
        &team,
        None,
        SessionForm::blank(),
        FormErrors::default(),
        None,
    );
}

pub fn create(req: &Request) {
    let Some(team) = team(req) else { return };
    let form = session_form(req);
    match SessionInput::parse(&form) {
        Ok(input) => {
            let session = create_uc(input, req.now());
            page::redirect(&format!("{team}/sessions/{}?done=added", session.id));
        }
        Err(errors) => {
            page::set_status(422);
            show_form(&team, None, form, errors, None);
        }
    }
}

pub fn edit(req: &Request) {
    let Some(team) = team(req) else { return };
    let Some(id) = req.param("session").and_then(SessionId::parse) else {
        return appos::not_found();
    };
    match session_uc(id) {
        Ok((session, _)) => show_form(
            &team,
            Some(id),
            SessionForm::of(&session),
            FormErrors::default(),
            None,
        ),
        Err(SessionError::NotFound) => appos::not_found(),
        Err(SessionError::Unreadable(e)) => appos::unreadable(&e),
    }
}

pub fn update(req: &Request) {
    let Some(team) = team(req) else { return };
    let Some(id) = req.param("session").and_then(SessionId::parse) else {
        return appos::not_found();
    };
    let form = session_form(req);
    let input = match SessionInput::parse(&form) {
        Ok(input) => input,
        Err(errors) => {
            page::set_status(422);
            return show_form(&team, Some(id), form, errors, None);
        }
    };
    match update_uc(id, input, req.now()) {
        Ok(_) => page::redirect(&format!("{team}/sessions/{id}?done=saved")),
        Err(SaveError::NotFound) => appos::not_found(),
        Err(SaveError::Unreadable(e)) => appos::unreadable(&e),
        Err(SaveError::SeatsTaken { taken }) => {
            page::set_status(409);
            let problem = format!(
                "{taken} people already have a seat, so the session needs at least {taken} seats. To free seats, cancel or delete registrations first."
            );
            show_form(&team, Some(id), form, FormErrors::default(), Some(problem));
        }
    }
}

pub fn report(req: &Request) {
    let Some(team) = team(req) else { return };
    let Some(id) = req.param("session").and_then(SessionId::parse) else {
        return appos::not_found();
    };
    let report = match report_uc(id) {
        Ok(report) => report,
        Err(ReportError::NotFound) => return appos::not_found(),
        Err(ReportError::Unreadable(e)) => return appos::unreadable(&e),
    };
    let notice = match req.query("done") {
        Some("added") => Some("The session was added. It's on the public page now."),
        Some("saved") => Some("Changes saved."),
        Some("cancelled") => Some("The session was cancelled."),
        Some("deleted") => Some("The registration was deleted."),
        _ => None,
    };
    let export_rows: Vec<Row> = report
        .rows
        .iter()
        .map(|(reg, status)| Row {
            reg,
            status: *status,
        })
        .collect();
    let rows = report
        .rows
        .iter()
        .map(|(r, status)| AttendeeRow {
            id: r.id,
            status: *status,
            name: format!("{} {}", r.first_name, r.last_name),
            email: r.email.clone(),
            free_mail: r.free_mail,
            company: r.company.clone(),
            job_title: r.job_title.clone(),
            consent: r.consent,
            consent_at: r.consent_at.map(export::berlin_stamp).unwrap_or_default(),
            registered_at: export::berlin_stamp(r.registered_at),
            source: r.source.clone(),
            campaign: r.campaign.clone(),
        })
        .collect();
    let t = ReportPage {
        base: req.base.clone(),
        team,
        s: SessionView::new(&report.session, &report.roster, req.now().unix()),
        notice: notice.map(String::from),
        free_mail: report.rows.iter().filter(|(r, _)| r.free_mail).count(),
        by_source: report.by_source,
        by_campaign: report.by_campaign,
        csv: export::data_url("text/csv", &export::csv(&export_rows)),
        csv_name: format!("webinar-{}-attendees.csv", report.session.date),
        tsv: export::tsv(&export_rows),
        rows,
    };
    private_head(&format!("{}: attendees", report.session.title));
    page::set_html("page", &html(&t));
}

pub fn cancel_session(req: &Request) {
    let Some(team) = team(req) else { return };
    let Some(id) = req.param("session").and_then(SessionId::parse) else {
        return appos::not_found();
    };
    match cancel_session_uc(id, req.now()) {
        Ok(_) => page::redirect(&format!("{team}/sessions/{id}?done=cancelled")),
        Err(CancelSessionError::NotFound) => appos::not_found(),
        Err(CancelSessionError::Unreadable(e)) => appos::unreadable(&e),
    }
}

pub fn delete_session(req: &Request) {
    let Some(team) = team(req) else { return };
    let Some(id) = req.param("session").and_then(SessionId::parse) else {
        return appos::not_found();
    };
    match delete_session_uc(id) {
        Ok(_) => page::redirect(&format!("{team}?done=deleted")),
        Err(DeleteSessionError::NotFound) => appos::not_found(),
        Err(DeleteSessionError::Unreadable(e)) => appos::unreadable(&e),
        // The page offers delete only without registrations; someone
        // registered in between. The page shows them.
        Err(DeleteSessionError::HasRegistrations) => {
            page::redirect(&format!("{team}/sessions/{id}"))
        }
    }
}

pub fn delete_registration(req: &Request) {
    let Some(team) = team(req) else { return };
    let (Some(session), Some(id)) = (
        req.param("session").and_then(SessionId::parse),
        req.param("reg").and_then(RegId::parse),
    ) else {
        return appos::not_found();
    };
    match delete_registration_uc(session, id) {
        Ok(_) => page::redirect(&format!("{team}/sessions/{session}?done=deleted")),
        // Deleted already (a second click): the list shows it's gone.
        Err(DeleteRegistrationError::NotFound) => {
            page::redirect(&format!("{team}/sessions/{session}"))
        }
        Err(DeleteRegistrationError::Unreadable(e)) => appos::unreadable(&e),
    }
}

fn session_form(req: &Request) -> SessionForm {
    let field = |name: &str| req.form(name).unwrap_or("").to_string();
    SessionForm {
        title: field("title"),
        description: field("description"),
        date: field("date"),
        time: field("time"),
        minutes: field("minutes"),
        speaker: field("speaker"),
        role: field("role"),
        seats: field("seats"),
        join_link: field("join_link"),
    }
}

fn show_form(
    team: &str,
    id: Option<SessionId>,
    form: SessionForm,
    errors: FormErrors,
    problem: Option<String>,
) {
    let t = FormPage {
        team: team.into(),
        action: match id {
            Some(id) => format!("{team}/sessions/{id}"),
            None => format!("{team}/sessions"),
        },
        editing: id.is_some(),
        form,
        errors,
        problem,
    };
    private_head(if id.is_some() {
        "Edit session"
    } else {
        "Add a session"
    });
    page::set_html("page", &html(&t));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::{Registration, Roster, Session, Tracking};
    use crate::events::testing::{NOW, applicant, session_input};

    #[test]
    fn the_report_lists_people_with_consent_and_flags() {
        let session = Session::new(session_input(), NOW);
        let mut reg = Registration::new(
            session.id,
            applicant("ann@gmail.com"),
            Tracking::parse("linkedin", "q4-agencies"),
            NOW,
        );
        reg.consent = true;
        reg.consent_at = Some(NOW);
        let mut roster = Roster::default();
        roster.add(reg.id, 50);
        let rows = [Row {
            reg: &reg,
            status: Status::Seated,
        }];
        let t = ReportPage {
            base: "/webinars".into(),
            team: "/webinars/team/x".into(),
            s: SessionView::new(&session, &roster, NOW.unix()),
            notice: None,
            free_mail: 1,
            by_source: vec![Count {
                name: "linkedin".into(),
                registered: 1,
                ..Count::default()
            }],
            by_campaign: vec![],
            rows: vec![AttendeeRow {
                id: reg.id,
                status: Status::Seated,
                name: "Ann Lee".into(),
                email: reg.email.clone(),
                free_mail: true,
                company: "Acme".into(),
                job_title: String::new(),
                consent: true,
                consent_at: export::berlin_stamp(NOW),
                registered_at: export::berlin_stamp(NOW),
                source: "linkedin".into(),
                campaign: "q4-agencies".into(),
            }],
            csv: export::data_url("text/csv", &export::csv(&rows)),
            csv_name: "a.csv".into(),
            tsv: export::tsv(&rows),
        };
        let page = t.render().unwrap();
        assert!(page.contains("1 <small"));
        assert!(page.contains("free mail"));
        assert!(page.contains("yes, 2026-11-13 10:00"));
        assert!(page.contains("data:text/csv"));
        assert!(page.contains("q4-agencies"));
    }

    #[test]
    fn the_form_shows_problems() {
        let t = FormPage {
            team: "/webinars/team/x".into(),
            action: "/webinars/team/x/sessions".into(),
            editing: false,
            form: SessionForm::blank(),
            errors: SessionInput::parse(&SessionForm::blank()).unwrap_err(),
            problem: None,
        };
        let page = t.render().unwrap();
        assert!(page.contains(r#"value="50""#));
        assert!(page.contains("Please fill this in."));
    }
}
