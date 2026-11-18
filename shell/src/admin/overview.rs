//! The page: the console (see `console`), the numbers of the last hour,
//! one app in detail, the inventory, and what was refused, rolled back or
//! turned off.

use crate::deploy::{FAILURE_WINDOW, FAILURES_BEFORE_ROLLBACK};
use crate::log_filter::Pick;
use crate::manifest::{MAX_RISK, MAX_SECONDS, Risk, SECONDS};
use crate::platform::{Actor, Event, Kind, VersionStatus, now_ms};
use crate::runtime::{FUEL, IO_BUDGET, MEMORY};
use crate::shell::Shell;
use crate::usage::KEEP;
use crate::web::escape;

use super::app::detail;
use super::charts::{MARKED, MINUTES, failures_chart, heat, marks, requests_chart};
use super::console::{Board, View};
use super::format::{FULL, clock, local, span, thousands};
use super::kpis::{Month, kpis};
use super::log::{from_event, of_app_kinds};
use super::model::{App, MINUTE, Tone, icon, load, name_of, sentence, state, word};
use super::style::{grant_label, icon_html, owner_html, path_icon, risk_label, risk_legend};
use super::{Ask, Viewer, missing, page, read};
use crate::kit::Reply;

pub(super) async fn overview(ask: Ask) -> Reply {
    let url = ask.platform_url();
    let Ask { shell, viewer, req } = ask;
    read(shell, move |shell| dashboard(shell, &viewer, &url, None, &req.query)).await
}

/// The app shown in detail, and what was asked about it.
pub(super) struct Selected<'a> {
    pub id: &'a str,
    pub all_versions: bool,
}

/// The page, with the selected app in detail or else the console's first;
/// `query` is the page's, the console's view and what an action said.
pub(super) fn dashboard(shell: &Shell, viewer: &Viewer, url: &str, selected: Option<Selected>, query: &[(String, String)]) -> Reply {
    // This page's URL, which it fetches again when something changed.
    let path = match &selected {
        None => "/admin".to_string(),
        Some(s) => format!("/admin/apps/{}", form_urlencoded::byte_serialize(s.id.as_bytes()).collect::<String>()),
    };
    let here = match form_urlencoded::Serializer::new(String::new()).extend_pairs(query).finish() {
        q if q.is_empty() => path,
        q => format!("{path}?{q}"),
    };
    let view = View::parse(query);
    let all_versions = selected.as_ref().is_some_and(|s| s.all_versions);
    let selected = selected.map(|s| s.id);
    let now = now_ms();
    let (apps, events) = load(shell, now);
    let state = state(&apps);
    if let Some(id) = selected.filter(|id| !apps.iter().any(|a| a.id == *id)) {
        return missing(shell, viewer, state, &format!("No app {id}."));
    }
    let board = Board::new(shell, &apps, &view, now, selected);
    let chosen = board.chosen();
    // What needs attention first, then the apps, then the platform's numbers.
    let mut body = String::new();
    if let Some(app) = chosen {
        body += &board.html();
        body += &kpis(&apps, &Month::load(shell, now));
        // What went live or was turned off, as lines on the charts.
        let changes = shell.platform.events_since(None, now - MINUTES * MINUTE, &MARKED).unwrap_or_default();
        let all = marks(&changes, &apps, None, now);
        body += &format!(r#"<div class="row">{}{}</div>"#, requests_chart(&apps, &all, now), failures_chart(&apps, &all, now));
        let own = marks(changes.iter().filter(|e| e.app_id == app.id), &apps, Some(&app.id), now);
        body += &format!(r#"<div class="row" id="detail">{}{}</div>"#, heat(&apps, &app.id, now), detail(shell, &apps, app, &own, all_versions));
        body += &inventory(&apps, &shell.risk);
        body += &history(&apps, &events);
        body += &format!(
            r#"<p class="foot">Measured by the shell on every request. The last {} requests per app are kept in platform.db, so a restart keeps them.</p>"#,
            thousands(KEEP as u64)
        );
    } else {
        // Nothing to report yet: how the first app gets here, and the rules
        // it will run under.
        body += &format!(r#"<div class="start">{}{}</div>"#, first_deploy(url), guardrails(&shell.risk, &shell.kits.now().for_apps()));
    }
    let title = match selected.and(chosen) {
        Some(app) => format!("{} · {} admin", app.name, shell.name),
        None => format!("{} admin", shell.name),
    };
    Reply::document(page(shell, viewer, state, "/admin", &title, &body, Some(&here)))
}

/// The first visit: how the first app gets here, with this platform's URL.
fn first_deploy(url: &str) -> String {
    format!(
        r#"<section class="card apps first" aria-labelledby="apps-h"><h2 id="apps-h">Deploy the first app</h2>
<p>From a terminal: connect the appos CLI to this platform once, then deploy from the app's folder.</p>
<pre class="cmd"><code>appos login {}
appos new hello &amp;&amp; cd hello
appos deploy</code></pre>
<p>It shows up here with its requests, versions and risk. From the app's folder, <code>appos versions</code> then tells what happened to each version, and <code>appos log</code> why a request failed.</p></section>"#,
        escape(url)
    )
}

/// The rules every app runs under, before there is one: the limits of a
/// request, the automatic rollback and the risk levels.
fn guardrails(risk: &Risk, names: &[String]) -> String {
    let grants: String = names.iter().map(|g| grant_label(g, risk.of(g))).collect();
    let levels = match names.iter().any(|g| risk.of(g).is_some()) {
        true => "Levels come from the platform's risk file; an app can't change them.",
        false => "No grant has a level yet: start the shell with <code>--risk</code> and a risk file.",
    };
    let window = match FAILURE_WINDOW.as_secs() {
        60 => "a minute".to_string(),
        s => format!("{s} s"),
    };
    format!(
        r#"<section class="card rules" aria-labelledby="rules-h"><h2 id="rules-h">Guardrails <small>· for every app, from the first</small></h2><ul>
<li><b>{SECONDS} s</b><p><strong>Limits per request.</strong> {SECONDS} s unless the app asks for more, up to {MAX_SECONDS} s; {} billion instructions; an I/O budget of {IO_BUDGET}, where a read costs 1 and a write 5; {} MB of memory. Over a limit, the request stops.</p></li>
<li><b>{FAILURES_BEFORE_ROLLBACK} failures</b><p><strong>Automatic rollback.</strong> {FAILURES_BEFORE_ROLLBACK} failed requests within {window} make the version before live again. New deploys then wait until someone makes a version live here.</p></li>
<li><b>0–{MAX_RISK}</b><p><strong>Risk per grant.</strong> {grants}<br>{levels}</p></li>
</ul></section>"#,
        FUEL / 1_000_000_000,
        MEMORY / (1024 * 1024),
    )
}

/// Every app in a row: what it is, who owns it, what runs, what it may
/// touch and how risky that is, how busy it was, what changed last and what
/// was refused.
pub(super) fn inventory(apps: &[App], risk: &Risk) -> String {
    let none = |text: &str| format!(r#"<span class="none">{text}</span>"#);
    let rows: String = apps
        .iter()
        .map(|a| {
            let id = escape(&a.id);
            let (version, grants) = match &a.running {
                Some(r) => {
                    let grants: String = r.manifest.capabilities.grant.iter().map(|g| grant_label(g, risk.of(g))).collect();
                    let version = format!(r#"<a class="version" href="/admin/apps/{id}#versions"><code>{}</code></a>"#, escape(&r.label));
                    (version, if grants.is_empty() { none("none") } else { grants })
                }
                None if a.versions.iter().any(|v| v.status == VersionStatus::Live) => (none("not running"), none("—")),
                None => (none("none"), none("—")),
            };
            let level = risk_label(risk, a).map_or_else(
                || none("—"),
                |chip| format!(r##"<a class="to-risk" href="/admin/apps/{id}#risk" title="{}'s grants and their risk">{chip}</a>"##, escape(&a.name)),
            );
            let changed = a.changed.as_ref().map_or_else(
                || none("—"),
                |e| {
                    format!(
                        r#"<span class="change tone-{}">{}<span>{}</span><a class="when" href="{}" title="{} in the log">{}</a></span>"#,
                        Tone::of_event(e.kind).name(),
                        path_icon(icon(e), 16),
                        word(e),
                        escape(&from_event(e.id, Some(&a.id))),
                        local(e.at, FULL),
                        clock(e.at),
                    )
                },
            );
            let owner = a.owner().map_or_else(|| none("—"), owner_html);
            let group = a.group().map_or_else(|| none("—"), escape);
            let refused = match a.refused {
                0 => none("0"),
                n => format!(r#"<a class="bad" href="{}" title="{}'s refused deploys">{n}</a>"#, escape(&of_app_kinds(&a.id, Pick::One(Kind::Refused))), escape(&a.name)),
            };
            format!(
                r#"<tr><th scope="row"><a class="who-app" href="/admin/apps/{id}#detail">{}<span class="names"><b>{}</b><code class="id">{id}</code></span></a></th><td>{}</td><td>{group}</td><td>{owner}</td><td>{version}</td><td>{level}</td><td class="chips">{grants}</td><td class="num">{}</td><td class="changed">{changed}</td><td class="num">{refused}</td></tr>"#,
                icon_html(a, 24),
                escape(&a.name),
                if a.is_kit() { "Kit" } else { "App" },
                thousands(a.reqs.len() as u64),
            )
        })
        .collect();
    format!(
        r#"<section class="card inventory" id="inventory" aria-labelledby="inv-h"><div class="cardhead"><h2 id="inv-h">Inventory <small>· an app is as risky as its riskiest grant <span class="bands">{}</span></small></h2></div><div class="scroll"><table><thead><tr><th scope="col">Name</th><th scope="col">Kind</th><th scope="col">Group</th><th scope="col">Owner</th><th scope="col">Live version</th><th scope="col">Risk</th><th scope="col">Grants and their risk</th><th scope="col" class="num">Requests, last hour</th><th scope="col">Last change</th><th scope="col" class="num">Refused deploys</th></tr></thead><tbody>{rows}</tbody></table></div></section>"#,
        risk_legend(),
    )
}

/// An automatic rollback's failures in local times, and how long until
/// the earlier version was back.
pub(super) fn window(e: &Event) -> String {
    match e.recovery_ms {
        Some(ms) => format!(", {} to {}; back in {}", local(e.at - ms, "%H:%M:%S"), local(e.at, "%H:%M:%S"), span(ms)),
        None => String::new(),
    }
}

/// What went wrong or was changed by hand, newest first: refused and held
/// deploys, rollbacks, versions made live, apps turned off and on.
pub(super) fn history(apps: &[App], events: &[Event]) -> String {
    let rows: String = events
        .iter()
        .filter(|e| {
            matches!(e.kind, Kind::Refused | Kind::AutoRolledBack | Kind::RolledBack | Kind::Held | Kind::TurnedOff | Kind::TurnedOn)
                || (e.kind == Kind::Deployed && e.actor == Actor::Admin)
        })
        .take(20)
        .map(|e| {
            // The sentence the log and the charts say, and for an automatic
            // rollback its failures' times.
            let window = if e.kind == Kind::AutoRolledBack { window(e) } else { String::new() };
            format!(
                r##"<div class="ev"><span class="evicon tone-{}" aria-hidden="true">{}</span><span class="evtext"><span><b>{}</b> <span class="sub">{} at <a href="{}" title="{} in the log">{}</a> · <code>{}</code></span></span><pre>{}{window}</pre></span></div>"##,
                Tone::of_event(e.kind).name(),
                path_icon(icon(e), 16),
                escape(name_of(apps, &e.app_id)),
                word(e).to_lowercase(),
                escape(&from_event(e.id, None)),
                local(e.at, FULL),
                clock(e.at),
                escape(&e.label),
                escape(&sentence(e, None)),
            )
        })
        .collect();
    let rows = if rows.is_empty() {
        r#"<p class="empty">None yet. Refused and held deploys, rollbacks and apps turned off show up here.</p>"#.to_string()
    } else {
        rows + r#"<a class="more" href="/admin/log">Everything in the log</a>"#
    };
    format!(r#"<section class="card events" aria-labelledby="ev-h"><h2 id="ev-h">Refused, rolled back, turned off</h2>{rows}</section>"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::model::{ProblemKind, problems};
    use crate::admin::tokens::monograms;
    use crate::platform::{Severity, Version};

    fn app(id: &str, name: &str) -> App {
        App {
            id: id.into(),
            name: name.into(),
            running: None,
            reqs: vec![],
            versions: vec![],
            paused: None,
            off: None,
            changed: None,
            refused: 0,
            problems: vec![],
            seen: None,
            tile: monograms([id])[0],
            limits: Default::default(),
            waits: None,
        }
    }

    #[test]
    fn a_refused_deploy_says_whether_the_app_is_live() {
        let refused = Event {
            id: 1,
            at: 1_000,
            app_id: "nps".into(),
            label: "0.1.0-abc".into(),
            kind: Kind::Refused,
            severity: Severity::Low,
            actor: Actor::Cli,
            message: "Refused".into(),
            reason: Some("uses kv::set, which needs kv-write".into()),
            risk: None,
            recovery_ms: None,
            outcome: None,
            before: None,
            person: None,
        };
        let mut never = app("nps", "NPS survey");
        let version = |status: &str| Version {
            id: 1,
            app_id: "nps".into(),
            label: "0.1.0-abc".into(),
            version: "0.1.0".into(),
            name: "NPS survey".into(),
            owner: "Jane Doe <jane@example.com>".into(),
            group: "Marketing".into(),
            status: VersionStatus::parse(status).unwrap(),
            reason: None,
            deployed_at: 1_000,
            took_ms: 1,
            grants: vec!["kv-read".into()],
            routes: vec![],
        };
        never.versions.push(version("refused"));
        let p = &problems(&never, &Risk::default(), std::slice::from_ref(&refused), 2_000)[0];
        assert_eq!(p.kind, ProblemKind::Refused);
        assert!(p.text.ends_with("; it isn't live yet"), "{}", p.text);
        assert_eq!(p.line, "Not live yet. Uses kv::set, which needs kv-write");
        // A deploy after it got through: the refusal is history, not a problem.
        let mut fixed = app("nps", "NPS survey");
        fixed.versions = vec![version("live"), version("refused")];
        assert!(problems(&fixed, &Risk::default(), std::slice::from_ref(&refused), 2_000).iter().all(|p| p.kind != ProblemKind::Refused));
    }
}
