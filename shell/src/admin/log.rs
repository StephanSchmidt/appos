//! The log: what happened to every app and to the platform, newest first,
//! kept for good. A row opens (`<details>`, no scripts) to the whole text,
//! and for a limit hit to the requests behind it. Filters are a GET form,
//! so a narrowed log is a link.

use chrono::NaiveDate;

use crate::kit::Reply;
use crate::log_filter::{Filters, PLATFORM, Pick};
use crate::platform::{Actor, Event, Kind, Severity, now_ms};
use crate::shell::Shell;
use crate::usage::{self, KEEP};

use super::format::{FULL, capitalize, duration, first_line, local, plural, span, thousands};
use super::model::{App, Tone, by, icon, kind_label, load, message, state, word};
use super::style::{badge, icon_html, path_icon, risk_chip};
use super::{Ask, Viewer, html, page, plain, read};

/// Events on a page; older ones are a link away.
const PAGE: u32 = 100;
/// Requests listed behind a limit hit.
const BEHIND: u32 = 20;

/// `GET /admin/log`: `?q=&app=&kind=&severity=&by=&from=&to=` narrow it,
/// `before=<id>` pages back.
pub(super) async fn log_page(ask: Ask) -> Reply {
    let (filters, before) = match Filters::parse(&ask.req.query) {
        Ok(parsed) => parsed,
        Err(e) => return plain(400, e),
    };
    let Ask { shell, viewer, .. } = ask;
    read(shell, move |shell| log_html(shell, &viewer, &filters, before)).await
}

fn log_html(shell: &Shell, viewer: &Viewer, filters: &Filters, before: Option<i64>) -> Reply {
    let now = now_ms();
    let (apps, _) = load(shell, now);
    let mut events = shell.platform.log(&filters.query(before), PAGE + 1).unwrap_or_default();
    let older = (events.len() > PAGE as usize).then(|| events[PAGE as usize - 1].id);
    events.truncate(PAGE as usize);
    let filtered = *filters != Filters::default();
    let list = match (events.is_empty(), filtered, before.is_some()) {
        (false, ..) => List::Days(days(shell, &apps, &events, now)),
        (true, false, false) => List::Fresh,
        (true, false, true) => List::NothingOlder,
        (true, true, older) => List::NoMatch(if older { "older " } else { "" }),
    };
    let body = html(&Log {
        form: (!events.is_empty() || filtered || before.is_some()).then(|| form(filters, &apps)),
        list,
        newest: before.map(|_| href(filters, None)),
        older: older.map(|id| href(filters, Some(id))),
    });
    Reply::document(page(shell, viewer, state(&apps), "/admin/log", &format!("Log · {} admin", shell.name), &body, None))
}

/// The page's card (templates/admin/log.html): the filters, the events by
/// day, and links to the newest and older pages.
#[derive(askama::Template)]
#[template(path = "admin/log.html")]
struct Log {
    form: Option<Form>,
    list: List,
    newest: Option<String>,
    older: Option<String>,
}

enum List {
    Days(Vec<Day>),
    /// Nothing has happened yet.
    Fresh,
    NothingOlder,
    /// No event matches the filters; "older " on a later page.
    NoMatch(&'static str),
}

struct Day {
    name: String,
    rows: Vec<Row>,
}

/// One event; it opens to the whole reason, the facts and the requests
/// behind it.
struct Row {
    id: i64,
    full: String,
    time: String,
    tone: &'static str,
    icon: String,
    word: String,
    app: Who,
    message: String,
    reason: Option<String>,
    label: String,
    severity: String,
    by: String,
    /// The reason, when it is longer than its first line.
    whole: Option<String>,
    when: String,
    risk: Option<String>,
    recovery: Option<String>,
    behind: Behind,
}

enum Who {
    /// Its id, icon and name.
    App(String, String, String),
    Platform,
    /// An app no longer here.
    Unknown(String),
}

/// The failed requests behind a limit hit or an automatic rollback.
enum Behind {
    Nothing,
    /// No longer kept: how many platform.db keeps.
    Gone(String),
    /// What they are, how many of them show when not all, and those.
    Listed(String, Option<u32>, Vec<Req>),
}

struct Req {
    time: String,
    route: String,
    took: String,
    error: String,
}

/// The filters in a row above the table, a label over each.
struct Form {
    text: String,
    selects: Vec<Select>,
    dates: Vec<(&'static str, &'static str, String)>,
    clear: bool,
}

/// A select's name, label and options, and a group of options after them.
struct Select {
    name: &'static str,
    label: &'static str,
    options: Vec<Opt>,
    group: Option<(&'static str, Vec<Opt>)>,
}

struct Opt {
    value: String,
    label: String,
    on: bool,
}

/// The log of one app.
pub(super) fn of_app(id: &str) -> String {
    href(&Filters { app: Some(id.to_string()), ..Default::default() }, None)
}

/// The log of some kinds of events.
pub(super) fn of_kinds(pick: Pick) -> String {
    href(&Filters { pick: Some(pick), ..Default::default() }, None)
}

/// The log of some kinds of events of one app.
pub(super) fn of_app_kinds(id: &str, pick: Pick) -> String {
    href(&Filters { app: Some(id.to_string()), pick: Some(pick), ..Default::default() }, None)
}

/// The log from an event down, so it is the first row, highlighted; only
/// its app's events when `app` is given.
pub(super) fn from_event(id: i64, app: Option<&str>) -> String {
    format!("{}#event-{id}", href(&Filters { app: app.map(Into::into), ..Default::default() }, Some(id + 1)))
}

/// The log with these filters, at the page before `before`.
fn href(f: &Filters, before: Option<i64>) -> String {
    f.at("/admin/log", before)
}

fn form(f: &Filters, apps: &[App]) -> Form {
    let option = |value: &str, label: &str, on: bool| Opt { value: value.to_string(), label: label.to_string(), on };
    let mut named: Vec<&App> = apps.iter().collect();
    named.sort_by_key(|a| a.name.to_lowercase());
    let mut app = vec![option("", "All apps", f.app.is_none())];
    app.extend(named.iter().map(|a| option(&a.id, &a.name, f.app.as_deref() == Some(&a.id))));
    if let Some(id) = f.app.as_deref().filter(|id| !id.is_empty() && !apps.iter().any(|a| a.id == *id)) {
        app.push(option(id, id, true));
    }
    app.push(option(PLATFORM, "Platform", f.app.as_deref() == Some("")));
    let kind = vec![
        option("", "All events", f.pick.is_none()),
        option("failures", "Failures", f.pick == Some(Pick::Failures)),
        option("changes", "Changes", f.pick == Some(Pick::Changes)),
    ];
    let kinds = Kind::ALL.into_iter().map(|k| option(k.as_str(), kind_name(k), f.pick == Some(Pick::One(k)))).collect();
    let mut severity = vec![option("", "Any", f.severity.is_none())];
    for s in &Severity::ALL[1..] {
        let label = if *s == Severity::Critical { "Critical".to_string() } else { format!("{} or worse", capitalize(s.as_str())) };
        severity.push(option(s.as_str(), &label, f.severity == Some(*s)));
    }
    let mut by = vec![option("", "Anyone", f.by.is_none())];
    by.extend(Actor::ALL.into_iter().map(|a| option(a.as_str(), actor(a), f.by == Some(a))));
    let select = |name, label, options| Select { name, label, options, group: None };
    let date = |d: Option<NaiveDate>| d.map(|d| d.to_string()).unwrap_or_default();
    Form {
        text: f.text.clone().unwrap_or_default(),
        selects: vec![
            select("app", "App", app),
            Select { name: "kind", label: "Event", options: kind, group: Some(("One kind", kinds)) },
            select("severity", "Severity", severity),
            select("by", "By", by),
        ],
        dates: vec![("from", "From", date(f.from)), ("to", "To", date(f.to))],
        clear: *f != Filters::default(),
    }
}

/// A kind as the Event filter lists it.
fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Deployed => "Deployed or made live",
        Kind::Refused => "Refused",
        Kind::RolledBack => "Rolled back by hand",
        Kind::AutoRolledBack => "Rolled back automatically",
        Kind::LimitHit => "Limit hit",
        Kind::RiskChanged => "Risk changed",
        Kind::PolicyChanged => "Policy changed",
        Kind::PolicyRefused => "Policy refused",
        Kind::Held => "Held",
        Kind::TurnedOff => "Turned off",
        Kind::TurnedOn => "Turned on",
        Kind::Seen => "Seen",
        Kind::Frozen => "Deploys frozen",
        Kind::Unfrozen => "Deploys unfrozen",
        Kind::LimitsSet => "Limits set",
        Kind::MessageFailed => "Message failed",
        Kind::MessageRetried => "Message retried",
    }
}

/// The events by day.
fn days(shell: &Shell, apps: &[App], events: &[Event], now: i64) -> Vec<Day> {
    let mut days: Vec<(String, Day)> = vec![];
    for e in events {
        let d = local(e.at, "%F");
        if days.last().is_none_or(|(day, _)| *day != d) {
            days.push((d, Day { name: day_name(e.at, now), rows: vec![] }));
        }
        if let Some((_, day)) = days.last_mut() {
            day.rows.push(row(shell, apps, e));
        }
    }
    days.into_iter().map(|(_, day)| day).collect()
}

/// "Today", "Yesterday", "Wednesday, November 11", with the year when it
/// isn't this one.
fn day_name(at: i64, now: i64) -> String {
    let day = local(at, "%F");
    if day == local(now, "%F") {
        "Today".to_string()
    } else if day == local(now - 24 * 3_600_000, "%F") {
        "Yesterday".to_string()
    } else if local(at, "%Y") == local(now, "%Y") {
        local(at, "%A, %B %-d")
    } else {
        local(at, "%A, %B %-d, %Y")
    }
}

/// Who did it, in the filter.
fn actor(a: Actor) -> &'static str {
    match a {
        Actor::Shell => "Shell",
        Actor::Cli => "appos CLI",
        Actor::Admin => "Admin screen",
    }
}

/// Who did it, in a row: the admin screen with who was signed in.
fn who(e: &Event) -> String {
    match &e.person {
        Some(person) => format!("{}, {person}", actor(e.actor)),
        None => actor(e.actor).to_string(),
    }
}

/// Routine is plain text; from medium on, a badge.
fn severity(s: Severity) -> String {
    let name = capitalize(s.as_str());
    match s {
        Severity::Info | Severity::Low => format!(r#"<span class="quiet">{name}</span>"#),
        Severity::Medium => badge(Tone::Caution, &name, false),
        Severity::High => badge(Tone::Warning, &name, false),
        Severity::Critical => badge(Tone::Critical, &name, true),
    }
}

fn row(shell: &Shell, apps: &[App], e: &Event) -> Row {
    let app = match apps.iter().find(|a| a.id == e.app_id) {
        Some(a) => Who::App(a.id.clone(), icon_html(a, 20), a.name.clone()),
        None if e.app_id.is_empty() => Who::Platform,
        None => Who::Unknown(e.app_id.clone()),
    };
    let (message, reason) = said(e);
    Row {
        id: e.id,
        full: local(e.at, FULL),
        time: local(e.at, "%H:%M:%S"),
        tone: Tone::of_event(e.kind).name(),
        icon: path_icon(icon(e), 16),
        word: word(e).to_string(),
        app,
        message,
        reason: reason.map(Into::into),
        label: e.label.clone(),
        severity: severity(e.severity),
        by: who(e),
        whole: e.reason.clone().filter(|r| r.trim_end().contains('\n')),
        when: local(e.at, "%A, %B %-d, %Y, %H:%M:%S (UTC%:z)"),
        risk: (!e.app_id.is_empty()).then(|| risk_chip(e.risk)),
        recovery: e.recovery_ms.map(span),
        behind: behind(shell, e),
    }
}

/// What happened in a line, and why in another: the message without the
/// event's word it starts with ("Deployed, live in 3 ms" → "Live in 3 ms"),
/// the reason when the message says nothing more ("Turned off"), or who
/// did it when there is no reason either.
fn said(e: &Event) -> (String, Option<&str>) {
    let reason = e.reason.as_deref().map(first_line);
    let message = message(e);
    let rest = message.strip_prefix(word(e)).map(|r| r.trim_start_matches([',', ' ', ':']));
    if e.kind == Kind::Refused || rest == Some("") {
        return (capitalize(reason.unwrap_or(&by(e))), None);
    }
    (rest.map_or(message.clone(), capitalize), reason)
}

/// The failed requests behind a limit hit (the log gets one event for all
/// in its 10 minutes) or an automatic rollback (the failures it counted).
fn behind(shell: &Shell, e: &Event) -> Behind {
    let Ok(Some((reqs, n))) = usage::behind(&shell.platform, e, BEHIND) else { return Behind::Nothing };
    if n == 0 {
        return Behind::Gone(thousands(KEEP as u64));
    }
    let what = match e.outcome {
        Some(outcome) if e.kind == Kind::LimitHit => {
            format!("{} {} in the 10 minutes from here; the log notes only the first.", plural(n as usize, "request"), kind_label(outcome).to_lowercase())
        }
        _ => format!("{} up to the rollback.", plural(n as usize, "failed request")),
    };
    let reqs = reqs
        .iter()
        .map(|r| Req {
            time: local(r.at, "%H:%M:%S"),
            route: r.route.to_string(),
            took: duration(r.wall_us),
            error: first_line(r.error.as_deref().unwrap_or("")).to_string(),
        })
        .collect();
    Behind::Listed(what, (n > BEHIND).then_some(BEHIND), reqs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: Kind, actor: Actor, message: &str, reason: Option<&str>) -> Event {
        Event {
            id: 1,
            at: 0,
            app_id: "a".into(),
            label: "0.1.0".into(),
            kind,
            severity: Severity::Info,
            actor,
            message: message.into(),
            reason: reason.map(Into::into),
            risk: None,
            recovery_ms: None,
            outcome: None,
            before: None,
            person: None,
        }
    }

    fn says(kind: Kind, message: &str, reason: Option<&str>) -> (String, Option<String>) {
        let e = event(kind, Actor::Cli, message, reason);
        let (line, why) = said(&e);
        (line, why.map(Into::into))
    }

    /// Links keep the filters and drop what is empty.
    #[test]
    fn links_into_the_log() {
        let (f, before) = Filters::parse(&crate::web::pairs(Some(
            "q=+consent+&app=_platform&kind=failures&severity=medium&by=admin&from=2026-11-01&to=2026-11-13&before=42",
        )))
        .unwrap();
        assert_eq!(href(&f, before), "/admin/log?q=consent&app=_platform&kind=failures&severity=medium&by=admin&from=2026-11-01&to=2026-11-13&before=42");
        assert_eq!(href(&Filters::default(), None), "/admin/log");
        assert_eq!(from_event(93, Some("leads")), "/admin/log?app=leads&before=94#event-93");
        assert_eq!(of_kinds(Pick::Failures), "/admin/log?kind=failures");
    }

    /// The message doesn't repeat the event's word; the reason, or else
    /// who did it, fills in when there is nothing else to say.
    #[test]
    fn a_row_says_what_the_event_column_does_not() {
        assert_eq!(says(Kind::Deployed, "Deployed, live in 3 ms", None), ("Live in 3 ms".into(), None));
        assert_eq!(
            says(Kind::AutoRolledBack, "Rolled back automatically from b to a", Some("5 failed requests in 9 s")),
            ("From b to a".into(), Some("5 failed requests in 9 s".into()))
        );
        assert_eq!(says(Kind::TurnedOff, "Turned off", Some("leaks\nemails")), ("Leaks".into(), None));
        assert_eq!(says(Kind::TurnedOn, "Turned on", None), ("With the appos CLI".into(), None));
        assert_eq!(says(Kind::TurnedOff, "Turned off with Stop all", None), ("With Stop all".into(), None));
        assert_eq!(says(Kind::Seen, "Seen: failing, near-limit", None), ("Failing, Near limit".into(), None));
        assert_eq!(says(Kind::Refused, "Deploy refused", Some("uses kv::set")), ("Uses kv::set".into(), None));
        assert_eq!(says(Kind::LimitHit, "Timed out on GET /", Some("limit")), ("Timed out on GET /".into(), Some("limit".into())));
    }
}
