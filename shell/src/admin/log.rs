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
use crate::web::escape;

use super::format::{FULL, capitalize, duration, first_line, local, plural, span, thousands};
use super::model::{App, Tone, by, icon, kind_label, load, message, state, word};
use super::style::{badge, icon_html, path_icon, risk_chip};
use super::{Ask, Viewer, page, plain, read};

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
        (false, ..) => table(shell, &apps, &events, now),
        (true, false, false) => {
            r#"<p class="empty">Nothing has happened yet. Every deploy, refusal, rollback, limit hit and switch shows up here, and stays.</p>"#.to_string()
        }
        (true, false, true) => r#"<p class="empty">Nothing older.</p>"#.to_string(),
        (true, true, older) => {
            format!(r#"<p class="empty">No {}events match these filters. <a href="/admin/log">Clear the filters</a></p>"#, if older { "older " } else { "" })
        }
    };
    let form = if events.is_empty() && !filtered && before.is_none() { String::new() } else { form(filters, &apps) };
    let mut pages = vec![];
    if before.is_some() {
        pages.push(format!(r#"<a class="button" href="{}">Newest</a>"#, escape(&href(filters, None))));
    }
    if let Some(id) = older {
        pages.push(format!(r#"<a class="button" href="{}">Older</a>"#, escape(&href(filters, Some(id)))));
    }
    let pages = if pages.is_empty() { String::new() } else { format!(r#"<div class="pages">{}</div>"#, pages.concat()) };
    let body = format!(
        r#"<section class="card logcard" aria-labelledby="log-h"><div class="cardhead"><h2 id="log-h">Log <small>· every deploy, refusal, rollback, limit hit and switch, newest first, kept for good</small></h2></div>{form}{list}{pages}</section>"#
    );
    Reply::document(page(shell, viewer, state(&apps), "/admin/log", &format!("Log · {} admin", shell.name), &body, None))
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

/// The filters in a row above the table, a label over each.
fn form(f: &Filters, apps: &[App]) -> String {
    let option =
        |value: &str, label: &str, on: bool| format!(r#"<option value="{}"{}>{}</option>"#, escape(value), if on { " selected" } else { "" }, escape(label));
    let mut named: Vec<&App> = apps.iter().collect();
    named.sort_by_key(|a| a.name.to_lowercase());
    let mut app = option("", "All apps", f.app.is_none());
    for a in named {
        app += &option(&a.id, &a.name, f.app.as_deref() == Some(&a.id));
    }
    if let Some(id) = f.app.as_deref().filter(|id| !id.is_empty() && !apps.iter().any(|a| a.id == *id)) {
        app += &option(id, id, true);
    }
    app += &option(PLATFORM, "Platform", f.app.as_deref() == Some(""));
    let mut kind = option("", "All events", f.pick.is_none());
    kind += &option("failures", "Failures", f.pick == Some(Pick::Failures));
    kind += &option("changes", "Changes", f.pick == Some(Pick::Changes));
    kind += r#"<optgroup label="One kind">"#;
    for k in Kind::ALL {
        kind += &option(k.as_str(), kind_name(k), f.pick == Some(Pick::One(k)));
    }
    kind += "</optgroup>";
    let mut severity = option("", "Any", f.severity.is_none());
    for s in &Severity::ALL[1..] {
        let label = if *s == Severity::Critical { "Critical".to_string() } else { format!("{} or worse", capitalize(s.as_str())) };
        severity += &option(s.as_str(), &label, f.severity == Some(*s));
    }
    let mut by = option("", "Anyone", f.by.is_none());
    for a in Actor::ALL {
        by += &option(a.as_str(), actor(a), f.by == Some(a));
    }
    let select = |name: &str, label: &str, options: &str| {
        format!(r#"<label class="field"><span>{label}</span><span class="select"><select name="{name}">{options}</select></span></label>"#)
    };
    let date = |name: &str, label: &str, d: Option<NaiveDate>| {
        let value = d.map(|d| d.to_string()).unwrap_or_default();
        format!(r#"<label class="field"><span>{label}</span><input type="date" name="{name}" value="{value}"></label>"#)
    };
    let clear = if *f == Filters::default() { String::new() } else { r#"<a href="/admin/log">Clear</a>"#.to_string() };
    format!(
        r#"<form class="filters" method="get" action="/admin/log" role="search" aria-label="Filter the log"><label class="field grow"><span>Search</span><input type="search" name="q" value="{text}" placeholder="Text or version"></label>{app}{kind}{severity}{by}{from}{to}<div class="go"><button class="primary">Filter</button>{clear}</div></form>"#,
        text = escape(f.text.as_deref().unwrap_or("")),
        app = select("app", "App", &app),
        kind = select("kind", "Event", &kind),
        severity = select("severity", "Severity", &severity),
        by = select("by", "By", &by),
        from = date("from", "From", f.from),
        to = date("to", "To", f.to),
    )
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

/// The events by day, a row each.
fn table(shell: &Shell, apps: &[App], events: &[Event], now: i64) -> String {
    let mut days = String::new();
    let mut day = String::new();
    for e in events {
        let d = local(e.at, "%F");
        if d != day {
            if !day.is_empty() {
                days += "</tbody>";
            }
            days += &format!(r#"<tbody><tr class="day"><th colspan="7" scope="rowgroup">{}</th></tr>"#, day_name(e.at, now));
            day = d;
        }
        days += &row(shell, apps, e);
    }
    if !day.is_empty() {
        days += "</tbody>";
    }
    format!(
        r#"<div class="scroll"><table class="log"><thead><tr><th scope="col">Time</th><th scope="col">Event</th><th scope="col">App</th><th scope="col">What happened</th><th scope="col">Version</th><th scope="col">Severity</th><th scope="col">By</th></tr></thead>{days}</table></div>"#
    )
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

fn row(shell: &Shell, apps: &[App], e: &Event) -> String {
    let app = match apps.iter().find(|a| a.id == e.app_id) {
        Some(a) => format!(r#"<a class="who-app" href="/admin/apps/{}#detail">{}<span>{}</span></a>"#, escape(&a.id), icon_html(a, 20), escape(&a.name)),
        None if e.app_id.is_empty() => r#"<span class="quiet">Platform</span>"#.to_string(),
        None => escape(&e.app_id),
    };
    let (message, reason) = said(e);
    let reason = reason.map(|r| format!(r#"<span class="because">{}</span>"#, escape(r))).unwrap_or_default();
    let label = if e.label.is_empty() { r#"<span class="quiet">—</span>"#.to_string() } else { format!("<code>{}</code>", escape(&e.label)) };
    format!(
        r#"<tr id="event-{id}"><td class="time" title="{full}">{time}</td><td><span class="event tone-{tone}">{icon}{word}</span></td><td>{app}</td><td class="what"><details><summary><span class="msg">{message}</span>{reason}</summary>{more}</details></td><td>{label}</td><td>{severity}</td><td>{by}</td></tr>"#,
        id = e.id,
        full = local(e.at, FULL),
        time = local(e.at, "%H:%M:%S"),
        tone = Tone::of_event(e.kind).name(),
        word = word(e),
        icon = path_icon(icon(e), 16),
        message = escape(&message),
        more = more(shell, e),
        severity = severity(e.severity),
        by = escape(&who(e)),
    )
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

/// What a row opens to: the reason when it is longer than its first line,
/// the facts, and for a limit hit the requests behind it.
fn more(shell: &Shell, e: &Event) -> String {
    let mut out = String::new();
    if let Some(r) = e.reason.as_deref().filter(|r| r.trim_end().contains('\n')) {
        out += &format!("<pre>{}</pre>", escape(r));
    }
    let mut facts = vec![("When", local(e.at, "%A, %B %-d, %Y, %H:%M:%S (UTC%:z)"))];
    if !e.app_id.is_empty() {
        facts.push(("App's risk then", risk_chip(e.risk)));
    }
    if let Some(ms) = e.recovery_ms {
        facts.push(("First failure to rolled back", span(ms)));
    }
    let facts: String = facts.iter().map(|(k, v)| format!("<dt>{k}</dt><dd>{v}</dd>")).collect();
    out += &format!(r#"<dl class="facts">{facts}</dl>"#);
    out += &behind(shell, e);
    format!(r#"<div class="whole">{out}</div>"#)
}

/// The failed requests behind a limit hit (the log gets one event for all
/// in its 10 minutes) or an automatic rollback (the failures it counted).
fn behind(shell: &Shell, e: &Event) -> String {
    let Ok(Some((reqs, n))) = usage::behind(&shell.platform, e, BEHIND) else { return String::new() };
    if n == 0 {
        return format!(r#"<p class="quiet">The requests behind it are no longer kept: platform.db keeps the newest {} per app.</p>"#, thousands(KEEP as u64));
    }
    let what = match e.outcome {
        Some(outcome) if e.kind == Kind::LimitHit => {
            format!("{} {} in the 10 minutes from here; the log notes only the first.", plural(n as usize, "request"), kind_label(outcome).to_lowercase())
        }
        _ => format!("{} up to the rollback.", plural(n as usize, "failed request")),
    };
    let first = if n > BEHIND { format!(" The first {BEHIND}:") } else { String::new() };
    let items: String = reqs
        .iter()
        .map(|r| {
            format!(
                r#"<li><span>{}</span><code>{}</code><span>{}</span><span class="err">{}</span></li>"#,
                local(r.at, "%H:%M:%S"),
                escape(&r.route),
                duration(r.wall_us),
                escape(first_line(r.error.as_deref().unwrap_or("")))
            )
        })
        .collect();
    format!(r#"<p>{what}{first}</p><ul class="behind">{items}</ul>"#)
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
