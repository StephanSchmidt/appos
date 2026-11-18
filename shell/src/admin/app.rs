//! One app in detail: its requests with its changes, its routes, its risk
//! grant by grant, its versions with "Make live", and its last failures.
//! Turn off, Turn on and Make live act at once; the console's toast says
//! what happened and offers Undo.

use crate::kit::{Kits, Offer, Reply};
use crate::log_filter::{PLATFORM, Pick};
use crate::manifest::Band;
use crate::platform::{Kind, Request, Version, VersionStatus, now_ms};
use crate::runtime::IO_BUDGET;
use crate::shell::Shell;

use super::charts::{Mark, app_chart, column};
use super::format::{FULL, and, capitalize, clock, duration, first_line, fuel, local, ms, plural, thousands};
use super::log::{from_event, of_app, of_app_kinds};
use super::model::{App, Tone, VERSIONS, by, kind_label, risk_changes, risk_of, stats};
use super::overview::{Selected, dashboard};
use super::style::{POWER_PATH, badge, grant_chip, icon_html, owner_html, path_icon, tone_icon};
use super::{Ask, html, read};

/// Versions shown before "All versions".
const SHOWN: usize = 10;

/// The same page, with this app in detail. `?versions=all` shows every
/// version; the rest is the console's view and its toast.
pub(super) async fn app_page(ask: Ask) -> Reply {
    let (url, id) = (ask.platform_url(), ask.id());
    let Ask { shell, viewer, req } = ask;
    let query = req.query;
    let all_versions = query.iter().any(|(k, v)| k == "versions" && v == "all");
    read(shell, move |shell| dashboard(shell, &viewer, &url, Some(Selected { id: &id, all_versions }), &query)).await
}

/// The app's section (templates/admin/app.html): its header, requests
/// with its changes, routes, kits, risk, versions and last failures.
#[derive(askama::Template)]
#[template(path = "admin/app.html")]
struct Detail<'a> {
    icon: String,
    name: &'a str,
    id: &'a str,
    /// Its page, which its forms post under and return to.
    path: String,
    group: Option<&'a str>,
    owner: Option<String>,
    requests: String,
    log: String,
    running: Option<Running>,
    limits: Option<Limits>,
    switch: Switch,
    chart: Option<String>,
    routes: Routes,
    kits: Option<KitsPart>,
    risk: RiskPart,
    versions: Option<Versions>,
    failures: Vec<Failure>,
    shown: usize,
    power: String,
    warn: String,
    critical: String,
    /// Its buttons act (admin-write).
    changes: bool,
}

struct Running {
    label: String,
    grants: String,
    /// The hosts a kit may fetch from.
    hosts: Option<String>,
    risk: Option<String>,
    origin: String,
}

struct Limits {
    time: String,
    fuel: String,
    io: u32,
    mb: usize,
    /// Set by an admin, not the defaults.
    set: bool,
}

/// The kill switch: off in the header, on in the banner while off (when,
/// by whom, why).
enum Switch {
    Off,
    On(String, String, Option<String>),
    Nothing,
}

/// The routes; none while it doesn't run (true when an earlier version
/// could be made live).
enum Routes {
    None(bool),
    Some(Vec<Route>),
}

/// A route's last hour; a dash where nothing ran.
struct Route {
    route: String,
    requests: String,
    bad: bool,
    failures: String,
    median: String,
    p95: Option<String>,
    max: String,
    fuel: String,
    io: String,
}

/// The kits it uses, and for a kit what uses it.
struct KitsPart {
    uses: Vec<Uses>,
    users: Option<Vec<User>>,
}

struct Uses {
    name: Link,
    from: String,
    grants: Vec<String>,
    through: Option<String>,
    now: String,
}

/// A Wasm kit's page, or a Rust kit's name.
enum Link {
    App(String, String),
    Code(String),
}

struct User {
    id: String,
    name: String,
    kind: &'static str,
    grants: Vec<String>,
    now: String,
}

/// Why it is as risky as it is (none while it doesn't run), its grants,
/// and the log of risk changes.
struct RiskPart {
    why: Option<String>,
    rows: Vec<GrantRow>,
    changes: String,
}

struct GrantRow {
    grant: String,
    chip: String,
    allows: String,
    /// It sets the app's risk.
    sets: bool,
    since: Option<Since>,
}

/// The risk file's change that gave a grant its level, in the log.
struct Since {
    href: String,
    full: String,
    clock: String,
}

/// The versions shown; `hold` while deploys wait (when, why).
struct Versions {
    hold: Option<(String, Option<String>)>,
    rows: Vec<VersionRow>,
    more: More,
}

struct VersionRow {
    label: String,
    badge: String,
    full: String,
    clock: String,
    took: String,
    live: bool,
    /// Make live, primary for a waiting version.
    make_live: Option<bool>,
    reason: Option<String>,
    /// The reason's first line.
    line: String,
}

/// A link to more versions, or back to the newest.
enum More {
    Nothing,
    Newest,
    All(String),
}

struct Failure {
    full: String,
    time: String,
    route: String,
    badge: String,
    label: String,
    error: Option<String>,
}

/// The app: its header, its requests with its changes, routes, versions
/// and last failures.
pub(super) fn detail(shell: &Shell, apps: &[App], a: &App, marks: &[Mark], all_versions: bool, changes: bool) -> String {
    let now = now_ms();
    let running = a.running.as_ref().map(|r| {
        let grants = &r.manifest.capabilities.grant;
        Running {
            label: r.label.clone(),
            grants: if grants.is_empty() { "nothing".to_string() } else { grants.join(", ") },
            hosts: r.manifest.kit.as_ref().filter(|k| !k.hosts.is_empty()).map(|k| k.hosts.join(", ")),
            risk: risk_of(&shell.risk, a),
            origin: shell.origin.clone(),
        }
    });
    let limits = a.running.as_ref().map(|_| {
        let b = a.budget();
        Limits { time: duration(a.limit_us()), fuel: fuel(b.fuel), io: IO_BUDGET, mb: b.memory >> 20, set: a.limits != Default::default() }
    });
    let switch = match (&a.running, &a.off) {
        (Some(_), None) => Switch::Off,
        (_, Some(e)) => Switch::On(clock(e.at), by(e), e.reason.clone()),
        (None, None) => Switch::Nothing,
    };
    // Nothing to draw for an app that isn't running and had nothing happen.
    let drawn = a.running.is_some() || !marks.is_empty() || a.reqs.iter().any(|r| column(r.at, now).is_some());
    html(&Detail {
        icon: icon_html(a, 48),
        name: &a.name,
        id: &a.id,
        path: format!("/admin/apps/{}", form_urlencoded::byte_serialize(a.id.as_bytes()).collect::<String>()),
        group: a.group(),
        owner: a.owner().map(owner_html),
        requests: plural(a.reqs.len(), "request"),
        log: of_app(&a.id),
        running,
        limits,
        switch,
        chart: drawn.then(|| app_chart(a, marks, now)),
        routes: routes(a),
        kits: kits_part(shell, apps, a),
        risk: risk_part(shell, a),
        versions: versions(shell, a, all_versions),
        failures: last_failures(a),
        shown: SHOWN,
        power: path_icon(POWER_PATH, 16),
        warn: tone_icon(Tone::Warning, 16),
        critical: tone_icon(Tone::Critical, 16),
        changes,
    })
}

/// A kit it uses: through its own grants, or through another kit's.
struct Used {
    offer: Offer,
    /// The grants it is used through.
    grants: Vec<String>,
    /// The kit whose grants those are, if not the app's own.
    through: Option<String>,
}

/// The kits behind `grants`, then the kits those call, each once.
fn used(kits: &Kits, grants: &[String]) -> Vec<Used> {
    let mut all: Vec<Used> = vec![];
    let add = |all: &mut Vec<Used>, grant: &str, through: Option<&str>| {
        let Some(offer) = kits.offer(grant) else { return };
        match all.iter_mut().find(|u| u.offer.name == offer.name) {
            Some(u) if u.through.as_deref() == through => u.grants.push(grant.to_string()),
            Some(_) => {}
            None => all.push(Used { offer, grants: vec![grant.to_string()], through: through.map(str::to_string) }),
        }
    };
    for g in grants {
        add(&mut all, g, None);
    }
    let mut i = 0;
    while i < all.len() {
        let (calls, via) = (all[i].offer.calls.clone(), all[i].offer.name.clone());
        for g in &calls {
            add(&mut all, g, Some(&via));
        }
        i += 1;
    }
    all
}

/// Whether it serves now, or what keeps it from serving.
fn serving(off: bool, up: bool, waits: Option<&str>) -> String {
    match (up, off, waits) {
        (false, ..) => badge(Tone::Critical, "Not loaded", false),
        (_, true, _) => badge(Tone::Warning, "Off", false),
        (_, _, Some(kit)) => badge(Tone::Warning, &format!("Waits for {kit}"), false),
        _ => badge(Tone::Success, "Serves", false),
    }
}

/// The kits it uses, by the grants it holds and through those kits; for a
/// kit, also what uses it, which waits while it is off.
fn kits_part(shell: &Shell, apps: &[App], a: &App) -> Option<KitsPart> {
    let r = a.running.as_ref()?;
    let (kits, live) = (shell.kits.now(), shell.state.live(None));
    let name_of = |id: &str| apps.iter().find(|x| x.id == id).map_or(id, |x| x.name.as_str()).to_string();
    let uses = used(&kits, &r.manifest.capabilities.grant)
        .into_iter()
        .map(|u| {
            let o = &u.offer;
            let (name, from) = match &o.wasm {
                Some(id) => {
                    let kit = apps.iter().find(|x| x.id == *id);
                    let live = kit.and_then(|k| k.versions.iter().find(|v| v.status == VersionStatus::Live)).map(|v| format!("Deployed, {} live", v.version));
                    let folder = kit.and_then(App::folder_version).map(|v| format!("From its folder, {v}"));
                    (Link::App(id.clone(), name_of(id)), live.or(folder).unwrap_or_else(|| "Deployed, none live".into()))
                }
                None => (Link::Code(o.name.clone()), if o.builtin { "Built into AppOS" } else { "In the platform" }.to_string()),
            };
            let off = o.wasm.as_deref().is_some_and(|id| shell.state.is_off(id));
            let now = serving(off, o.up, live.kit_waits(&kits, &o.calls).as_deref());
            Uses { name, from, through: u.through.as_deref().map(name_of), grants: u.grants, now }
        })
        .collect();
    let users = r.manifest.kit.as_ref().map(|offers| {
        let offered: Vec<&str> = offers.grants.iter().map(|g| g.name.as_str()).collect();
        apps.iter()
            .filter(|b| b.id != a.id)
            .filter_map(|b| {
                let grants: Vec<String> = b.running.as_ref()?.manifest.capabilities.grant.iter().filter(|g| offered.contains(&g.as_str())).cloned().collect();
                (!grants.is_empty()).then(|| User {
                    id: b.id.clone(),
                    name: b.name.clone(),
                    kind: if b.is_kit() { "Kit" } else { "App" },
                    grants,
                    now: serving(b.off.is_some(), true, b.waits.as_ref().map(|(k, _)| k.as_str())),
                })
            })
            .collect()
    });
    Some(KitsPart { uses, users })
}

/// Per route, the last hour, all versions.
fn routes(a: &App) -> Routes {
    let mut routes: Vec<(String, Vec<&Request>)> = a.running.iter().flat_map(|r| r.routes.iter().map(|x| (x.pattern(), vec![]))).collect();
    for r in &a.reqs {
        match routes.iter_mut().find(|(p, _)| **p == *r.route) {
            Some((_, reqs)) => reqs.push(r),
            None => routes.push((r.route.to_string(), vec![r])),
        }
    }
    if routes.is_empty() {
        return Routes::None(a.versions.iter().any(|v| matches!(v.status, VersionStatus::Retired | VersionStatus::Failed)));
    }
    let row = |(route, reqs): (String, Vec<&Request>)| {
        let s = stats(&reqs);
        let ran = s.requests > s.failures;
        let or_dash = |v: String| if ran { v } else { "—".to_string() };
        Route {
            route,
            requests: thousands(s.requests as u64),
            bad: s.failures > 0,
            failures: thousands(s.failures as u64),
            median: or_dash(ms(s.wall_us.0)),
            p95: ran.then(|| ms(s.wall_us.1)),
            max: reqs.iter().filter(|r| !r.outcome.failed()).map(|r| r.wall_us).max().map_or("—".to_string(), ms),
            fuel: or_dash(fuel(s.fuel.1)),
            io: or_dash(s.io.1.to_string()),
        }
    };
    Routes::Some(routes.into_iter().map(row).collect())
}

/// Each grant with what it lets the app do and its level, riskiest first;
/// the ones that set the app's risk are tinted. Since: when the risk file
/// gave the grant its level, linked to that change in the log.
fn risk_part(shell: &Shell, a: &App) -> RiskPart {
    let changes = of_app_kinds(PLATFORM, Pick::One(Kind::RiskChanged));
    let Some(r) = &a.running else { return RiskPart { why: None, rows: vec![], changes } };
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
    let since = risk_changes(shell);
    let kits = shell.kits.now();
    let rows = grants
        .iter()
        .map(|(g, l)| GrantRow {
            grant: g.to_string(),
            chip: grant_chip(*l),
            allows: kits.allows(g).to_string(),
            sets: *l == level,
            since: since.get(*g).map(|e| Since { href: from_event(e.id, Some(PLATFORM)), full: local(e.at, FULL), clock: clock(e.at) }),
        })
        .collect();
    RiskPart { why: Some(why), rows, changes }
}

/// The live version first and marked, then the others newest first: when
/// each was deployed, how long it took to go live, why one failed or was
/// refused, and "Make live"; none when it was loaded from a folder.
fn versions(shell: &Shell, a: &App, all: bool) -> Option<Versions> {
    if a.versions.is_empty() {
        return None;
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
    let mut shown: Vec<&Version> = versions.iter().collect();
    shown.sort_by_key(rank);
    if !all {
        shown.truncate(SHOWN);
    }
    let live = a.running.as_ref().map(|r| r.label.as_str());
    let rows = shown
        .iter()
        .map(|v| VersionRow {
            label: v.label.clone(),
            badge: badge(Tone::of_version(v.status), &capitalize(v.status.as_str()), false),
            full: local(v.deployed_at, FULL),
            clock: clock(v.deployed_at),
            // A refused or waiting version never went live.
            took: if matches!(v.status, VersionStatus::Refused | VersionStatus::Waiting) {
                "—".to_string()
            } else {
                duration(v.took_ms.max(0) as u64 * 1000)
            },
            live: v.status == VersionStatus::Live,
            // Making a waiting version live is what ends a pause: the one
            // thing to do here then.
            make_live: (matches!(v.status, VersionStatus::Retired | VersionStatus::Failed | VersionStatus::Waiting) && live != Some(v.label.as_str()))
                .then_some(v.status == VersionStatus::Waiting),
            reason: v.reason.clone(),
            line: v.reason.as_deref().map(|r| capitalize(first_line(r))).unwrap_or_default(),
        })
        .collect();
    let more = if all && versions.len() > SHOWN {
        More::Newest
    } else if versions.len() > SHOWN {
        More::All(if versions.len() < VERSIONS as usize { format!("all {}", versions.len()) } else { "all".to_string() })
    } else {
        More::Nothing
    };
    Some(Versions { hold: a.paused.as_ref().map(|e| (clock(e.at), e.reason.clone())), rows, more })
}

/// Its last five failed requests.
fn last_failures(a: &App) -> Vec<Failure> {
    a.reqs
        .iter()
        .filter(|r| r.outcome.failed())
        .take(5)
        .map(|r| Failure {
            full: local(r.at, FULL),
            time: local(r.at, "%H:%M:%S"),
            route: r.route.to_string(),
            badge: badge(Tone::of_outcome(r.outcome), kind_label(r.outcome), false),
            label: r.label.to_string(),
            error: r.error.clone(),
        })
        .collect()
}
