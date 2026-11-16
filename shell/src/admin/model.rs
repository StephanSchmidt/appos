//! What the admin screen shows: every app with its requests, versions and
//! problems, read from the shell and platform.db.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::log_filter::CHANGES;
use crate::manifest::{Band, MAX_RISK, Risk, SECONDS};
use crate::platform::{Actor, Event, Kind, Outcome, Request, Version, VersionStatus};
use crate::runtime::{AdminLimits, Budget, IO_BUDGET, LoadedApp};
use crate::shell::Shell;

use super::format::{and, capitalize, clock, first_line, plural, uncapitalize};
use super::style::{FROZEN_PATH, HELD_PATH, KINDS, LIMIT_PATH, LIVE_PATH, POLICY_PATH, POWER_PATH, REFUSED_PATH, RISK_PATH, ROLLBACK_PATH, SEEN_PATH};
use super::tokens::{Oklch, monograms};

pub(super) const MINUTE: i64 = 60_000;
pub(super) const HOUR: i64 = 60 * MINUTE;
pub(super) const DAY: i64 = 24 * HOUR;
/// The newest versions of each app the page reads; all of them on request.
pub(super) const VERSIONS: u32 = 100;
/// A limit counts as close from this share of it on.
pub(super) const CLOSE: f64 = 0.8;

/// An app as the admin screen knows it: running, deployed, or both.
pub(super) struct App {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) running: Option<Arc<LoadedApp>>,
    /// Its requests in the last hour, all versions, newest first.
    pub(super) reqs: Vec<Request>,
    /// Newest first.
    pub(super) versions: Vec<Version>,
    /// The automatic rollback that paused it: deploys wait until someone
    /// makes a version live here.
    pub(super) paused: Option<Event>,
    /// The event that turned it off, while it is off.
    pub(super) off: Option<Event>,
    /// The newest change: deployed, rolled back, turned off or on.
    pub(super) changed: Option<Event>,
    /// Deploys the shell refused, ever.
    pub(super) refused: u32,
    /// What needs attention, most pressing first.
    pub(super) problems: Vec<Problem>,
    /// The newest time an admin marked its problems seen.
    pub(super) seen: Option<Event>,
    /// Its monogram tile's colour.
    pub(super) tile: Oklch,
    /// What an admin set for its requests.
    pub(super) limits: AdminLimits,
}

names! {
    /// What can need an admin about an app. A "seen" event names them.
    pub(super) enum ProblemKind {
        RolledBack = "rolled-back",
        TurnedOff = "turned-off",
        NotRunning = "not-running",
        Failing = "failing",
        Busy = "busy",
        NearLimit = "near-limit",
        Unrated = "unrated",
        Refused = "refused",
    }
}

impl ProblemKind {
    /// One or two words, never wrapping: past tense for what happened
    /// ("Rolled back"), state words for states ("Failing").
    pub(super) fn chip(self) -> &'static str {
        match self {
            ProblemKind::RolledBack => "Rolled back",
            ProblemKind::TurnedOff => "Turned off",
            ProblemKind::NotRunning => "Not running",
            ProblemKind::Failing => "Failing",
            ProblemKind::Busy => "Busy",
            ProblemKind::NearLimit => "Near limit",
            ProblemKind::Unrated => "Risk unrated",
            ProblemKind::Refused => "Deploy refused",
        }
    }

    /// The same in the console's narrow status column.
    pub(super) fn word(self) -> &'static str {
        match self {
            ProblemKind::RolledBack => "Rollback",
            ProblemKind::TurnedOff => "Off",
            ProblemKind::NotRunning => "Down",
            ProblemKind::Failing => "Failing",
            ProblemKind::Busy => "Busy",
            ProblemKind::NearLimit => "Near lim",
            ProblemKind::Unrated => "Unrated",
            ProblemKind::Refused => "Refused",
        }
    }
}

pub(super) struct Problem {
    pub(super) kind: ProblemKind,
    pub(super) tone: Tone,
    /// A sentence for the summary, which puts the app's name beside it.
    pub(super) text: String,
    /// The reason without the name, for the app's tile.
    pub(super) line: String,
    /// When it began, as far as the platform knows: a "seen" after it
    /// covers it.
    pub(super) since: i64,
}

/// How a status reads, the same everywhere (Polaris's tones), most pressing
/// first: critical is broken now, warning hurts users (requests turned
/// away), caution needs a look before it does, info is news, success is
/// fine, neutral is nothing going on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Tone {
    Critical,
    Warning,
    Caution,
    Info,
    Success,
    Neutral,
}

impl Tone {
    pub(super) const ALL: [Tone; 6] = [Tone::Critical, Tone::Warning, Tone::Caution, Tone::Info, Tone::Success, Tone::Neutral];

    pub(super) fn name(self) -> &'static str {
        match self {
            Tone::Critical => "critical",
            Tone::Warning => "warning",
            Tone::Caution => "caution",
            Tone::Info => "info",
            Tone::Success => "success",
            Tone::Neutral => "neutral",
        }
    }

    /// Of a version's status in platform.db.
    pub(super) fn of_version(status: VersionStatus) -> Tone {
        match status {
            VersionStatus::Live => Tone::Success,
            VersionStatus::Failed => Tone::Critical,
            VersionStatus::Refused | VersionStatus::Waiting => Tone::Caution,
            VersionStatus::Retired => Tone::Neutral,
        }
    }

    /// Of a request's outcome: turned away as busy hurts users, the other
    /// failures are broken requests.
    pub(super) fn of_outcome(outcome: Outcome) -> Tone {
        match outcome {
            Outcome::Ok | Outcome::ClientError | Outcome::Denied => Tone::Success,
            Outcome::Busy => Tone::Warning,
            _ => Tone::Critical,
        }
    }

    /// Of an event's kind.
    pub(super) fn of_event(kind: Kind) -> Tone {
        match kind {
            Kind::AutoRolledBack => Tone::Critical,
            Kind::LimitHit => Tone::Warning,
            Kind::Refused => Tone::Caution,
            Kind::RolledBack => Tone::Info,
            Kind::Deployed => Tone::Success,
            Kind::RiskChanged => Tone::Neutral,
            Kind::PolicyChanged => Tone::Neutral,
            Kind::PolicyRefused => Tone::Caution,
            Kind::Held => Tone::Caution,
            Kind::TurnedOff => Tone::Warning,
            Kind::TurnedOn => Tone::Success,
            Kind::Seen => Tone::Neutral,
            Kind::Frozen => Tone::Caution,
            Kind::Unfrozen => Tone::Success,
            Kind::LimitsSet => Tone::Info,
        }
    }
}

impl App {
    /// Who to ask about it: the running manifest, then the live or newest
    /// stored version that names an owner.
    pub(super) fn owner(&self) -> Option<&str> {
        let live = self.versions.iter().find(|v| v.status == VersionStatus::Live);
        self.running
            .as_ref()
            .and_then(|a| a.manifest.app.owner.as_deref())
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .or_else(|| live.into_iter().chain(&self.versions).map(|v| v.owner.as_str()).find(|o| !o.is_empty()))
    }

    /// Its group from the running manifest, then the live or newest
    /// stored version that names one.
    pub(super) fn group(&self) -> Option<&str> {
        let live = self.versions.iter().find(|v| v.status == VersionStatus::Live);
        self.running
            .as_ref()
            .and_then(|a| a.manifest.app.group.as_deref())
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .or_else(|| live.into_iter().chain(&self.versions).map(|v| v.group.as_str()).find(|g| !g.is_empty()))
    }

    /// One line for the tile: the most pressing reason, or what the app
    /// did in the last hour.
    pub(super) fn line(&self) -> String {
        match self.problems.first() {
            Some(p) => p.line.clone(),
            None if self.running.is_none() => "No live version".into(),
            None if self.reqs.is_empty() => "No requests in the last hour".into(),
            None => format!("{} in the last hour", plural(self.reqs.len(), "request")),
        }
    }

    /// The time app.toml asks for, or the default.
    pub(super) fn toml_time(&self) -> Duration {
        self.running.as_ref().map_or(Duration::from_secs(SECONDS), |a| a.manifest.time_limit())
    }

    /// What one of its requests may use, with what an admin set.
    pub(super) fn budget(&self) -> Budget {
        Budget::with(self.toml_time(), self.limits)
    }

    pub(super) fn limit_us(&self) -> u64 {
        self.budget().time.as_micros() as u64
    }

    /// The requests the running version served.
    pub(super) fn live(&self) -> Vec<&Request> {
        let label = self.running.as_ref().map(|a| a.label.as_str());
        self.reqs.iter().filter(|r| Some(&*r.label) == label).collect()
    }
}

/// The platform's state for the favicon: the most pressing tone of what
/// needs an admin; what they saw no longer counts.
pub(super) fn state(apps: &[App]) -> Tone {
    apps.iter().map(App::first_tone).min().unwrap_or(Tone::Success)
}

/// Every app, needs-you first, and the newest events. One query per kind
/// of data, not per app; it reads platform.db, so call it off the async
/// workers (`super::read`).
pub(super) fn load(shell: &Shell, now: i64) -> (Vec<App>, Vec<Event>) {
    let db = &shell.platform;
    let running = shell.state.apps();
    let mut reqs = db.requests(now - HOUR).unwrap_or_default();
    let events = db.events(None, 200).unwrap_or_default();
    let mut versions = db.versions_of_all(VERSIONS).unwrap_or_default();
    let mut paused = db.paused_all().unwrap_or_default();
    let mut off = db.off_all().unwrap_or_default();
    let mut changed = db.newest_of(&CHANGES).unwrap_or_default();
    let mut seen = db.newest_of(&[Kind::Seen]).unwrap_or_default();
    let refused = db.count_versions(VersionStatus::Refused).unwrap_or_default();
    let mut ids: Vec<String> = running.keys().cloned().collect();
    ids.extend(versions.keys().filter(|id| !running.contains_key(*id)).cloned());
    let mut sorted: Vec<&str> = ids.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let tiles: BTreeMap<String, Oklch> = sorted.iter().map(|id| id.to_string()).zip(monograms(sorted.iter().copied())).collect();
    let mut apps: Vec<App> = ids
        .into_iter()
        .map(|id| {
            let running = running.get(&id).cloned();
            let versions = versions.remove(&id).unwrap_or_default();
            let name = match (&running, versions.first()) {
                (Some(a), _) => a.manifest.app.name.clone(),
                (None, Some(v)) => v.name.clone(),
                (None, None) => id.clone(),
            };
            let mut app = App {
                tile: tiles.get(&id).copied().unwrap_or_else(|| monograms([id.as_str()])[0]),
                paused: paused.remove(&id),
                off: off.remove(&id),
                changed: changed.remove(&id),
                refused: refused.get(&id).copied().unwrap_or(0),
                reqs: reqs.remove(&id).unwrap_or_default(),
                seen: seen.remove(&id),
                limits: shell.state.limits_of(&id),
                id,
                name,
                running,
                versions,
                problems: vec![],
            };
            app.problems = problems(&app, &shell.risk, &events, now);
            app
        })
        .collect();
    apps.sort_by_cached_key(|a| (a.urgency(), a.name.to_lowercase()));
    (apps, events)
}

/// Per grant, when risk.toml last gave it its level.
pub(super) fn risk_changes(shell: &Shell) -> BTreeMap<String, Event> {
    let changes = shell.platform.events_since(Some(""), 0, &[Kind::RiskChanged]).unwrap_or_default();
    let mut out = BTreeMap::new();
    for e in changes {
        // "kv-write: risk 3 → 6", oldest first: the newest stays.
        if let Some((grant, _)) = e.message.split_once(':') {
            out.insert(grant.to_string(), e);
        }
    }
    out
}

pub(super) fn problems(app: &App, risk: &Risk, events: &[Event], now: i64) -> Vec<Problem> {
    let newest = |kind: Kind| events.iter().find(|e| e.app_id == app.id && e.kind == kind && e.at >= now - DAY);
    let why = |e: &Event| capitalize(e.reason.as_deref().unwrap_or(&e.message));
    let mut out = vec![];
    // Paused until someone makes a version live, however long ago.
    if let Some(e) = &app.paused {
        let text = match app.versions.iter().find(|v| v.status == VersionStatus::Waiting) {
            Some(w) => format!("Rolled itself back at {}; {} waits until someone makes it live", clock(e.at), w.version),
            None => format!("Rolled itself back at {}; deploys wait until someone makes a version live", clock(e.at)),
        };
        out.push(Problem { kind: ProblemKind::RolledBack, tone: Tone::Critical, text, line: why(e), since: e.at });
    }
    // Turned off on purpose, but visitors are turned away.
    if let Some(e) = &app.off {
        let text = format!("Turned off at {} {}; visitors get a page saying so", clock(e.at), by(e));
        let line = e.reason.as_deref().map_or_else(|| format!("Turned off at {}", clock(e.at)), capitalize);
        out.push(Problem { kind: ProblemKind::TurnedOff, tone: Tone::Warning, text, line, since: e.at });
    }
    // Since the live version went live: a new one is news.
    let deployed = app.versions.iter().find(|v| v.status == VersionStatus::Live).map_or(0, |v| v.deployed_at);
    if app.running.is_none() && app.versions.iter().any(|v| v.status == VersionStatus::Live) {
        let text = "The live version is not running; see the shell's output".to_string();
        let line = "The live version didn't start; see the shell's output".into();
        out.push(Problem { kind: ProblemKind::NotRunning, tone: Tone::Critical, text, line, since: deployed });
    }
    let live = app.live();
    let recent: Vec<&&Request> = live.iter().filter(|r| r.at >= now - 15 * MINUTE).collect();
    let fails = recent.iter().filter(|r| r.outcome.failed() && r.outcome != Outcome::Busy).count();
    if fails > 0 {
        let text = format!("Failing: {} in the last 15 minutes", plural(fails, "failed request"));
        let since = began(&live, |r| r.outcome.failed() && r.outcome != Outcome::Busy);
        out.push(Problem { kind: ProblemKind::Failing, tone: Tone::Critical, text, line: format!("{} in 15 minutes", plural(fails, "failed request")), since });
    }
    let busy = recent.iter().filter(|r| r.outcome == Outcome::Busy).count();
    if busy > 0 {
        let text = format!("Turned {} away as busy in the last 15 minutes", plural(busy, "request"));
        let since = began(&live, |r| r.outcome == Outcome::Busy);
        out.push(Problem { kind: ProblemKind::Busy, tone: Tone::Warning, text, line: format!("{} turned away in 15 minutes", plural(busy, "request")), since });
    }
    if app.running.is_some() {
        let close = close_to_limits(&stats(&live), app.budget());
        if !close.is_empty() {
            let text = format!("Close to its limits: {}", close.join(", "));
            out.push(Problem { kind: ProblemKind::NearLimit, tone: Tone::Caution, text, line: capitalize(&close.join(", ")), since: deployed });
        }
    }
    // Unrated: nobody knows how risky it is until the risk file rates
    // every grant it uses.
    if let Some(running) = &app.running {
        let unrated: Vec<&str> = running.manifest.capabilities.grant.iter().filter(|g| risk.of(g).is_none()).map(String::as_str).collect();
        if !unrated.is_empty() {
            let line = format!("The risk file has no level for {}", and(&unrated));
            let text = format!("Risk unknown: {}", uncapitalize(&line));
            out.push(Problem { kind: ProblemKind::Unrated, tone: Tone::Caution, text, line, since: deployed });
        }
    }
    // A refusal matters until a deploy after it gets through: while the
    // newest version is the refused one.
    let still_refused = app.versions.first().is_some_and(|v| v.status == VersionStatus::Refused);
    if let Some(e) = newest(Kind::Refused).filter(|_| still_refused) {
        // Whether visitors still get the app is what the operator asks next.
        let never = app.running.is_none() && !app.versions.iter().any(|v| v.status == VersionStatus::Live);
        let (then, line) = match () {
            _ if app.running.is_some() => ("; the live version keeps running", why(e)),
            _ if never => ("; it isn't live yet", format!("Not live yet. {}", why(e))),
            _ => ("", why(e)),
        };
        let text = format!("A deploy was refused at {}{then}", clock(e.at));
        out.push(Problem { kind: ProblemKind::Refused, tone: Tone::Caution, text, line, since: e.at });
    }
    out.sort_by_key(|p| p.tone);
    out
}

/// When the newest run of requests like `like` began: the oldest of them
/// with no 15-minute gap to the next, among those kept (newest first).
fn began(reqs: &[&Request], like: impl Fn(&Request) -> bool) -> i64 {
    let mut since = i64::MAX;
    for r in reqs.iter().filter(|r| like(r)) {
        if since != i64::MAX && since - r.at > 15 * MINUTE {
            break;
        }
        since = r.at;
    }
    since
}

/// What a "seen" event saw: "Seen: failing, busy", the kinds' names.
const SEEN_PREFIX: &str = "Seen: ";

/// A "seen" event's message for these problems; none brings them back to
/// the inbox.
pub(super) fn seen_message(kinds: &[ProblemKind]) -> String {
    if kinds.is_empty() {
        return "Needs you again".to_string();
    }
    let names: Vec<&str> = kinds.iter().map(|k| k.as_str()).collect();
    format!("{SEEN_PREFIX}{}", names.join(", "))
}

/// The problems a "seen" event saw.
pub(super) fn seen_kinds(e: &Event) -> Vec<ProblemKind> {
    e.message.strip_prefix(SEEN_PREFIX).map_or_else(Vec::new, |m| m.split(", ").filter_map(ProblemKind::parse).collect())
}

impl Problem {
    /// An admin saw it: a "seen" since it began that names it.
    pub(super) fn seen_in(&self, seen: Option<&Event>) -> bool {
        seen.is_some_and(|e| e.at >= self.since && seen_kinds(e).contains(&self.kind))
    }
}

impl App {
    /// Its problems no admin has seen yet, most pressing first.
    pub(super) fn unseen(&self) -> impl Iterator<Item = &Problem> {
        self.problems.iter().filter(|p| !p.seen_in(self.seen.as_ref()))
    }

    /// Something it needs an admin for that nobody has seen.
    pub(super) fn needs_you(&self) -> bool {
        self.unseen().next().is_some()
    }

    /// The tone of the most pressing problem nobody has seen.
    pub(super) fn first_tone(&self) -> Tone {
        self.unseen().next().map_or(Tone::Neutral, |p| p.tone)
    }

    /// How pressing it is, to sort by, the same everywhere: what needs you
    /// first, the most pressing of those first.
    pub(super) fn urgency(&self) -> (bool, Tone) {
        (!self.needs_you(), self.first_tone())
    }
}

/// An event's word, the same everywhere: "Made live" is a deploy by an
/// admin (a held version going live).
pub(super) fn word(e: &Event) -> &'static str {
    match e.kind {
        Kind::Deployed if e.actor == Actor::Admin => "Made live",
        Kind::Deployed => "Deployed",
        Kind::Refused => "Refused",
        Kind::RolledBack => "Rolled back",
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
    }
}

/// An event's icon, beside its word.
pub(super) fn icon(e: &Event) -> &'static str {
    match e.kind {
        Kind::Deployed => LIVE_PATH,
        Kind::Refused => REFUSED_PATH,
        Kind::RolledBack | Kind::AutoRolledBack => ROLLBACK_PATH,
        Kind::LimitHit | Kind::LimitsSet => LIMIT_PATH,
        Kind::RiskChanged => RISK_PATH,
        Kind::PolicyChanged | Kind::PolicyRefused => POLICY_PATH,
        Kind::Held => HELD_PATH,
        Kind::TurnedOff | Kind::TurnedOn => POWER_PATH,
        Kind::Seen => SEEN_PATH,
        Kind::Frozen | Kind::Unfrozen => FROZEN_PATH,
    }
}

/// What it says: the message, with the problems a "seen" saw in words.
pub(super) fn message(e: &Event) -> String {
    match e.kind {
        Kind::Seen if !seen_kinds(e).is_empty() => {
            let chips: Vec<&str> = seen_kinds(e).iter().map(|k| k.chip()).collect();
            format!("Seen: {}", chips.join(", "))
        }
        _ => e.message.clone(),
    }
}

/// An event in a sentence, the same on every page: "Calculator 0.2.0
/// deployed, live in 12 ms", "Calculator turned off by an admin". Without
/// a name, of the app at hand. Its reason follows after a colon.
pub(super) fn sentence(e: &Event, name: Option<&str>) -> String {
    let message = uncapitalize(&message(e));
    // The version, where the message doesn't name it.
    let version = match e.kind {
        Kind::Deployed | Kind::Refused | Kind::Held | Kind::LimitHit if e.actor != Actor::Admin => format!("{} ", ver(&e.label)),
        _ => String::new(),
    };
    // Who, where it isn't the shell on its own; after a comma when the
    // message has one ("made live, replacing 0.2.0, by an admin").
    let who = match e.kind {
        Kind::AutoRolledBack | Kind::LimitHit | Kind::RiskChanged | Kind::PolicyChanged | Kind::PolicyRefused | Kind::Refused | Kind::Held => String::new(),
        Kind::Deployed if e.actor != Actor::Admin => String::new(),
        _ if message.contains(',') => format!(", {}", by(e)),
        _ => format!(" {}", by(e)),
    };
    let what = match name {
        Some(n) => format!("{n} {version}{message}{who}"),
        None => capitalize(&format!("{version}{message}{who}")),
    };
    match &e.reason {
        Some(r) => format!("{what}: {}", first_line(r).split_whitespace().collect::<Vec<_>>().join(" ")),
        None => what,
    }
}

/// A version label's version: "0.3.1" of "0.3.1-ab12…".
pub(super) fn ver(label: &str) -> &str {
    label.rsplit_once('-').map_or(label, |(v, _)| v)
}

/// Who did it, after the verb: "by ann@acme.com", "by an admin" when no
/// one was signed in.
pub(super) fn by(e: &Event) -> String {
    match (e.actor, &e.person) {
        (Actor::Admin, Some(person)) => format!("by {person}"),
        (Actor::Admin, None) => "by an admin".into(),
        (Actor::Cli, _) => "with the appos CLI".into(),
        (Actor::Shell, _) => "by the shell".into(),
    }
}

/// Counts per kind of failure, in the order of KINDS.
pub(super) fn by_kind(reqs: &[&Request]) -> Vec<u32> {
    KINDS.iter().map(|(o, _, _)| reqs.iter().filter(|r| r.outcome == *o).count() as u32).collect()
}

/// A failed request's outcome in words, like "Timed out".
pub(super) fn kind_label(outcome: Outcome) -> &'static str {
    KINDS.iter().find(|(o, _, _)| *o == outcome).map_or(outcome.as_str(), |(_, label, _)| label)
}

/// Like "4 timed out, 1 crashed".
pub(super) fn kinds_text(counts: &[u32]) -> String {
    let parts: Vec<String> = KINDS.iter().zip(counts).filter(|(_, n)| **n > 0).map(|((_, label, _), n)| format!("{n} {}", label.to_lowercase())).collect();
    parts.join(", ")
}

/// The value at quantile `q` of `values`.
pub(super) fn quantile(values: &mut [u64], q: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[((values.len() - 1) as f64 * q).round() as usize]
}

pub(super) struct Stats {
    pub(super) requests: usize,
    pub(super) failures: usize,
    /// (p50, p95) of each measure, over the requests that didn't fail.
    pub(super) wall_us: (u64, u64),
    pub(super) fuel: (u64, u64),
    pub(super) io: (u64, u64),
}

pub(super) fn stats(reqs: &[&Request]) -> Stats {
    let pick = |f: fn(&Request) -> u64| {
        // Failed requests often stop at a limit; they are counted as failures.
        let mut ran: Vec<u64> = reqs.iter().filter(|r| !r.outcome.failed()).map(|r| f(r)).collect();
        (quantile(&mut ran, 0.5), quantile(&mut ran, 0.95))
    };
    Stats {
        requests: reqs.len(),
        failures: reqs.iter().filter(|r| r.outcome.failed()).count(),
        wall_us: pick(|r| r.wall_us),
        fuel: pick(|r| r.fuel),
        io: pick(|r| r.io as u64),
    }
}

/// Limits the app is close to, at the 95th percentile.
pub(super) fn close_to_limits(s: &Stats, b: Budget) -> Vec<String> {
    let mut out = vec![];
    let limit_us = b.time.as_micros() as u64;
    for (what, p95, limit) in [("time", s.wall_us.1, limit_us), ("fuel", s.fuel.1, b.fuel), ("I/O units", s.io.1, IO_BUDGET as u64)] {
        if p95 as f64 >= limit as f64 * CLOSE {
            out.push(format!("{what} at {}% of its limit", p95 * 100 / limit.max(1)));
        }
    }
    out
}

pub(super) fn name_of<'a>(apps: &'a [App], id: &'a str) -> &'a str {
    apps.iter().find(|a| a.id == id).map_or(id, |a| a.name.as_str())
}

/// A running app's risk in words, like "low risk (2 of 10)"; none when it
/// isn't running.
pub(super) fn risk_of(risk: &Risk, a: &App) -> Option<String> {
    let r = a.running.as_ref()?;
    Some(match risk.of_app(&r.manifest) {
        Some(level) => format!("{} risk ({level} of {MAX_RISK})", Band::of(level).name().to_lowercase()),
        None => "risk unrated".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn startup_apps_keep_their_manifest_owner_and_group() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let checked = crate::testing::bundle("1", None).checked().unwrap();
        let app = crate::runtime::load(&shell.engine, shell.db.clone(), &checked).await.unwrap();
        shell.state.insert_loaded([Arc::new(app)]);
        let (apps, _) = load(&shell, crate::platform::now_ms());
        let app = apps.iter().find(|a| a.id == "test").unwrap();
        assert_eq!(app.owner(), Some("Tests"));
        assert_eq!(app.group(), Some("Tests"));
    }

    /// A "seen" names its problems by id and reads them back; none is
    /// "needs you again", which sees nothing.
    #[test]
    fn seen_names_the_problems() {
        let kinds = [ProblemKind::Failing, ProblemKind::Busy];
        let e = Event {
            id: 1,
            at: 0,
            app_id: "a".into(),
            label: String::new(),
            kind: Kind::Seen,
            severity: crate::platform::Severity::Info,
            actor: Actor::Admin,
            message: seen_message(&kinds),
            reason: None,
            risk: None,
            recovery_ms: None,
            outcome: None,
            before: None,
            person: None,
        };
        assert_eq!(e.message, "Seen: failing, busy");
        assert_eq!(seen_kinds(&e), kinds);
        assert_eq!(message(&e), "Seen: Failing, Busy");
        let again = Event { message: seen_message(&[]), ..e };
        assert_eq!(again.message, "Needs you again");
        assert!(seen_kinds(&again).is_empty());
    }

    #[test]
    fn tones_go_from_most_pressing_and_failures_are_never_fine() {
        assert!(Tone::ALL.windows(2).all(|w| w[0] < w[1]));
        for outcome in Outcome::ALL {
            assert_eq!(outcome.failed(), Tone::of_outcome(outcome) != Tone::Success, "{outcome}");
            assert_eq!(outcome.failed(), KINDS.iter().any(|(o, _, _)| *o == outcome), "{outcome}");
        }
        assert_eq!(kind_label(Outcome::Timeout), "Timed out");
        assert_eq!(Tone::of_version(VersionStatus::Live), Tone::Success);
        assert_eq!(Tone::of_event(Kind::AutoRolledBack), Tone::Critical);
    }

    #[test]
    fn levels_fall_into_fixed_bands() {
        let bands: Vec<Band> = (0..=MAX_RISK).map(Band::of).collect();
        use Band::*;
        assert_eq!(bands, [Low, Low, Low, Low, Medium, Medium, Medium, High, High, Critical, Critical]);
    }
}
