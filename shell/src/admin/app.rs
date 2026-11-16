//! One app in detail: its requests with its changes, its routes, its risk
//! grant by grant, its versions with "Make live", and its last failures.
//! Turn off, Turn on and Make live act at once; the console's toast says
//! what happened and offers Undo.

use axum::extract::{Path, RawQuery, State};
use axum::response::Response;

use crate::log_filter::{PLATFORM, Pick};
use crate::manifest::{Band, allows};
use crate::platform::{Kind, Request, Version, VersionStatus, now_ms};
use crate::runtime::IO_BUDGET;
use crate::shell::Shell;
use crate::web::escape;

use super::charts::{Mark, app_chart, column};
use super::format::{FULL, and, capitalize, clock, duration, first_line, fuel, local, ms, plural, thousands};
use super::log::{from_event, of_app, of_app_kinds};
use super::model::{App, Tone, VERSIONS, by, kind_label, risk_changes, risk_of, stats};
use super::overview::{Selected, dashboard};
use super::style::{POWER_PATH, badge, grant_chip, icon_html, owner_html, path_icon, tone_icon};
use super::{PlatformUrl, S, Viewer, read};
use crate::web::pairs;

/// Versions shown before "All versions".
const SHOWN: usize = 10;

/// The same page, with this app in detail. `?versions=all` shows every
/// version; the rest is the console's view and its toast.
pub(super) async fn app_page(State(shell): S, viewer: Viewer, PlatformUrl(url): PlatformUrl, Path(id): Path<String>, RawQuery(query): RawQuery) -> Response {
    let query = pairs(query.as_deref());
    let all_versions = query.iter().any(|(k, v)| k == "versions" && v == "all");
    read(shell, move |shell| dashboard(shell, &viewer, &url, Some(Selected { id: &id, all_versions }), &query)).await
}

/// A button that changes the app at once: a form posting to `action`,
/// back to the app's page.
fn act(a: &App, action: &str, fields: &[(&str, &str)], button: &str) -> String {
    let path = format!("/admin/apps/{}", form_urlencoded::byte_serialize(a.id.as_bytes()).collect::<String>());
    let fields: String =
        fields.iter().chain(&[("then", path.as_str())]).map(|(k, v)| format!(r#"<input type="hidden" name="{k}" value="{}">"#, escape(v))).collect();
    format!(r#"<form method="post" action="{}">{fields}{button}</form>"#, escape(&format!("{path}/{action}")))
}

/// The app: its header, its requests with its changes, routes, versions
/// and last failures.
pub(super) fn detail(shell: &Shell, a: &App, marks: &[Mark], all_versions: bool) -> String {
    let requests = plural(a.reqs.len(), "request");
    let log = escape(&of_app(&a.id));
    let group = a.group().map(|g| format!("{} · ", escape(g))).unwrap_or_default();
    let owner = format!("{group}{}", a.owner().map(|o| format!("owned by {} · ", owner_html(o))).unwrap_or_default());
    let (sub, limits) = match &a.running {
        Some(r) => {
            let grants = &r.manifest.capabilities.grant;
            let grants = if grants.is_empty() { "nothing".to_string() } else { grants.join(", ") };
            (
                format!(
                    r#"{owner}{requests} in the last hour · live <code>{}</code> · grants {} · {} · <a href="{log}">its log</a> · <a href="{}/{}">open the app</a>"#,
                    escape(&r.label),
                    escape(&grants),
                    risk_of(&shell.risk, a).map(|r| format!(r##"<a href="#risk">{r}</a>"##)).unwrap_or_default(),
                    escape(&shell.origin),
                    escape(&a.id)
                ),
                {
                    let b = a.budget();
                    let set = if a.limits == Default::default() { "" } else { " (set by an admin)" };
                    format!(
                        r#"<span class="limits">limits {}, {} fuel, {IO_BUDGET} I/O, {} MB{set}</span>"#,
                        duration(a.limit_us()),
                        fuel(b.fuel),
                        b.memory >> 20
                    )
                },
            )
        }
        None => (format!(r#"{owner}{requests} in the last hour · not running · <a href="{log}">its log</a>"#), String::new()),
    };
    // The kill switch: off in the header, on in the banner while off.
    let (switch, off) = match (&a.running, &a.off) {
        (Some(_), None) => (act(a, "off", &[], &format!(r#"<button class="button switch">{}Turn off</button>"#, path_icon(POWER_PATH, 16))), String::new()),
        (_, Some(e)) => (
            String::new(),
            format!(
                r#"<div class="warn tone-warning banner">{}<p><b>Turned off</b> at {} {}{}. Visitors get a page saying so; nothing runs.</p>{}</div>"#,
                tone_icon(Tone::Warning, 16),
                clock(e.at),
                escape(&by(e)),
                e.reason.as_deref().map(|r| format!(": {}", escape(r))).unwrap_or_default(),
                act(a, "on", &[], r#"<button class="primary">Turn on</button>"#),
            ),
        ),
        (None, None) => (String::new(), String::new()),
    };
    format!(
        r#"<section class="detail" aria-labelledby="dt-h"><div class="dhead">{icon}<span class="dname"><h3 id="dt-h">{name}</h3><span class="sub">{sub}</span></span>{limits}{switch}</div>{off}
{chart}{routes}{risk}{versions}{failures}</section>"#,
        icon = icon_html(a, 48),
        // Nothing to draw for an app that isn't running and had nothing happen.
        chart = if a.running.is_some() || !marks.is_empty() || a.reqs.iter().any(|r| column(r.at, now_ms()).is_some()) {
            app_chart(a, marks, now_ms())
        } else {
            String::new()
        },
        name = escape(&a.name),
        routes = routes_table(a),
        risk = risk_table(shell, a),
        versions = versions_table(shell, a, all_versions),
        failures = last_failures(a),
    )
}

/// Per route, the last hour, all versions.
pub(super) fn routes_table(a: &App) -> String {
    let mut routes: Vec<(String, Vec<&Request>)> = a.running.iter().flat_map(|r| r.routes.iter().map(|x| (x.pattern(), vec![]))).collect();
    for r in &a.reqs {
        match routes.iter_mut().find(|(p, _)| **p == *r.route) {
            Some((_, reqs)) => reqs.push(r),
            None => routes.push((r.route.to_string(), vec![r])),
        }
    }
    if routes.is_empty() {
        let next = if a.versions.iter().any(|v| matches!(v.status, VersionStatus::Retired | VersionStatus::Failed)) {
            "Make an earlier version live below, or deploy a fixed one."
        } else {
            "Deploy a fixed one with <code>appos deploy</code>."
        };
        return format!(r#"<div class="part"><h4>Routes</h4><p class="empty">Not running, so no routes. {next}</p></div>"#);
    }
    let rows: String = routes
        .iter()
        .map(|(route, reqs)| {
            let s = stats(reqs);
            let ran = s.requests > s.failures;
            let or_dash = |v: String| if ran { v } else { "—".to_string() };
            let max = reqs.iter().filter(|r| !r.outcome.failed()).map(|r| r.wall_us).max().map_or("—".to_string(), ms);
            format!(
                r#"<tr><td><code>{}</code></td><td class="num">{}</td><td class="num{}">{}</td><td class="num">{}</td><td class="num">{}</td><td class="num">{max}</td><td class="num">{}</td><td class="num">{}</td></tr>"#,
                escape(route),
                thousands(s.requests as u64),
                if s.failures > 0 { " bad" } else { "" },
                thousands(s.failures as u64),
                or_dash(ms(s.wall_us.0)),
                or_dash(format!("<b>{}</b>", ms(s.wall_us.1))),
                or_dash(fuel(s.fuel.1)),
                or_dash(s.io.1.to_string()),
            )
        })
        .collect();
    format!(
        r#"<div class="part"><h4 id="routes-h">Routes <small>· last hour</small></h4><div class="scroll"><table class="routes" aria-labelledby="routes-h"><thead><tr><th scope="col">Route</th><th scope="col" class="num">Requests</th><th scope="col" class="num">Failed</th><th scope="col" class="num">Median ms</th><th scope="col" class="num">p95 ms</th><th scope="col" class="num">Max ms</th><th scope="col" class="num">Fuel p95</th><th scope="col" class="num">I/O p95</th></tr></thead><tbody>{rows}</tbody></table></div></div>"#
    )
}

/// Each grant with what it lets the app do and its level, riskiest first;
/// the ones that set the app's risk are tinted. Since: when the risk file
/// gave the grant its level, linked to that change in the log.
pub(super) fn risk_table(shell: &Shell, a: &App) -> String {
    let log = format!(
        r#"<p class="partfoot">Levels come from the platform's risk file; an app can't change them. <a href="{}">Its log</a> · <a href="{}">Risk changes</a></p>"#,
        escape(&of_app(&a.id)),
        escape(&of_app_kinds(PLATFORM, Pick::One(Kind::RiskChanged))),
    );
    let Some(r) = &a.running else {
        return format!(r#"<div class="part" id="risk"><h4>Risk</h4><p class="empty">Not running, so it can touch nothing.</p>{log}</div>"#);
    };
    let level = shell.risk.of_app(&r.manifest);
    let mut grants: Vec<(&str, Option<u8>)> = r.manifest.capabilities.grant.iter().map(|g| (g.as_str(), shell.risk.of(g))).collect();
    // Unrated first, it makes the app's risk unknown; then riskiest first.
    grants.sort_by_key(|(g, l)| (l.is_some(), std::cmp::Reverse(*l), *g));
    let why = match (level, grants.iter().find(|(_, l)| l.is_none())) {
        (_, Some((g, _))) => format!("unrated: {g} has no level"),
        (Some(l), None) if grants.is_empty() => format!("{} {l}: granted nothing, it can only answer requests", Band::of(l).name()),
        (Some(l), None) => {
            let top: Vec<&str> = grants.iter().filter(|(_, x)| *x == Some(l)).map(|(g, _)| *g).collect();
            format!("{} {l}, from {}", Band::of(l).name(), and(&top))
        }
        (None, None) => "unrated".to_string(),
    };
    let changes = risk_changes(shell);
    let rows: String = grants
        .iter()
        .map(|(g, l)| {
            let since = changes.get(*g).map_or_else(
                || r#"<span class="none">—</span>"#.to_string(),
                |e| {
                    format!(
                        r#"<a class="when" href="{}" title="{} in the log">{}</a>"#,
                        escape(&from_event(e.id, Some(PLATFORM))),
                        local(e.at, FULL),
                        clock(e.at)
                    )
                },
            );
            format!(
                r#"<tr{}><td><code>{}</code></td><td>{}</td><td>{}</td><td>{since}</td></tr>"#,
                if *l == level { r#" class="sets""# } else { "" },
                escape(g),
                grant_chip(*l),
                allows(g),
            )
        })
        .collect();
    let table = if rows.is_empty() {
        String::new()
    } else {
        format!(
            r#"<div class="scroll"><table class="grants" aria-labelledby="risk-h"><thead><tr><th scope="col">Grant</th><th scope="col">Risk</th><th scope="col">Lets the app</th><th scope="col">Since</th></tr></thead><tbody>{rows}</tbody></table></div>"#
        )
    };
    format!(r#"<div class="part" id="risk"><h4 id="risk-h">Risk <small>· {why}</small></h4>{table}{log}</div>"#, why = escape(&why))
}

/// The live version first and marked, then the others newest first: when
/// each was deployed, how long it took to go live, why one failed or was
/// refused, and "Make live".
pub(super) fn versions_table(shell: &Shell, a: &App, all: bool) -> String {
    if a.versions.is_empty() {
        return r#"<div class="part" id="versions"><h4>Versions</h4><p class="empty">None to go back to: loaded from a folder when the shell started. Deployed with <code>appos deploy</code>, every version is kept.</p></div>"#
            .into();
    }
    let every;
    let versions = if all {
        every = shell.platform.versions(&a.id, u32::MAX).unwrap_or_default();
        &every
    } else {
        &a.versions
    };
    // The live one, then those waiting to go live, then the rest.
    let rank = |v: &&Version| match v.status {
        VersionStatus::Live => 0,
        VersionStatus::Waiting => 1,
        _ => 2,
    };
    let mut rows: Vec<&Version> = versions.iter().collect();
    rows.sort_by_key(rank);
    if !all {
        rows.truncate(SHOWN);
    }
    let live = a.running.as_ref().map(|r| r.label.as_str());
    let html: String = rows
        .iter()
        .map(|v| {
            // Making a waiting version live is what ends a pause: the one
            // thing to do here then.
            let action = if matches!(v.status, VersionStatus::Retired | VersionStatus::Failed | VersionStatus::Waiting) && live != Some(v.label.as_str()) {
                let primary = if v.status == VersionStatus::Waiting { r#" class="primary""# } else { "" };
                act(a, "rollback", &[("label", &v.label)], &format!("<button{primary}>Make live</button>"))
            } else {
                String::new()
            };
            // A refused or waiting version never went live.
            let took =
                if matches!(v.status, VersionStatus::Refused | VersionStatus::Waiting) { "—".to_string() } else { duration(v.took_ms.max(0) as u64 * 1000) };
            // Why it failed or was refused, under it at the table's width.
            let reason = v
                .reason
                .as_deref()
                .map(|r| {
                    format!(r#"<tr class="reason"><td></td><td colspan="4" title="{}"><div>{}</div></td></tr>"#, escape(r), escape(&capitalize(first_line(r))))
                })
                .unwrap_or_default();
            let class = match (v.status == VersionStatus::Live, v.reason.is_some()) {
                (true, _) => r#" class="live""#,
                (false, true) => r#" class="has-reason""#,
                _ => "",
            };
            format!(
                r#"<tr{class}><td><code>{}</code></td><td>{}</td><td title="{}">{}</td><td class="num">{took}</td><td class="act">{action}</td></tr>{reason}"#,
                escape(&v.label),
                badge(Tone::of_version(v.status), &capitalize(v.status.as_str()), false),
                local(v.deployed_at, FULL),
                clock(v.deployed_at),
            )
        })
        .collect();
    let more = if all && versions.len() > SHOWN {
        format!(r#"<a class="more" href="/admin/apps/{}#versions">The newest {SHOWN} only</a>"#, escape(&a.id))
    } else if versions.len() > SHOWN {
        let n = if versions.len() < VERSIONS as usize { format!("all {}", versions.len()) } else { "all".to_string() };
        format!(r#"<a class="more" href="/admin/apps/{}?versions=all#versions">Show {n} versions</a>"#, escape(&a.id))
    } else {
        String::new()
    };
    let hold = match &a.paused {
        Some(e) => format!(
            r#"<div class="warn tone-critical banner">{}<p><b>Deploys wait.</b> {} rolled itself back at {}{}. New deploys are stored but not live until someone makes a version live here.</p></div>"#,
            tone_icon(Tone::Critical, 16),
            escape(&a.name),
            clock(e.at),
            e.reason.as_deref().map(|r| format!(" ({})", escape(r))).unwrap_or_default(),
        ),
        None => String::new(),
    };
    format!(
        r#"<div class="part" id="versions"><h4>Versions</h4>{hold}<div class="scroll"><table class="versions"><thead><tr><th scope="col">Version</th><th scope="col">Status</th><th scope="col">Deployed</th><th scope="col" class="num">Live in</th><th scope="col"><span class="sr-only">Action</span></th></tr></thead><tbody>{html}</tbody></table></div>{more}</div>"#
    )
}

pub(super) fn last_failures(a: &App) -> String {
    let rows: String = a
        .reqs
        .iter()
        .filter(|r| r.outcome.failed())
        .take(5)
        .map(|r| {
            // The error, under its row like a version's reason.
            let error =
                r.error.as_deref().map(|e| format!(r#"<tr class="reason"><td></td><td colspan="3"><pre>{}</pre></td></tr>"#, escape(e))).unwrap_or_default();
            format!(
                r#"<tr{}><td title="{}">{}</td><td><code>{}</code></td><td>{}</td><td><code>{}</code></td></tr>{error}"#,
                if error.is_empty() { "" } else { r#" class="has-reason""# },
                local(r.at, FULL),
                local(r.at, "%H:%M:%S"),
                escape(&r.route),
                badge(Tone::of_outcome(r.outcome), kind_label(r.outcome), false),
                escape(&r.label),
            )
        })
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    format!(
        r#"<div class="part"><h4>Last failures</h4><div class="scroll"><table class="failures"><thead><tr><th scope="col">Time</th><th scope="col">Route</th><th scope="col">What happened</th><th scope="col">Version</th></tr></thead><tbody>{rows}</tbody></table></div></div>"#
    )
}
