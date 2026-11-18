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

use super::app::detail;
use super::charts::{MARKED, MINUTES, failures_chart, heat, marks, requests_chart};
use super::console::{Board, View};
use super::format::{FULL, clock, local, span, thousands};
use super::kpis::{Month, kpis};
use super::log::{from_event, of_app_kinds};
use super::model::{App, MINUTE, Tone, icon, load, name_of, sentence, state, word};
use super::style::{grant_label, icon_html, owner_html, path_icon, risk_label, risk_legend};
use super::{Ask, Viewer, html, missing, page, read};
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
    let board = Board::new(shell, &apps, &view, now, selected).changes(viewer.changes);
    let chosen = board.chosen();
    let overview = match chosen {
        Some(app) => {
            // What went live or was turned off, as lines on the charts.
            let changes = shell.platform.events_since(None, now - MINUTES * MINUTE, &MARKED).unwrap_or_default();
            let all = marks(&changes, &apps, None, now);
            let own = marks(changes.iter().filter(|e| e.app_id == app.id), &apps, Some(&app.id), now);
            Overview {
                start: None,
                parts: Some(Parts {
                    board: board.html(),
                    kpis: kpis(&apps, &Month::load(shell, now)),
                    requests: requests_chart(&apps, &all, now),
                    failures: failures_chart(&apps, &all, now),
                    heat: heat(&apps, &app.id, now),
                    detail: detail(shell, &apps, app, &own, all_versions, viewer.changes),
                    legend: risk_legend(),
                    inventory: inventory(&apps, &shell.risk),
                    history: history(&apps, &events),
                    kept: thousands(KEEP as u64),
                }),
            }
        }
        None => Overview { start: Some(start(url, &shell.risk, &shell.kits.now().for_apps())), parts: None },
    };
    let title = match selected.and(chosen) {
        Some(app) => format!("{} · {} admin", app.name, shell.name),
        None => format!("{} admin", shell.name),
    };
    Reply::document(page(shell, viewer, state, "/admin", &title, &html(&overview), Some(&here)))
}

/// The page around the console (templates/admin/overview.html): with an
/// app, its parts; before the first, how it gets here.
#[derive(askama::Template)]
#[template(path = "admin/overview.html")]
struct Overview {
    start: Option<Start>,
    parts: Option<Parts>,
}

/// The console, the numbers, the charts (drawn in Rust), the selected app,
/// the inventory and the history.
struct Parts {
    board: String,
    kpis: String,
    requests: String,
    failures: String,
    heat: String,
    detail: String,
    legend: String,
    inventory: Vec<Item>,
    history: Vec<Happened>,
    /// The requests platform.db keeps per app.
    kept: String,
}

/// The first visit: this platform's URL for `appos login`, and the rules
/// every app runs under: the limits of a request, the automatic rollback
/// and the risk levels.
struct Start {
    url: String,
    seconds: u64,
    max_seconds: u64,
    billions: u64,
    io: u32,
    mb: usize,
    failures: usize,
    window: String,
    max_risk: u8,
    grants: String,
    /// A grant has a level.
    rated: bool,
}

fn start(url: &str, risk: &Risk, names: &[String]) -> Start {
    Start {
        url: url.to_string(),
        seconds: SECONDS,
        max_seconds: MAX_SECONDS,
        billions: FUEL / 1_000_000_000,
        io: IO_BUDGET,
        mb: MEMORY / (1024 * 1024),
        failures: FAILURES_BEFORE_ROLLBACK,
        window: match FAILURE_WINDOW.as_secs() {
            60 => "a minute".to_string(),
            s => format!("{s} s"),
        },
        max_risk: MAX_RISK,
        grants: names.iter().map(|g| grant_label(g, risk.of(g))).collect(),
        rated: names.iter().any(|g| risk.of(g).is_some()),
    }
}

/// An app in the inventory: what it is, who owns it, what runs, what it
/// may touch and how risky that is, how busy it was, what changed last and
/// what was refused.
struct Item {
    id: String,
    icon: String,
    name: String,
    kind: &'static str,
    group: Option<String>,
    owner: Option<String>,
    /// The live version; none when it doesn't run.
    version: Option<String>,
    /// Not running though a version is live.
    stopped: bool,
    chip: Option<String>,
    /// Its grants' labels while it runs.
    grants: Option<String>,
    requests: String,
    changed: Option<Change>,
    refused: u32,
    refused_href: String,
}

/// An event, as a line with its icon and a link into the log.
struct Change {
    tone: &'static str,
    icon: String,
    word: String,
    href: String,
    full: String,
    clock: String,
}

/// What happened, with whose it was and the sentence the log says.
struct Happened {
    tone: &'static str,
    icon: String,
    name: String,
    word: String,
    href: String,
    full: String,
    clock: String,
    label: String,
    sentence: String,
    window: String,
}

fn inventory(apps: &[App], risk: &Risk) -> Vec<Item> {
    let item = |a: &App| Item {
        id: a.id.clone(),
        icon: icon_html(a, 24),
        name: a.name.clone(),
        kind: if a.is_kit() { "Kit" } else { "App" },
        group: a.group().map(Into::into),
        owner: a.owner().map(owner_html),
        version: a.running.as_ref().map(|r| r.label.clone()),
        stopped: a.running.is_none() && a.versions.iter().any(|v| v.status == VersionStatus::Live),
        chip: risk_label(risk, a),
        grants: a.running.as_ref().map(|r| r.manifest.capabilities.grant.iter().map(|g| grant_label(g, risk.of(g))).collect()),
        requests: thousands(a.reqs.len() as u64),
        changed: a.changed.as_ref().map(|e| Change {
            tone: Tone::of_event(e.kind).name(),
            icon: path_icon(icon(e), 16),
            word: word(e).to_string(),
            href: from_event(e.id, Some(&a.id)),
            full: local(e.at, FULL),
            clock: clock(e.at),
        }),
        refused: a.refused,
        refused_href: of_app_kinds(&a.id, Pick::One(Kind::Refused)),
    };
    apps.iter().map(item).collect()
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
fn history(apps: &[App], events: &[Event]) -> Vec<Happened> {
    events
        .iter()
        .filter(|e| {
            matches!(e.kind, Kind::Refused | Kind::AutoRolledBack | Kind::RolledBack | Kind::Held | Kind::TurnedOff | Kind::TurnedOn)
                || (e.kind == Kind::Deployed && e.actor == Actor::Admin)
        })
        .take(20)
        .map(|e| Happened {
            tone: Tone::of_event(e.kind).name(),
            icon: path_icon(icon(e), 16),
            name: name_of(apps, &e.app_id).to_string(),
            word: word(e).to_lowercase(),
            href: from_event(e.id, None),
            full: local(e.at, FULL),
            clock: clock(e.at),
            label: e.label.clone(),
            // The sentence the log and the charts say, and for an automatic
            // rollback its failures' times.
            sentence: sentence(e, None),
            window: if e.kind == Kind::AutoRolledBack { window(e) } else { String::new() },
        })
        .collect()
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
