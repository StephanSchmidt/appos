//! The desk's handlers: parse the input, call usecases with the parsed
//! values, and turn what they return, errors included, into the page.
//!
//! Who is looking comes from the path (/as/<id>/...): there are no logins
//! yet, and the shell gives apps no cookies, so the choice travels in every
//! link.

use super::close_usecase::{CloseError, close_uc};
use super::decide_usecase::{DecideError, decide_uc};
use super::domain::{
    DiscountRequest, FormErrors, Outcome, Status, Terms, TermsInput, Verdict, parse_comment,
};
use super::list_usecase::list_uc;
use super::people::{self, Person, Rep, Role};
use super::resubmit_usecase::{ResubmitError, resubmit_uc};
use super::show_usecase::{ShowError, show_uc};
use super::stats;
use super::submit_usecase::submit_uc;
use super::time;
use super::views::{self, Filter, FormMode, NoticePage, Tab};
use crate::appos::{self, Request, html, page};

const CSS: &str = include_str!("../../templates/style.css");

fn show(title: &str, body: &str) {
    appos::style(CSS);
    appos::title(&format!("{title} · Discount desk"));
    page::set_html("page", body);
}

/// The person in the path.
fn me(req: &Request) -> Option<&'static Person> {
    req.param("who").and_then(people::find)
}

/// A request number from the path or a query: digits only.
fn parse_number(text: &str) -> Option<u32> {
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn number(req: &Request) -> Option<u32> {
    req.param("n").and_then(parse_number)
}

fn now(req: &Request) -> i64 {
    req.now().unix()
}

/// The time to store: RFC 3339 text, to the second.
fn stamp(req: &Request) -> String {
    req.now().to_string()
}

/// "2026-11-13", on the platform's clock.
fn today(req: &Request) -> String {
    req.now().date().to_string()
}

fn request_url(req: &Request, me: &Person, n: u32) -> String {
    req.url(&format!("/as/{}/r/{n}", me.id))
}

/// A page saying why this person can't be here, with a way back.
fn notice(req: &Request, me: &Person, status: u16, heading: &str, text: &str) {
    page::set_status(status);
    let all = list_uc();
    let t = NoticePage {
        frame: views::frame(&req.base, me, &all, now(req), Tab::Other, ""),
        heading: heading.into(),
        text: text.into(),
        link_url: req.url(&format!("/as/{}", me.id)),
        link_label: "Back to my page".into(),
    };
    show(heading, &html(&t));
}

// ---- Who are you -----------------------------------------------------------

pub fn pick(req: &Request) {
    show("Who are you?", &html(&views::pick_page(&req.base)));
}

/// The "You are" picker: the same page as the new person, where that makes
/// sense (a request, the overview), else their home.
pub fn switch(req: &Request) {
    let Some(me) = req.query("who").and_then(people::find) else {
        return page::redirect(&req.url("/"));
    };
    let to = req.query("to").unwrap_or("");
    let rest = match to.strip_prefix("r/").and_then(parse_number) {
        Some(n) => format!("/r/{n}"),
        None if to == "all" && me.sees_all() => "/all".into(),
        None if to == "new" && Rep::of(me).is_some() => "/new".into(),
        None => String::new(),
    };
    page::redirect(&req.url(&format!("/as/{}{rest}", me.id)));
}

// ---- Home ------------------------------------------------------------------

pub fn home(req: &Request) {
    let Some(me) = me(req) else {
        return appos::not_found();
    };
    if me.role == Role::Ops {
        return page::redirect(&req.url(&format!("/as/{}/all", me.id)));
    }
    let all = list_uc();
    let frame = views::frame(&req.base, me, &all, now(req), Tab::Home, "");
    let t = views::home_page(frame, me, &all, now(req));
    show(&t.heading, &html(&t));
}

// ---- All requests and the quarter --------------------------------------------

pub fn all(req: &Request) {
    let Some(me) = me(req) else {
        return appos::not_found();
    };
    if !me.sees_all() {
        return notice(
            req,
            me,
            403,
            "All requests are for Hannah and Marcus",
            "Sales ops and the VP see every request and the quarter's numbers.",
        );
    }
    let all = list_uc();
    let now = now(req);
    let current = time::quarter(&today(req)).unwrap_or((2026, 4));
    let (y, q) = req
        .query("q")
        .and_then(stats::parse_quarter)
        .unwrap_or(current);
    // Filters that aren't a status or a rep show everything, as "Any" does.
    let filter = Filter {
        status: req.query("status").and_then(Status::parse),
        rep: req
            .query("rep")
            .and_then(people::find)
            .filter(|p| p.role == Role::Rep),
    };
    let stats = stats::quarter_stats(&all, y, q, now);
    let frame = views::frame(&req.base, me, &all, now, Tab::All, "all");
    let t = views::all_page(frame, &all, stats, current, filter, now);
    show("All requests", &html(&t));
}

// ---- One request -------------------------------------------------------------

fn request_page(req: &Request, me: &Person, r: &DiscountRequest, error: String, comment: String) {
    let all = list_uc();
    let frame = views::frame(
        &req.base,
        me,
        &all,
        now(req),
        Tab::Other,
        &format!("r/{}", r.number),
    );
    let t = views::request_page(frame, me, r, now(req), error, comment);
    show(
        &format!("{} {}", r.reference(), r.terms.customer),
        &html(&t),
    );
}

/// The request page again, with why the last try didn't work.
fn request_again(req: &Request, me: &Person, n: u32, status: u16, error: String) {
    match show_uc(n) {
        Ok(r) => {
            page::set_status(status);
            let typed = req.form("comment").unwrap_or("").to_string();
            request_page(req, me, &r, error, typed);
        }
        Err(ShowError::NotFound) => appos::not_found(),
        Err(ShowError::Unreadable(e)) => appos::unreadable(&e),
    }
}

pub fn request(req: &Request) {
    let (Some(me), Some(n)) = (me(req), number(req)) else {
        return appos::not_found();
    };
    match show_uc(n) {
        Ok(r) => request_page(req, me, &r, String::new(), String::new()),
        Err(ShowError::NotFound) => appos::not_found(),
        Err(ShowError::Unreadable(e)) => appos::unreadable(&e),
    }
}

pub fn decide(req: &Request) {
    let (Some(me), Some(n)) = (me(req), number(req)) else {
        return appos::not_found();
    };
    let verdict = match Verdict::parse(
        req.form("decision").unwrap_or(""),
        req.form("comment").unwrap_or(""),
    ) {
        Ok(verdict) => verdict,
        Err(error) => return request_again(req, me, n, 422, error.to_string()),
    };
    match decide_uc(n, me, verdict, &stamp(req)) {
        Ok(_) => page::redirect(&request_url(req, me, n)),
        Err(DecideError::NotFound) => appos::not_found(),
        Err(DecideError::Unreadable(e)) => appos::unreadable(&e),
        Err(DecideError::NotYourTurn) => request_again(
            req,
            me,
            n,
            409,
            "This request isn't waiting for your decision (any more). Here is where it is now."
                .into(),
        ),
    }
}

pub fn close(req: &Request) {
    let (Some(me), Some(n)) = (me(req), number(req)) else {
        return appos::not_found();
    };
    let Some(outcome) = req.form("outcome").and_then(Outcome::parse) else {
        return request_again(req, me, n, 422, "Pick won, lost or withdraw.".into());
    };
    let Some(comment) = parse_comment(req.form("comment").unwrap_or("")) else {
        return request_again(
            req,
            me,
            n,
            422,
            "Keep the comment to 1000 characters.".into(),
        );
    };
    match close_uc(n, me, outcome, comment, &stamp(req)) {
        Ok(_) => page::redirect(&request_url(req, me, n)),
        Err(CloseError::NotFound) => appos::not_found(),
        Err(CloseError::Unreadable(e)) => appos::unreadable(&e),
        Err(CloseError::OtherRep) => request_again(req, me, n, 409, "Only the rep who asked can withdraw a request or close the deal.".into()),
        Err(CloseError::NotOpen) => request_again(req, me, n, 409, "That isn't possible any more: see where the request is now.".into()),
        Err(CloseError::Expired) => request_again(
            req,
            me,
            n,
            409,
            "The approval has expired, so the deal can't be marked won on it. Ask again with these terms.".into(),
        ),
    }
}

// ---- New request, or change one ------------------------------------------------

/// The form's fields as typed.
fn form_input(req: &Request) -> TermsInput {
    let f = |name| req.form(name).unwrap_or("").to_string();
    TermsInput {
        customer: f("customer"),
        link: f("link"),
        plan: f("plan"),
        seats: f("seats"),
        years: f("years"),
        discount: f("discount"),
        billing: f("billing"),
        payment: f("payment"),
        competitor: f("competitor"),
        close_date: f("close_date"),
        justification: f("justification"),
    }
}

fn form(
    req: &Request,
    me: &Person,
    rep: Rep,
    mode: FormMode,
    input: TermsInput,
    errors: FormErrors,
    note: String,
) {
    let all = list_uc();
    let here = match mode {
        FormMode::New => "new".to_string(),
        FormMode::Change(n) => format!("r/{n}"),
    };
    let tab = match mode {
        FormMode::New => Tab::New,
        FormMode::Change(_) => Tab::Other,
    };
    let frame = views::frame(&req.base, me, &all, now(req), tab, &here);
    let t = views::form_page(frame, rep, mode, input, errors, note, &today(req));
    show(&t.heading, &html(&t));
}

fn only_reps(req: &Request, me: &Person) {
    notice(
        req,
        me,
        403,
        "Only reps ask for discounts",
        "Pick a rep under \"You are\" to ask for one.",
    );
}

pub fn new_request(req: &Request) {
    let Some(me) = me(req) else {
        return appos::not_found();
    };
    let Some(rep) = Rep::of(me) else {
        return only_reps(req, me);
    };
    // "Ask again with these terms" after an approval expired.
    let input = match req.query("from").and_then(parse_number) {
        Some(n) => match show_uc(n) {
            Ok(r) if r.rep == me.id => {
                let mut input = TermsInput::of(&r.terms);
                if input.close_date < today(req) {
                    input.close_date.clear();
                }
                input
            }
            Ok(_) => TermsInput::blank(),
            Err(ShowError::NotFound) => TermsInput::blank(),
            Err(ShowError::Unreadable(e)) => return appos::unreadable(&e),
        },
        None => TermsInput::blank(),
    };
    form(
        req,
        me,
        rep,
        FormMode::New,
        input,
        FormErrors::default(),
        String::new(),
    );
}

pub fn create(req: &Request) {
    let Some(me) = me(req) else {
        return appos::not_found();
    };
    let Some(rep) = Rep::of(me) else {
        return only_reps(req, me);
    };
    let input = form_input(req);
    match Terms::parse(&input, &today(req)) {
        Ok(terms) => match submit_uc(rep, terms, &stamp(req)) {
            Ok(r) => page::redirect(&request_url(req, me, r.number)),
            Err(e) => appos::unreadable(&e),
        },
        Err(errors) => {
            page::set_status(422);
            form(req, me, rep, FormMode::New, input, errors, String::new());
        }
    }
}

/// htmx, while the rep types: the numbers and who approves.
pub fn quote(req: &Request) {
    let (Some(me), true) = (me(req), req.htmx) else {
        return appos::not_found();
    };
    let Some(rep) = Rep::of(me) else {
        return appos::not_found();
    };
    let frame = views::frame(&req.base, me, &[], now(req), Tab::New, "");
    let t = views::form_page(
        frame,
        rep,
        FormMode::New,
        form_input(req),
        FormErrors::default(),
        String::new(),
        &today(req),
    );
    page::set_html("quote", &html(&t.as_quote()));
}

pub fn change_form(req: &Request) {
    let (Some(me), Some(n)) = (me(req), number(req)) else {
        return appos::not_found();
    };
    let Some(rep) = Rep::of(me) else {
        return only_reps(req, me);
    };
    match show_uc(n) {
        Ok(r) if r.rep != me.id => request_again(
            req,
            me,
            n,
            409,
            "Only the rep who asked can change a request.".into(),
        ),
        Ok(r) if r.status(now(req)) != Status::SentBack => request_again(
            req,
            me,
            n,
            409,
            "Only a request that was sent back can be changed.".into(),
        ),
        Ok(r) => form(
            req,
            me,
            rep,
            FormMode::Change(n),
            TermsInput::of(&r.terms),
            FormErrors::default(),
            String::new(),
        ),
        Err(ShowError::NotFound) => appos::not_found(),
        Err(ShowError::Unreadable(e)) => appos::unreadable(&e),
    }
}

pub fn resubmit(req: &Request) {
    let (Some(me), Some(n)) = (me(req), number(req)) else {
        return appos::not_found();
    };
    let Some(rep) = Rep::of(me) else {
        return only_reps(req, me);
    };
    let input = form_input(req);
    let typed_note = req.form("note").unwrap_or("").to_string();
    let (terms, note) = match (
        Terms::parse(&input, &today(req)),
        parse_comment(&typed_note),
    ) {
        (Ok(terms), Some(note)) => (terms, note),
        (Ok(_), None) | (Err(_), None) | (Err(_), Some(_)) => {
            let errors = Terms::parse(&input, &today(req)).err().unwrap_or_default();
            page::set_status(422);
            return form(req, me, rep, FormMode::Change(n), input, errors, typed_note);
        }
    };
    match resubmit_uc(n, rep, terms, note, &stamp(req)) {
        Ok(_) => page::redirect(&request_url(req, me, n)),
        Err(ResubmitError::NotFound) => appos::not_found(),
        Err(ResubmitError::Unreadable(e)) => appos::unreadable(&e),
        Err(ResubmitError::OtherRep) => request_again(
            req,
            me,
            n,
            409,
            "Only the rep who asked can change a request.".into(),
        ),
        Err(ResubmitError::NotSentBack) => request_again(
            req,
            me,
            n,
            409,
            "Only a request that was sent back can be changed.".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_allocated_number_can_be_opened() {
        for n in [1, 999_999_999, 1_000_000_000, u32::MAX] {
            assert_eq!(parse_number(&n.to_string()), Some(n));
        }
        for invalid in ["", "-1", "+1", "4294967296", "10000000000"] {
            assert_eq!(parse_number(invalid), None, "{invalid}");
        }
    }
}
