//! The platform's own database, separate from the apps' kv: the versions of
//! every app, what happened to them (events), what is switched now (an app
//! off or paused, deploys frozen), what each request cost (usage.rs) and
//! per hour, and the risk levels of the last start. SQLite here; nothing
//! outside this file knows that.
//!
//! Events and hours are kept for good; only requests are pruned. Pruning
//! anything else would be a platform job like the usage pruning, never part
//! of a request.
//!
//! A change of state writes its rows and its event in one transaction; the
//! runtime state in memory follows (state.rs).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, MutexGuard};
use rusqlite::{Connection, OptionalExtension, params};

use crate::manifest::Band;
use crate::runtime::AdminLimits;

/// Goes up with every change to SCHEMA. Unreleased, so there are no
/// migrations: an older platform.db is refused with a message to delete it.
const SCHEMA_VERSION: i32 = 9;

const SCHEMA: &str = r#"
-- Every deploy, refused ones too. At most one version per app is live.
CREATE TABLE IF NOT EXISTS versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id TEXT NOT NULL,
    label TEXT NOT NULL,
    version TEXT NOT NULL,
    name TEXT NOT NULL,
    owner TEXT NOT NULL,           -- from app.toml, like "Jane Doe <jane@example.com>"; '' if refused without one
    app_group TEXT NOT NULL,       -- from app.toml, like "Marketing"; '' if refused without one
    status TEXT NOT NULL,          -- VersionStatus
    reason TEXT,                   -- why it failed, was refused or rolled back
    deployed_at INTEGER NOT NULL,  -- unix ms
    took_ms INTEGER NOT NULL,      -- from receiving it to live, or refused
    grants TEXT NOT NULL,          -- JSON array, as deployed
    routes TEXT NOT NULL,          -- JSON array like "GET /", empty if refused
    packages TEXT                  -- JSON array of package URLs from deps.json; NULL without one
);
CREATE INDEX IF NOT EXISTS versions_app ON versions (app_id, id);

-- What happened, to an app or (app_id '') to the platform.
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    app_id TEXT NOT NULL,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,            -- Kind
    severity TEXT NOT NULL,        -- Severity
    actor TEXT NOT NULL,           -- Actor
    message TEXT NOT NULL,         -- what happened
    reason TEXT,                   -- why
    risk INTEGER,                  -- the app's risk then; NULL while unrated
    recovery_ms INTEGER,           -- auto-rolled-back: first failure to rolled back
    outcome TEXT,                  -- limit-hit: the Outcome of the requests behind it
    before TEXT,                   -- an admin's change: what it changed, for Undo (limits as JSON, the label that was live)
    person TEXT                    -- who did it: the email of the admin signed in, or of the CLI's token
);
CREATE INDEX IF NOT EXISTS events_app ON events (app_id, id);

-- What is switched now: an app turned off or paused after an automatic
-- rollback, deploys frozen (app_id ''). No row: not switched. The event
-- that switched it says when, by whom and why.
CREATE TABLE IF NOT EXISTS switches (
    app_id TEXT NOT NULL,
    switch TEXT NOT NULL,          -- Switch
    event_id INTEGER NOT NULL,
    stop_all INTEGER NOT NULL,     -- off by Stop all: Start all turns it on
    PRIMARY KEY (app_id, switch)
);

-- What each request cost. Only the newest rows per app are kept (usage.rs).
CREATE TABLE IF NOT EXISTS requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    app_id TEXT NOT NULL,
    label TEXT NOT NULL,
    route TEXT NOT NULL,
    outcome TEXT NOT NULL,         -- Outcome
    status INTEGER NOT NULL,
    wall_us INTEGER NOT NULL,
    queue_us INTEGER NOT NULL,
    fuel INTEGER NOT NULL,
    io INTEGER NOT NULL,
    runs INTEGER NOT NULL,
    error TEXT
);
CREATE INDEX IF NOT EXISTS requests_app ON requests (app_id, id);

-- Requests per app and hour, beyond the newest requests: a month and more.
-- Hours, not days, so a window of 24 hours ends now in any time zone.
CREATE TABLE IF NOT EXISTS hours (
    app_id TEXT NOT NULL,
    hour INTEGER NOT NULL,         -- unix ms / 3,600,000
    requests INTEGER NOT NULL,
    failures INTEGER NOT NULL,
    stops INTEGER NOT NULL,        -- stopped at a limit: timed out, over budget
    PRIMARY KEY (app_id, hour)
);

-- What an admin set for an app's requests on the console, over app.toml
-- and the defaults; NULL is the default. Kept across deploys.
CREATE TABLE IF NOT EXISTS limits (
    app_id TEXT PRIMARY KEY,
    memory_mb INTEGER,
    seconds INTEGER,
    fuel_m INTEGER                 -- millions of instructions
);

-- risk.toml as of the last start, to notice changes.
CREATE TABLE IF NOT EXISTS risk_levels (
    grant_name TEXT PRIMARY KEY,
    level INTEGER NOT NULL
);

-- Tokens for the platform API (appos login), made on the admin screen.
-- Only their SHA-256 is kept. A token acts as the person who made it.
CREATE TABLE IF NOT EXISTS tokens (
    hash TEXT PRIMARY KEY,         -- hex
    person TEXT NOT NULL,          -- JSON: who made it, as they were signed in
    made_at INTEGER NOT NULL,      -- unix ms
    expires_at INTEGER NOT NULL,
    used_at INTEGER
);
"#;

#[derive(Clone)]
pub struct PlatformDb(Arc<Mutex<Connection>>);

names! {
    /// Where a version stands.
    pub enum VersionStatus {
        /// Serving; at most one per app.
        Live = "live",
        /// Was live, and can be again.
        Retired = "retired",
        /// Rolled back automatically because it kept failing; it goes live
        /// again only by name.
        Failed = "failed",
        /// Refused at deploy and never stored.
        Refused = "refused",
        /// Stored while the app was paused or deploys frozen: it waits for
        /// an admin to make it live.
        Waiting = "waiting",
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Version {
    pub id: i64,
    pub app_id: String,
    pub label: String,
    pub version: String,
    pub name: String,
    /// Who to ask about it, as app.toml names them; empty for a deploy
    /// refused for having none.
    pub owner: String,
    /// The group it belongs to, as app.toml names it; empty for a deploy
    /// refused for having none.
    pub group: String,
    pub status: VersionStatus,
    pub reason: Option<String>,
    pub deployed_at: i64,
    pub took_ms: i64,
    /// What it was deployed with, to compare versions without loading them.
    pub grants: Vec<String>,
    pub routes: Vec<String>,
}

names! {
    /// What happened to an app, or to the platform.
    pub enum Kind {
        Deployed = "deployed",
        Refused = "refused",
        RolledBack = "rolled-back",
        AutoRolledBack = "auto-rolled-back",
        /// A request stopped at a limit or crashed (`outcome` says which);
        /// at most one per app, outcome and 10 minutes (usage.rs).
        LimitHit = "limit-hit",
        /// A grant's level in risk.toml changed between two starts.
        RiskChanged = "risk-changed",
        /// policy.csv changed, and the new one is in force.
        PolicyChanged = "policy-changed",
        /// policy.csv changed but has a mistake (`reason`): the policy
        /// stays as it was.
        PolicyRefused = "policy-refused",
        /// A deploy checked and stored while the app is paused after an
        /// automatic rollback, or deploys are frozen: it waits for an admin
        /// to make it live.
        Held = "held",
        /// The kill switch: the app answers with a "turned off" page and
        /// runs nothing, until it is turned on.
        TurnedOff = "turned-off",
        TurnedOn = "turned-on",
        /// An admin saw an app's problems (`message` lists them, "Seen:
        /// Failing; Busy"): they leave the console's inbox until something
        /// new happens.
        Seen = "seen",
        /// An admin froze deploys (app_id ""): new deploys are checked,
        /// stored and held until an admin makes them live; the CLI can't
        /// roll back.
        Frozen = "frozen",
        Unfrozen = "unfrozen",
        /// An admin changed an app's limits per request (`message` says
        /// from what to what).
        LimitsSet = "limits-set",
        /// A command an app sent failed at its kit every try (`message`
        /// names it, `reason` says the last error): it waits in the outbox
        /// for an admin (outbox.rs).
        MessageFailed = "message-failed",
        /// An admin had a failed command tried again.
        MessageRetried = "message-retried",
    }
}

names! {
    /// How much an event matters.
    #[derive(PartialOrd, Ord)]
    pub enum Severity {
        Info = "info",
        Low = "low",
        Medium = "medium",
        High = "high",
        Critical = "critical",
    }
}

impl Severity {
    /// The kind's base, one step up for an app at high or critical risk.
    pub fn of(kind: Kind, risk: Option<u8>) -> Self {
        let base = match kind {
            Kind::Deployed | Kind::TurnedOn | Kind::Seen | Kind::Unfrozen => Severity::Info,
            Kind::RolledBack | Kind::RiskChanged | Kind::PolicyChanged | Kind::LimitsSet | Kind::MessageRetried => Severity::Low,
            Kind::Refused | Kind::LimitHit | Kind::Held | Kind::TurnedOff | Kind::Frozen => Severity::Medium,
            Kind::AutoRolledBack | Kind::PolicyRefused | Kind::MessageFailed => Severity::High,
        };
        if risk.is_some_and(|r| Band::of(r) >= Band::High) { base.up() } else { base }
    }

    fn up(self) -> Self {
        match self {
            Severity::Info => Severity::Low,
            Severity::Low => Severity::Medium,
            Severity::Medium => Severity::High,
            Severity::High | Severity::Critical => Severity::Critical,
        }
    }
}

names! {
    /// Who did it: the shell on its own, the appos CLI, or the admin
    /// screen (with the person signed in there, `person`, if any).
    pub enum Actor {
        Shell = "shell",
        Cli = "cli",
        Admin = "admin",
    }
}

names! {
    /// How a request ended.
    pub enum Outcome {
        /// A page or a redirect.
        Ok = "ok",
        /// The app answered with a 4xx: the visitor asked for something
        /// wrong, the app works.
        ClientError = "4xx",
        /// The app answered with a 5xx error page.
        ServerError = "5xx",
        /// It took longer than its time limit.
        Timeout = "timeout",
        /// It ran out of fuel, memory or I/O units.
        OverBudget = "over-budget",
        /// Other requests kept changing what it read.
        Conflict = "conflict",
        /// The app trapped or panicked.
        Crashed = "crashed",
        /// Turned away: it waited too long for the busy app.
        Busy = "busy",
        /// The policy doesn't let this person make a host call the app
        /// made (access.rs): sent to sign in, or shown a 403. Not the
        /// app's failure.
        Denied = "denied",
    }
}

impl Outcome {
    /// Of a page the app answered with, by its HTTP status.
    pub fn of_status(status: u16) -> Outcome {
        match status {
            ..400 => Outcome::Ok,
            400..500 => Outcome::ClientError,
            _ => Outcome::ServerError,
        }
    }

    /// A request failed: anything but a page, a redirect, a 4xx or a denial.
    pub fn failed(self) -> bool {
        !matches!(self, Outcome::Ok | Outcome::ClientError | Outcome::Denied)
    }

    /// A request stopped at a limit.
    pub fn stopped(self) -> bool {
        matches!(self, Outcome::Timeout | Outcome::OverBudget)
    }

    /// What a limit-hit event calls it, "Timed out on POST /signup"; none
    /// for outcomes that get no event.
    pub fn limit_hit(self) -> Option<&'static str> {
        match self {
            Outcome::Timeout => Some("Timed out"),
            Outcome::OverBudget => Some("Over budget"),
            Outcome::Crashed => Some("Crashed"),
            _ => None,
        }
    }

    /// The failures that roll a version back when they keep coming: those
    /// that get a limit-hit event. A conflict comes from load, a busy app
    /// from too many visitors and an error page from the app's own choice,
    /// not from a broken version.
    pub fn counts_toward_rollback(self) -> bool {
        self.limit_hit().is_some()
    }
}

names! {
    /// What can be switched (the switches table).
    enum Switch {
        Off = "off",
        Paused = "paused",
        Frozen = "frozen",
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    /// In the order written, so newer events have higher ids.
    pub id: i64,
    pub at: i64,
    /// Empty for the platform itself.
    pub app_id: String,
    pub label: String,
    pub kind: Kind,
    pub severity: Severity,
    pub actor: Actor,
    /// What happened.
    pub message: String,
    pub reason: Option<String>,
    /// The app's risk then; none while unrated.
    pub risk: Option<u8>,
    /// Of an automatic rollback: from the first failure to rolled back.
    pub recovery_ms: Option<i64>,
    /// Of a limit hit: how the requests behind it ended.
    pub outcome: Option<Outcome>,
    /// Of an admin's change: what it changed, for Undo: the limits before
    /// (JSON), the label that was live before.
    pub before: Option<String>,
    /// Who did it: the email of the admin signed in, or of the CLI's token.
    pub person: Option<String>,
}

/// An event to write, now; its severity follows from kind and risk.
pub struct NewEvent<'a> {
    pub app_id: &'a str,
    pub label: &'a str,
    pub kind: Kind,
    pub actor: Actor,
    pub message: &'a str,
    pub reason: Option<&'a str>,
    pub risk: Option<u8>,
    pub recovery_ms: Option<i64>,
    pub outcome: Option<Outcome>,
    pub before: Option<&'a str>,
    pub person: Option<&'a str>,
}

impl<'a> NewEvent<'a> {
    /// An event with only what every event has.
    pub fn new(app_id: &'a str, label: &'a str, kind: Kind, actor: Actor, message: &'a str) -> Self {
        NewEvent { app_id, label, kind, actor, message, reason: None, risk: None, recovery_ms: None, outcome: None, before: None, person: None }
    }
}

pub const HOUR_MS: i64 = 3_600_000;

/// An app's requests stopped at a limit (timed out or over budget) in one
/// hour.
#[derive(Debug, Clone)]
pub struct Hour {
    /// Unix ms / HOUR_MS.
    pub hour: i64,
    pub stops: u64,
}

#[derive(Debug, Clone)]
pub struct Usage {
    pub at: i64,
    pub app_id: String,
    pub label: String,
    pub route: String,
    pub outcome: Outcome,
    pub status: u16,
    pub wall_us: u64,
    pub queue_us: u64,
    pub fuel: u64,
    pub io: u32,
    pub runs: u32,
    pub error: Option<String>,
    /// The app's risk, for the events the usage writer writes.
    pub risk: Option<u8>,
}

/// A request as read back, for the admin screen. Requests repeat a few
/// versions and routes, so rows share them instead of copying.
#[derive(Debug, Clone)]
pub struct Request {
    pub at: i64,
    pub label: Arc<str>,
    pub route: Arc<str>,
    pub outcome: Outcome,
    pub wall_us: u64,
    pub fuel: u64,
    pub io: u32,
    pub error: Option<String>,
}

/// Requests in a stretch of time: how many, how many failed or stopped
/// at a limit, and the slowest that didn't fail (unknown from the hours).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cell {
    pub requests: u64,
    pub failures: u64,
    pub stops: u64,
    pub slowest_us: u64,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Limits as an event keeps them for Undo: `{"memory_mb":128}`.
fn limits_json(l: AdminLimits) -> String {
    serde_json::json!({ "memory_mb": l.memory_mb, "seconds": l.seconds, "fuel_m": l.fuel_m }).to_string()
}

/// The limits a limits-set event changed, from its `before`.
pub fn limits_before(e: &Event) -> Option<AdminLimits> {
    let v: serde_json::Value = serde_json::from_str(e.before.as_deref()?).ok()?;
    let get = |k: &str| v.get(k).and_then(serde_json::Value::as_u64).and_then(|n| u32::try_from(n).ok());
    Some(AdminLimits { memory_mb: get("memory_mb"), seconds: get("seconds"), fuel_m: get("fuel_m") })
}

/// Names as a JSON array, for `IN (SELECT value FROM json_each(?))`.
fn json_names<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    serde_json::to_string(&names.into_iter().collect::<Vec<_>>()).unwrap_or_default()
}

const VERSION_COLS: &str = "id, app_id, label, version, name, owner, app_group, status, reason, deployed_at, took_ms, grants, routes";

fn version(r: &rusqlite::Row) -> rusqlite::Result<Version> {
    Ok(Version {
        id: r.get(0)?,
        app_id: r.get(1)?,
        label: r.get(2)?,
        version: r.get(3)?,
        name: r.get(4)?,
        owner: r.get(5)?,
        group: r.get(6)?,
        status: r.get(7)?,
        reason: r.get(8)?,
        deployed_at: r.get(9)?,
        took_ms: r.get(10)?,
        grants: list(r, 11)?,
        routes: list(r, 12)?,
    })
}

/// A JSON array of strings in a column.
fn list(r: &rusqlite::Row, i: usize) -> rusqlite::Result<Vec<String>> {
    let text: String = r.get(i)?;
    serde_json::from_str(&text).map_err(|e| rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e)))
}

/// The columns an event is written with; it is read with `id` in front.
const EVENT_COLS: &str = "at, app_id, label, kind, severity, actor, message, reason, risk, recovery_ms, outcome, before, person";

fn event(r: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get(0)?,
        at: r.get(1)?,
        app_id: r.get(2)?,
        label: r.get(3)?,
        kind: r.get(4)?,
        severity: r.get(5)?,
        actor: r.get(6)?,
        message: r.get(7)?,
        reason: r.get(8)?,
        risk: r.get(9)?,
        recovery_ms: r.get(10)?,
        outcome: r.get(11)?,
        before: r.get(12)?,
        person: r.get(13)?,
    })
}

/// A deploy: the app and version, its owner and group, who sent it, the
/// app's risk, and what it was deployed with.
#[derive(Debug, Clone)]
pub struct NewVersion {
    pub app_id: String,
    pub label: String,
    pub version: String,
    pub name: String,
    /// Empty when refused for having none.
    pub owner: String,
    /// Empty when refused for having none.
    pub group: String,
    pub actor: Actor,
    /// Who sent it: the email of the token or admin, if known.
    pub person: Option<String>,
    pub risk: Option<u8>,
    pub grants: Vec<String>,
    /// Empty for a refused version: its routes aren't known.
    pub routes: Vec<String>,
    /// From deps.json: what it is built with, for `appos audit`. None
    /// without one, and for a refused version.
    pub packages: Option<Vec<String>>,
}

/// Who rolled back and why. The shell rolls back on its own after failures
/// and says how long they lasted.
pub struct Rollback<'a> {
    pub actor: Actor,
    pub person: Option<&'a str>,
    pub reason: Option<&'a str>,
    /// The risk of the version that goes live.
    pub risk: Option<u8>,
    pub recovery_ms: Option<i64>,
}

/// An app to turn off or on: what runs of it and its risk, for the event.
#[derive(Debug, Clone)]
pub struct Target {
    pub app_id: String,
    pub label: String,
    pub risk: Option<u8>,
}

impl PlatformDb {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        let found: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let empty: bool = conn.query_row("SELECT count(*) = 0 FROM sqlite_master", [], |r| r.get(0))?;
        if !empty && found != SCHEMA_VERSION {
            anyhow::bail!(
                "{} was made by another shell (schema {found}, this one needs {SCHEMA_VERSION}). The schema is unreleased and not migrated: delete the file and deploy the apps again",
                path.display()
            );
        }
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(PlatformDb(Arc::new(Mutex::new(conn))))
    }

    #[cfg(test)]
    pub fn in_memory() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        PlatformDb(Arc::new(Mutex::new(conn)))
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.0.lock()
    }

    /// Runs `f` on a blocking thread: SQLite and the connection's lock can
    /// wait, and the async workers go on serving requests meanwhile.
    pub async fn run<T: Send + 'static>(&self, f: impl FnOnce(&PlatformDb) -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
        let db = self.clone();
        tokio::task::spawn_blocking(move || f(&db)).await?
    }

    fn write_event(conn: &Connection, e: &NewEvent) -> rusqlite::Result<i64> {
        conn.execute(
            &format!("INSERT INTO events ({EVENT_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
            params![
                now_ms(),
                e.app_id,
                e.label,
                e.kind,
                Severity::of(e.kind, e.risk),
                e.actor,
                e.message,
                e.reason,
                e.risk,
                e.recovery_ms,
                e.outcome,
                e.before,
                e.person
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Keeps a new token (its hash) for `person` until `expires_at`.
    pub fn token_made(&self, hash: &str, person: &str, expires_at: i64) -> anyhow::Result<()> {
        self.conn().execute("INSERT INTO tokens (hash, person, made_at, expires_at) VALUES (?1, ?2, ?3, ?4)", params![hash, person, now_ms(), expires_at])?;
        Ok(())
    }

    /// Whose token this is, unless it is unknown or expired at `now`; notes
    /// that it was used.
    pub fn token(&self, hash: &str, now: i64) -> anyhow::Result<Option<String>> {
        let conn = self.conn();
        let person = conn.query_row("SELECT person FROM tokens WHERE hash = ?1 AND expires_at > ?2", params![hash, now], |r| r.get(0)).optional()?;
        if person.is_some() {
            conn.execute("UPDATE tokens SET used_at = ?2 WHERE hash = ?1", params![hash, now])?;
        }
        Ok(person)
    }

    /// Forgets a token (`appos logout`); false if it wasn't known.
    pub fn token_dropped(&self, hash: &str) -> anyhow::Result<bool> {
        Ok(self.conn().execute("DELETE FROM tokens WHERE hash = ?1", params![hash])? > 0)
    }

    /// Something that happened and changes no state, like a limit hit.
    /// Writes an event; returns its id.
    pub fn event(&self, e: &NewEvent) -> anyhow::Result<i64> {
        Ok(Self::write_event(&self.conn(), e)?)
    }

    fn insert_version(conn: &Connection, v: &NewVersion, status: VersionStatus, reason: Option<&str>, took: Duration) -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO versions (app_id, label, version, name, owner, app_group, status, reason, deployed_at, took_ms, grants, routes, packages)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                v.app_id,
                v.label,
                v.version,
                v.name,
                v.owner,
                v.group,
                status,
                reason,
                now_ms(),
                took.as_millis() as i64,
                serde_json::to_string(&v.grants)?,
                serde_json::to_string(&v.routes)?,
                v.packages.as_ref().map(serde_json::to_string).transpose()?
            ],
        )?;
        Ok(())
    }

    /// A deploy the shell refused after `took`.
    pub fn refused(&self, v: &NewVersion, reason: &str, took: Duration) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        Self::insert_version(&tx, v, VersionStatus::Refused, Some(reason), took)?;
        let e = NewEvent {
            reason: Some(reason),
            risk: v.risk,
            person: v.person.as_deref(),
            ..NewEvent::new(&v.app_id, &v.label, Kind::Refused, v.actor, "Deploy refused")
        };
        Self::write_event(&tx, &e)?;
        tx.commit()?;
        Ok(())
    }

    /// A deploy that went live after `took`: the live version before it is
    /// retired.
    pub fn deployed(&self, v: &NewVersion, took: Duration) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let before: Option<(String, String)> = tx
            .query_row("SELECT owner, app_group FROM versions WHERE app_id = ?1 AND status = ?2", params![v.app_id, VersionStatus::Live], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        tx.execute("UPDATE versions SET status = ?3 WHERE app_id = ?1 AND status = ?2", params![v.app_id, VersionStatus::Live, VersionStatus::Retired])?;
        Self::insert_version(&tx, v, VersionStatus::Live, None, took)?;
        let mut message = format!("Deployed, live in {} ms", took.as_millis());
        if let Some((owner, group)) = before {
            if owner != v.owner {
                message += &format!("; the owner is now {}", v.owner);
            }
            if group != v.group {
                message += &format!("; the group is now {}", v.group);
            }
        }
        let e = NewEvent { risk: v.risk, person: v.person.as_deref(), ..NewEvent::new(&v.app_id, &v.label, Kind::Deployed, v.actor, &message) };
        Self::write_event(&tx, &e)?;
        // An admin making a version live ends the pause.
        if v.actor == Actor::Admin {
            Self::set_switch(&tx, &v.app_id, Switch::Paused, None, false)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A deploy that loaded but waits, after `took`: the app is paused or
    /// deploys are frozen, and an admin makes it live.
    pub fn held(&self, v: &NewVersion, why: &str, took: Duration) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        Self::insert_version(&tx, v, VersionStatus::Waiting, Some(why), took)?;
        let message = "Deploy held until an admin makes it live";
        let e = NewEvent { reason: Some(why), risk: v.risk, person: v.person.as_deref(), ..NewEvent::new(&v.app_id, &v.label, Kind::Held, v.actor, message) };
        Self::write_event(&tx, &e)?;
        tx.commit()?;
        Ok(())
    }

    /// Switches on (`event` set) or off (none).
    fn set_switch(conn: &Connection, app_id: &str, switch: Switch, event: Option<i64>, stop_all: bool) -> rusqlite::Result<()> {
        match event {
            Some(id) => conn.execute(
                "INSERT INTO switches (app_id, switch, event_id, stop_all) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (app_id, switch) DO UPDATE SET event_id = ?3, stop_all = ?4",
                params![app_id, switch, id, stop_all],
            )?,
            None => conn.execute("DELETE FROM switches WHERE app_id = ?1 AND switch = ?2", params![app_id, switch])?,
        };
        Ok(())
    }

    /// Per app, the event that switched it, of the apps switched so.
    fn switched_all(&self, switch: Switch) -> anyhow::Result<BTreeMap<String, Event>> {
        let cols = EVENT_COLS.split(", ").map(|c| format!("e.{c}")).collect::<Vec<_>>().join(", ");
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT e.id, {cols} FROM switches s JOIN events e ON e.id = s.event_id WHERE s.switch = ?1"))?;
        let rows = stmt.query_map([switch], event)?;
        Ok(rows.map(|e| e.map(|e| (e.app_id.clone(), e))).collect::<Result<_, _>>()?)
    }

    /// The paused apps, each with the rollback that paused it.
    pub fn paused_all(&self) -> anyhow::Result<BTreeMap<String, Event>> {
        self.switched_all(Switch::Paused)
    }

    /// The apps that are off, each with the event that turned it off.
    pub fn off_all(&self) -> anyhow::Result<BTreeMap<String, Event>> {
        self.switched_all(Switch::Off)
    }

    /// The event that switched it, while it is switched.
    fn switched_by(&self, app_id: &str, switch: Switch) -> anyhow::Result<Option<Event>> {
        let cols = EVENT_COLS.split(", ").map(|c| format!("e.{c}")).collect::<Vec<_>>().join(", ");
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT e.id, {cols} FROM switches s JOIN events e ON e.id = s.event_id WHERE s.app_id = ?1 AND s.switch = ?2"),
                params![app_id, switch],
                event,
            )
            .optional()?)
    }

    /// The automatic rollback that paused the app, while no admin has made
    /// a version live since. Deploys wait while it lasts.
    pub fn paused(&self, app_id: &str) -> anyhow::Result<Option<Event>> {
        self.switched_by(app_id, Switch::Paused)
    }

    /// The event that turned the app off, while it is off.
    pub fn off(&self, app_id: &str) -> anyhow::Result<Option<Event>> {
        self.switched_by(app_id, Switch::Off)
    }

    /// The event that froze deploys, while they are frozen.
    pub fn frozen(&self) -> anyhow::Result<Option<Event>> {
        self.switched_by("", Switch::Frozen)
    }

    /// The apps that are off, for the shell to know at startup.
    pub fn off_apps(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT app_id FROM switches WHERE switch = ?1 ORDER BY app_id")?;
        Ok(stmt.query_map([Switch::Off], |r| r.get(0))?.collect::<Result<_, _>>()?)
    }

    /// The apps Stop all turned off that are still off.
    pub fn stopped_all(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT app_id FROM switches WHERE switch = ?1 AND stop_all ORDER BY app_id")?;
        Ok(stmt.query_map([Switch::Off], |r| r.get(0))?.collect::<Result<_, _>>()?)
    }

    /// Turns apps off or on, each with its event, in one transaction.
    /// `all`: by Stop all or Start all. Returns the events' ids.
    pub fn turned(&self, apps: &[Target], on: bool, actor: Actor, person: Option<&str>, reason: Option<&str>, all: bool) -> anyhow::Result<Vec<i64>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (kind, message) = match (on, all) {
            (false, false) => (Kind::TurnedOff, "Turned off"),
            (false, true) => (Kind::TurnedOff, "Turned off with Stop all"),
            (true, false) => (Kind::TurnedOn, "Turned on"),
            (true, true) => (Kind::TurnedOn, "Turned on with Start all"),
        };
        let mut ids = vec![];
        for t in apps {
            let e = NewEvent { reason, person, risk: t.risk, ..NewEvent::new(&t.app_id, &t.label, kind, actor, message) };
            let id = Self::write_event(&tx, &e)?;
            Self::set_switch(&tx, &t.app_id, Switch::Off, (!on).then_some(id), all)?;
            ids.push(id);
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Freezes or unfreezes deploys; returns the event's id.
    pub fn froze(&self, frozen: bool, actor: Actor, person: Option<&str>) -> anyhow::Result<i64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (kind, message) = if frozen { (Kind::Frozen, "Deploys frozen") } else { (Kind::Unfrozen, "Deploys unfrozen") };
        let id = Self::write_event(&tx, &NewEvent { person, ..NewEvent::new("", "", kind, actor, message) })?;
        Self::set_switch(&tx, "", Switch::Frozen, frozen.then_some(id), false)?;
        tx.commit()?;
        Ok(id)
    }

    /// What admins set for the apps' requests, by app.
    pub fn limits(&self) -> anyhow::Result<BTreeMap<String, AdminLimits>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT app_id, memory_mb, seconds, fuel_m FROM limits")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, AdminLimits { memory_mb: r.get(1)?, seconds: r.get(2)?, fuel_m: r.get(3)? })))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Keeps what an admin set for an app's requests (all defaults forget
    /// it), with its event saying `message` and keeping the limits
    /// `before`, for Undo. Returns the event's id.
    pub fn set_limits(&self, app_id: &str, label: &str, set: AdminLimits, before: AdminLimits, message: &str, person: Option<&str>) -> anyhow::Result<i64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if set == AdminLimits::default() {
            tx.execute("DELETE FROM limits WHERE app_id = ?1", [app_id])?;
        } else {
            tx.execute(
                "INSERT INTO limits (app_id, memory_mb, seconds, fuel_m) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (app_id) DO UPDATE SET memory_mb = ?2, seconds = ?3, fuel_m = ?4",
                params![app_id, set.memory_mb, set.seconds, set.fuel_m],
            )?;
        }
        let before = limits_json(before);
        let id = Self::write_event(&tx, &NewEvent { before: Some(&before), person, ..NewEvent::new(app_id, label, Kind::LimitsSet, Actor::Admin, message) })?;
        tx.commit()?;
        Ok(id)
    }

    /// Requests per app in `n` cells of `cell` ms from `from` (unix ms):
    /// counted from the kept requests for cells under an hour, else from
    /// the hours, which are kept for good. Apps without any have none.
    pub fn cells(&self, from: i64, cell: i64, n: usize) -> anyhow::Result<BTreeMap<String, Vec<Cell>>> {
        let conn = self.conn();
        let to = from + cell * n as i64;
        let mut out: BTreeMap<String, Vec<Cell>> = BTreeMap::new();
        let mut add = |app: String, k: i64, c: Cell| {
            if let Ok(k) = usize::try_from(k)
                && k < n
            {
                let cells = out.entry(app).or_insert_with(|| vec![Cell::default(); n]);
                let into = &mut cells[k];
                into.requests += c.requests;
                into.failures += c.failures;
                into.stops += c.stops;
                into.slowest_us = into.slowest_us.max(c.slowest_us);
            }
        };
        if cell < HOUR_MS {
            let failed = json_names(Outcome::ALL.into_iter().filter(|o| o.failed()).map(Outcome::as_str));
            let stopped = json_names(Outcome::ALL.into_iter().filter(|o| o.stopped()).map(Outcome::as_str));
            let mut stmt = conn.prepare_cached(
                "SELECT app_id, (at - ?1) / ?2, count(*), sum(outcome IN (SELECT value FROM json_each(?4))), sum(outcome IN (SELECT value FROM json_each(?5))),
                     max(CASE WHEN outcome IN (SELECT value FROM json_each(?4)) THEN 0 ELSE wall_us END)
                 FROM requests WHERE at >= ?1 AND at < ?3 GROUP BY 1, 2",
            )?;
            let mut rows = stmt.query(params![from, cell, to, failed, stopped])?;
            while let Some(r) = rows.next()? {
                let c = Cell {
                    requests: r.get::<_, i64>(2)? as u64,
                    failures: r.get::<_, i64>(3)? as u64,
                    stops: r.get::<_, i64>(4)? as u64,
                    slowest_us: r.get::<_, i64>(5)? as u64,
                };
                add(r.get(0)?, r.get(1)?, c);
            }
        } else {
            let mut stmt = conn.prepare_cached(
                "SELECT app_id, (hour * ?4 - ?1) / ?2, sum(requests), sum(failures), sum(stops)
                 FROM hours WHERE hour >= ?1 / ?4 AND hour < ?3 / ?4 GROUP BY 1, 2",
            )?;
            let mut rows = stmt.query(params![from, cell, to, HOUR_MS])?;
            while let Some(r) = rows.next()? {
                let c = Cell { requests: r.get::<_, i64>(2)? as u64, failures: r.get::<_, i64>(3)? as u64, stops: r.get::<_, i64>(4)? as u64, slowest_us: 0 };
                add(r.get(0)?, r.get(1)?, c);
            }
        }
        Ok(out)
    }

    /// Makes another version live: an earlier one again, or one that
    /// waited. The one that was live becomes `failed` when the shell rolled
    /// back on its own, which pauses the app; else `retired`, and by an
    /// admin it ends the pause. The reason goes on the one that was live
    /// and on the event, the label that was live as what it changed. A
    /// version that waited goes live as a deploy. Returns the event's id.
    pub fn rolled_back(&self, app_id: &str, to: &Version, from: Option<&Version>, why: &Rollback) -> anyhow::Result<i64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let auto = why.actor == Actor::Shell;
        let waited = to.status == VersionStatus::Waiting;
        let (status, kind) = match (auto, waited) {
            (true, _) => (VersionStatus::Failed, Kind::AutoRolledBack),
            (false, true) => (VersionStatus::Retired, Kind::Deployed),
            (false, false) => (VersionStatus::Retired, Kind::RolledBack),
        };
        // The reason is why the live version goes; a held version going live
        // says nothing against the one it replaces.
        let blame = if waited && !auto { None } else { why.reason };
        tx.execute("UPDATE versions SET status = ?2, reason = ?3 WHERE app_id = ?1 AND status = ?4", params![app_id, status, blame, VersionStatus::Live])?;
        tx.execute("UPDATE versions SET status = ?2, reason = NULL WHERE id = ?1", params![to.id, VersionStatus::Live])?;
        let from_label = from.map_or("none", |f| f.label.as_str());
        let message = match (auto, waited) {
            (true, _) => format!("Rolled back automatically from {from_label} to {}", to.label),
            (false, true) => format!("{} made live, replacing {from_label}", to.label),
            (false, false) => format!("{} made live again, replacing {from_label}", to.label),
        };
        let e = NewEvent {
            reason: why.reason,
            risk: why.risk,
            recovery_ms: why.recovery_ms,
            before: from.map(|f| f.label.as_str()),
            person: why.person,
            ..NewEvent::new(app_id, &to.label, kind, why.actor, &message)
        };
        let id = Self::write_event(&tx, &e)?;
        match why.actor {
            Actor::Shell => Self::set_switch(&tx, app_id, Switch::Paused, Some(id), false)?,
            Actor::Admin => Self::set_switch(&tx, app_id, Switch::Paused, None, false)?,
            Actor::Cli => {}
        }
        tx.commit()?;
        Ok(id)
    }

    /// Compares risk.toml's levels with those of the last start, writes a
    /// `risk-changed` event per grant that changed, and keeps the new ones.
    pub fn risk_levels(&self, levels: &BTreeMap<String, u8>) -> anyhow::Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let old: BTreeMap<String, u8> = {
            let mut stmt = tx.prepare("SELECT grant_name, level FROM risk_levels")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?
        };
        let mut changed = 0;
        for grant in old.keys().chain(levels.keys().filter(|g| !old.contains_key(*g))) {
            let (message, level) = match (old.get(grant), levels.get(grant)) {
                (Some(a), Some(b)) if a == b => continue,
                (Some(a), Some(b)) => (format!("{grant}: risk {a} → {b}"), Some(*b)),
                (None, Some(b)) => (format!("{grant}: rated {b}"), Some(*b)),
                (Some(a), None) => (format!("{grant}: no longer rated, was {a}"), None),
                (None, None) => continue,
            };
            Self::write_event(&tx, &NewEvent { risk: level, ..NewEvent::new("", "", Kind::RiskChanged, Actor::Shell, &message) })?;
            changed += 1;
        }
        tx.execute("DELETE FROM risk_levels", [])?;
        for (grant, level) in levels {
            tx.execute("INSERT INTO risk_levels (grant_name, level) VALUES (?1, ?2)", params![grant, level])?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// The live version of every app.
    pub fn live(&self) -> anyhow::Result<Vec<Version>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {VERSION_COLS} FROM versions WHERE status = ?1 ORDER BY app_id"))?;
        Ok(stmt.query_map([VersionStatus::Live], version)?.collect::<Result<_, _>>()?)
    }

    /// The live version of every app, with the packages it is built with
    /// (None: it has no deps.json).
    pub fn live_packages(&self) -> anyhow::Result<Vec<(Version, Option<Vec<String>>)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {VERSION_COLS}, packages FROM versions WHERE status = ?1 ORDER BY app_id"))?;
        let rows = stmt.query_map([VersionStatus::Live], |r| {
            let packages = match r.get::<_, Option<String>>(13)? {
                Some(_) => Some(list(r, 13)?),
                None => None,
            };
            Ok((version(r)?, packages))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn live_of(&self, app_id: &str) -> anyhow::Result<Option<Version>> {
        let conn = self.conn();
        let sql = format!("SELECT {VERSION_COLS} FROM versions WHERE app_id = ?1 AND status = ?2");
        Ok(conn.query_row(&sql, params![app_id, VersionStatus::Live], version).optional()?)
    }

    /// An app's versions, newest first.
    pub fn versions(&self, app_id: &str, limit: u32) -> anyhow::Result<Vec<Version>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {VERSION_COLS} FROM versions WHERE app_id = ?1 ORDER BY id DESC LIMIT ?2"))?;
        Ok(stmt.query_map(params![app_id, limit], version)?.collect::<Result<_, _>>()?)
    }

    /// The newest event, version and request, so a page can tell that
    /// something happened since it was drawn.
    pub fn newest(&self) -> anyhow::Result<Newest> {
        let conn = self.conn();
        Ok(conn.query_row(
            "SELECT (SELECT coalesce(max(id), 0) FROM events), (SELECT coalesce(max(id), 0) FROM versions), (SELECT coalesce(max(id), 0) FROM requests)",
            [],
            |r| Ok(Newest { events: r.get(0)?, versions: r.get(1)?, requests: r.get(2)? }),
        )?)
    }

    /// How many of each app's versions have this status; apps without
    /// any are left out.
    pub fn count_versions(&self, status: VersionStatus) -> anyhow::Result<BTreeMap<String, u32>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT app_id, count(*) FROM versions WHERE status = ?1 GROUP BY app_id")?;
        Ok(stmt.query_map([status], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?)
    }

    /// Every app's newest versions, at most `limit` each, newest first.
    pub fn versions_of_all(&self, limit: u32) -> anyhow::Result<BTreeMap<String, Vec<Version>>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {VERSION_COLS} FROM (SELECT *, row_number() OVER (PARTITION BY app_id ORDER BY id DESC) AS n FROM versions) WHERE n <= ?1 ORDER BY app_id, id DESC"
        ))?;
        let mut out: BTreeMap<String, Vec<Version>> = BTreeMap::new();
        for v in stmt.query_map([limit], version)? {
            let v = v?;
            out.entry(v.app_id.clone()).or_default().push(v);
        }
        Ok(out)
    }

    /// What a rollback goes back to: the given label, or else the newest
    /// retired version deployed before the live one. Refused versions were
    /// never stored; `failed` and `waiting` ones go live only by name.
    pub fn rollback_target(&self, app_id: &str, label: Option<&str>) -> anyhow::Result<Option<Version>> {
        use VersionStatus::{Failed, Live, Retired, Waiting};
        let conn = self.conn();
        let found = match label {
            Some(label) => conn.query_row(
                &format!("SELECT {VERSION_COLS} FROM versions WHERE app_id = ?1 AND label = ?2 AND status IN (?3, ?4, ?5) ORDER BY id DESC LIMIT 1"),
                params![app_id, label, Retired, Failed, Waiting],
                version,
            ),
            None => conn.query_row(
                &format!(
                    "SELECT {VERSION_COLS} FROM versions WHERE app_id = ?1 AND status = ?2
                     AND id < COALESCE((SELECT id FROM versions WHERE app_id = ?1 AND status = ?3), 1 << 62)
                     AND label != COALESCE((SELECT label FROM versions WHERE app_id = ?1 AND status = ?3), '')
                     ORDER BY id DESC LIMIT 1"
                ),
                params![app_id, Retired, Live],
                version,
            ),
        };
        Ok(found.optional()?)
    }

    /// The newest events, of one app or all.
    pub fn events(&self, app_id: Option<&str>, limit: u32) -> anyhow::Result<Vec<Event>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT id, {EVENT_COLS} FROM events WHERE ?1 IS NULL OR app_id = ?1 ORDER BY id DESC LIMIT ?2"))?;
        Ok(stmt.query_map(params![app_id, limit], event)?.collect::<Result<_, _>>()?)
    }

    /// One event by its id.
    pub fn event_by_id(&self, id: i64) -> anyhow::Result<Option<Event>> {
        let conn = self.conn();
        Ok(conn.query_row(&format!("SELECT id, {EVENT_COLS} FROM events WHERE id = ?1"), [id], event).optional()?)
    }

    /// Per app (and "" for the platform), its newest event of these kinds.
    pub fn newest_of(&self, kinds: &[Kind]) -> anyhow::Result<BTreeMap<String, Event>> {
        let kinds = json_names(kinds.iter().map(|k| k.as_str()));
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, {EVENT_COLS} FROM events WHERE id IN (SELECT max(id) FROM events WHERE kind IN (SELECT value FROM json_each(?1)) GROUP BY app_id)"
        ))?;
        let rows = stmt.query_map([kinds], event)?;
        Ok(rows.map(|e| e.map(|e| (e.app_id.clone(), e))).collect::<Result<_, _>>()?)
    }

    /// Events of these kinds since `since` (unix ms), of one app or all,
    /// oldest first.
    pub fn events_since(&self, app_id: Option<&str>, since: i64, kinds: &[Kind]) -> anyhow::Result<Vec<Event>> {
        let kinds = json_names(kinds.iter().map(|k| k.as_str()));
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, {EVENT_COLS} FROM events WHERE (?1 IS NULL OR app_id = ?1) AND at >= ?2 AND kind IN (SELECT value FROM json_each(?3)) ORDER BY id"
        ))?;
        Ok(stmt.query_map(params![app_id, since, kinds], event)?.collect::<Result<_, _>>()?)
    }

    /// When each app was first deployed (unix ms); refused deploys don't count.
    pub fn first_deploys(&self) -> anyhow::Result<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT app_id, MIN(deployed_at) FROM versions WHERE status != ?1 GROUP BY app_id ORDER BY app_id")?;
        Ok(stmt.query_map([VersionStatus::Refused], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?)
    }

    /// Limit stops per app and hour, from the hour of `since` (unix ms) on,
    /// oldest first.
    pub fn hours(&self, app_id: Option<&str>, since: i64) -> anyhow::Result<Vec<Hour>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT hour, stops FROM hours
             WHERE (?1 IS NULL OR app_id = ?1) AND hour >= ?2 ORDER BY hour, app_id",
        )?;
        let rows = stmt.query_map(params![app_id, since.div_euclid(HOUR_MS)], |r| Ok(Hour { hour: r.get(0)?, stops: r.get::<_, i64>(1)? as u64 }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Writes requests and adds them to their hours.
    pub fn record(&self, batch: &[Usage]) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO requests (at, app_id, label, route, outcome, status, wall_us, queue_us, fuel, io, runs, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )?;
            for u in batch {
                stmt.execute(params![
                    u.at,
                    u.app_id,
                    u.label,
                    u.route,
                    u.outcome,
                    u.status,
                    u.wall_us as i64,
                    u.queue_us as i64,
                    u.fuel as i64,
                    u.io,
                    u.runs,
                    u.error
                ])?;
            }
            let mut hour = tx.prepare_cached(
                "INSERT INTO hours (app_id, hour, requests, failures, stops) VALUES (?1, ?2, 1, ?3, ?4)
                 ON CONFLICT (app_id, hour) DO UPDATE SET
                     requests = requests + 1, failures = failures + excluded.failures, stops = stops + excluded.stops",
            )?;
            for u in batch {
                hour.execute(params![u.app_id, u.at.div_euclid(HOUR_MS), u.outcome.failed() as i64, u.outcome.stopped() as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Keeps the newest `keep` requests of each app. One app at a time, so
    /// requests are written in between.
    pub fn prune(&self, keep: u32) -> anyhow::Result<()> {
        let apps: Vec<String> = {
            let conn = self.conn();
            let mut stmt = conn.prepare("SELECT DISTINCT app_id FROM requests")?;
            stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
        };
        for app in apps {
            self.conn().execute(
                "DELETE FROM requests WHERE app_id = ?1 AND id <= (SELECT id FROM requests WHERE app_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
                params![app, keep],
            )?;
        }
        Ok(())
    }

    /// Every app's requests since `since` (unix ms), newest first.
    pub fn requests(&self, since: i64) -> anyhow::Result<BTreeMap<String, Vec<Request>>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!("SELECT app_id, {REQUEST_COLS} FROM requests WHERE at >= ?1 ORDER BY id DESC"))?;
        let mut rows = stmt.query(params![since])?;
        let (mut out, mut shared) = (BTreeMap::<String, Vec<Request>>::new(), Shared::default());
        while let Some(r) = rows.next()? {
            let request = request(r, 1, &mut shared)?;
            let app_id = text(r, 0)?;
            match out.get_mut(app_id) {
                Some(reqs) => reqs.push(request),
                None => drop(out.insert(app_id.to_string(), vec![request])),
            }
        }
        Ok(out)
    }

    /// An app's requests with one of these outcomes from `from` to before
    /// `to` (unix ms): `limit` of them by when they came in, the first or
    /// (`newest`) the newest first, and how many there are.
    pub fn requests_like(&self, app_id: &str, outcomes: &[Outcome], from: i64, to: i64, limit: u32, newest: bool) -> anyhow::Result<(Vec<Request>, u32)> {
        let outcomes = json_names(outcomes.iter().map(|o| o.as_str()));
        let conn = self.conn();
        let filter = "FROM requests WHERE app_id = ?1 AND outcome IN (SELECT value FROM json_each(?2)) AND at >= ?3 AND at < ?4";
        let count = conn.query_row(&format!("SELECT count(*) {filter}"), params![app_id, outcomes, from, to], |r| r.get(0))?;
        let order = if newest { "at DESC, id DESC" } else { "at, id" };
        let mut stmt = conn.prepare(&format!("SELECT {REQUEST_COLS} {filter} ORDER BY {order} LIMIT ?5"))?;
        let mut shared = Shared::default();
        let rows = stmt.query_map(params![app_id, outcomes, from, to, limit], |r| request(r, 0, &mut shared))?;
        Ok((rows.collect::<Result<_, _>>()?, count))
    }

    /// The log: events newest first, as `q` narrows them, at most `limit`.
    pub fn log(&self, q: &LogQuery, limit: u32) -> anyhow::Result<Vec<Event>> {
        let json = |v: Vec<&str>| if v.is_empty() { None } else { Some(json_names(v)) };
        let kinds = json(q.kinds.iter().map(|k| k.as_str()).collect());
        let severities = q.min_severity.and_then(|min| json(Severity::ALL.iter().filter(|s| **s >= min).map(|s| s.as_str()).collect()));
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, {EVENT_COLS} FROM events
             WHERE (?1 IS NULL OR app_id = ?1) AND (?2 IS NULL OR id < ?2) AND (?3 IS NULL OR at >= ?3) AND (?4 IS NULL OR at < ?4)
               AND (?5 IS NULL OR kind IN (SELECT value FROM json_each(?5)))
               AND (?6 IS NULL OR severity IN (SELECT value FROM json_each(?6)))
               AND (?7 IS NULL OR actor = ?7)
               AND (?8 IS NULL OR instr(lower(message || ' ' || coalesce(reason, '') || ' ' || label || ' ' || app_id || ' ' || coalesce(person, '')), lower(?8)) > 0)
             ORDER BY id DESC LIMIT ?9"
        ))?;
        let rows = stmt.query_map(params![q.app_id, q.before, q.from, q.until, kinds, severities, q.actor, q.text, limit], event)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// The ids of the newest rows: they only grow.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Newest {
    pub events: i64,
    pub versions: i64,
    pub requests: i64,
}

impl Newest {
    /// As in a URL: "12.3.4567".
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.split('.').map(|p| p.parse::<i64>().ok());
        let (events, versions, requests) = (parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(Newest { events, versions, requests })
    }
}

impl std::fmt::Display for Newest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.events, self.versions, self.requests)
    }
}

/// Which events the log shows; every part is optional.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct LogQuery {
    /// Of this app; "" is the platform itself.
    pub app_id: Option<String>,
    /// Older than this event: the next page.
    pub before: Option<i64>,
    /// From and before these times (unix ms).
    pub from: Option<i64>,
    pub until: Option<i64>,
    /// Any of these; all when empty.
    pub kinds: Vec<Kind>,
    pub min_severity: Option<Severity>,
    pub actor: Option<Actor>,
    /// In the message, the reason, the version or the app's id, in any case.
    pub text: Option<String>,
}

const REQUEST_COLS: &str = "at, label, route, outcome, wall_us, fuel, io, error";

/// The strings rows repeat, each kept once.
#[derive(Default)]
struct Shared(std::collections::HashSet<Arc<str>>);

impl Shared {
    fn get(&mut self, s: &str) -> Arc<str> {
        if let Some(shared) = self.0.get(s) {
            return shared.clone();
        }
        let shared: Arc<str> = s.into();
        self.0.insert(shared.clone());
        shared
    }
}

/// A text column, borrowed from the row.
fn text<'r>(r: &'r rusqlite::Row, i: usize) -> rusqlite::Result<&'r str> {
    r.get_ref(i)?.as_str().map_err(|e| rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e)))
}

/// A request from its columns, `REQUEST_COLS` from `i` on.
fn request(r: &rusqlite::Row, i: usize, shared: &mut Shared) -> rusqlite::Result<Request> {
    Ok(Request {
        at: r.get(i)?,
        label: shared.get(text(r, i + 1)?),
        route: shared.get(text(r, i + 2)?),
        outcome: r.get(i + 3)?,
        wall_us: r.get::<_, i64>(i + 4)? as u64,
        fuel: r.get::<_, i64>(i + 5)? as u64,
        io: r.get(i + 6)?,
        error: r.get(i + 7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(label: &str) -> NewVersion {
        NewVersion {
            app_id: "a".into(),
            label: label.into(),
            version: "0.1.0".into(),
            name: "A".into(),
            owner: "Jane Doe <jane@example.com>".into(),
            group: "Marketing".into(),
            actor: Actor::Cli,
            person: None,
            risk: Some(8),
            grants: vec![],
            routes: vec![],
            packages: None,
        }
    }

    fn usage(app: &str, at: i64, outcome: Outcome) -> Usage {
        Usage {
            at,
            app_id: app.into(),
            label: "l".into(),
            route: "GET /".into(),
            outcome,
            status: 200,
            wall_us: 1,
            queue_us: 0,
            fuel: 1,
            io: 0,
            runs: 1,
            error: None,
            risk: None,
        }
    }

    fn target(app_id: &str) -> Target {
        Target { app_id: app_id.into(), label: "one".into(), risk: None }
    }

    #[test]
    fn limits_are_kept_until_set_back_to_the_defaults() {
        let db = PlatformDb::in_memory();
        let set = AdminLimits { memory_mb: Some(128), seconds: None, fuel_m: Some(2000) };
        db.set_limits("a", "one", set, AdminLimits::default(), "Limits per request: memory 64 → 128 MB", None).unwrap();
        db.set_limits("a", "one", AdminLimits { seconds: Some(5), ..set }, set, "Limits per request: time 2 → 5 s", None).unwrap();
        assert_eq!(db.limits().unwrap().get("a"), Some(&AdminLimits { seconds: Some(5), ..set }));
        let e = &db.events(Some("a"), 1).unwrap()[0];
        assert_eq!((e.kind, e.actor, e.message.as_str()), (Kind::LimitsSet, Actor::Admin, "Limits per request: time 2 → 5 s"));
        assert_eq!(limits_before(e), Some(set), "the limits it changed, for Undo");
        db.set_limits("a", "one", AdminLimits::default(), set, "back", None).unwrap();
        assert!(db.limits().unwrap().is_empty());
    }

    /// A new file gets the schema; a file from an older shell is refused,
    /// not half used.
    #[test]
    fn an_older_platform_db_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("platform.db");
        PlatformDb::open(&path).unwrap();
        PlatformDb::open(&path).unwrap();
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 0).unwrap();
        let err = PlatformDb::open(&path).err().unwrap().to_string();
        assert!(err.contains("schema 0, this one needs") && err.contains("delete the file"), "{err}");
    }

    #[test]
    fn rollback_steps_back_through_deploys() {
        let db = PlatformDb::in_memory();
        db.deployed(&v("one"), Duration::ZERO).unwrap();
        db.deployed(&v("two"), Duration::ZERO).unwrap();
        db.refused(&v("bad"), "no", Duration::ZERO).unwrap();
        db.deployed(&v("three"), Duration::ZERO).unwrap();
        assert_eq!(db.live_of("a").unwrap().unwrap().label, "three");
        assert_eq!(db.count_versions(VersionStatus::Refused).unwrap(), BTreeMap::from([("a".to_string(), 1)]));
        let all = db.versions_of_all(2).unwrap();
        assert_eq!(all["a"].iter().map(|v| v.label.as_str()).collect::<Vec<_>>(), ["three", "bad"]);
        let newest = db.newest().unwrap();
        assert_eq!((newest.versions, newest.requests), (4, 0));
        assert_eq!(Newest::parse(&newest.to_string()), Some(newest));
        assert_eq!(Newest::parse("1.2"), None);
        assert_eq!(Newest::parse("1.2.3.4"), None);

        let to = db.rollback_target("a", None).unwrap().unwrap();
        assert_eq!(to.label, "two");
        let from = db.live_of("a").unwrap();
        let why = Rollback { actor: Actor::Shell, person: None, reason: Some("5 failed requests in 38 s"), risk: Some(8), recovery_ms: Some(41_000) };
        let id = db.rolled_back("a", &to, from.as_ref(), &why).unwrap();
        assert_eq!(db.live_of("a").unwrap().unwrap().label, "two");
        let e = &db.events(Some("a"), 1).unwrap()[0];
        assert_eq!((e.id, e.before.as_deref()), (id, Some("three")), "what it changed, for Undo");
        assert_eq!(db.event_by_id(id).unwrap().unwrap().id, id);
        assert_eq!(db.paused_all().unwrap()["a"].id, id);
        assert_eq!((e.kind, e.severity, e.actor), (Kind::AutoRolledBack, Severity::Critical, Actor::Shell));
        assert_eq!((e.reason.as_deref(), e.recovery_ms), (Some("5 failed requests in 38 s"), Some(41_000)));
        assert_eq!(e.message, "Rolled back automatically from three to two");

        // "three" failed: not a target of an automatic rollback, but it can
        // be made live by name.
        assert_eq!(db.rollback_target("a", None).unwrap().unwrap().label, "one");
        assert_eq!(db.rollback_target("a", Some("three")).unwrap().unwrap().status, VersionStatus::Failed);
        assert!(db.rollback_target("a", Some("bad")).unwrap().is_none());
    }

    /// After an automatic rollback deploys wait; a held version goes live
    /// only by an admin, which ends the pause. The CLI's rollbacks don't.
    #[test]
    fn an_automatic_rollback_pauses_until_an_admin_makes_a_version_live() {
        let db = PlatformDb::in_memory();
        db.deployed(&v("one"), Duration::ZERO).unwrap();
        db.deployed(&v("two"), Duration::ZERO).unwrap();
        assert!(db.paused("a").unwrap().is_none());
        let one = db.rollback_target("a", None).unwrap().unwrap();
        let auto = Rollback { actor: Actor::Shell, person: None, reason: Some("5 failed requests in 38 s"), risk: None, recovery_ms: Some(1) };
        db.rolled_back("a", &one, db.live_of("a").unwrap().as_ref(), &auto).unwrap();
        assert_eq!(db.paused("a").unwrap().unwrap().kind, Kind::AutoRolledBack);

        db.held(&v("three"), "held", Duration::ZERO).unwrap();
        assert_eq!(db.live_of("a").unwrap().unwrap().label, "one");
        assert_eq!(db.events(Some("a"), 1).unwrap()[0].kind, Kind::Held);
        // Waiting versions go live only by name.
        assert_ne!(db.rollback_target("a", None).unwrap().map(|t| t.status), Some(VersionStatus::Waiting));
        let three = db.rollback_target("a", Some("three")).unwrap().unwrap();
        assert_eq!(three.status, VersionStatus::Waiting);

        let cli = Rollback { actor: Actor::Cli, person: None, reason: None, risk: None, recovery_ms: None };
        db.rolled_back("a", &db.rollback_target("a", Some("two")).unwrap().unwrap(), db.live_of("a").unwrap().as_ref(), &cli).unwrap();
        assert!(db.paused("a").unwrap().is_some(), "the CLI ended the pause");

        let admin = Rollback { actor: Actor::Admin, person: None, reason: Some("fixed"), risk: None, recovery_ms: None };
        db.rolled_back("a", &three, db.live_of("a").unwrap().as_ref(), &admin).unwrap();
        assert!(db.paused("a").unwrap().is_none());
        let e = &db.events(Some("a"), 1).unwrap()[0];
        assert_eq!((e.kind, e.actor, e.message.as_str(), e.reason.as_deref()), (Kind::Deployed, Actor::Admin, "three made live, replacing two", Some("fixed")));
        let two = db.versions("a", 10).unwrap().into_iter().find(|v| v.label == "two").unwrap();
        assert_eq!((two.status, two.reason), (VersionStatus::Retired, None));
    }

    /// A deploy by another owner or group says so; each version keeps its own.
    #[test]
    fn a_new_owner_or_group_shows_in_the_deploy_event() {
        let db = PlatformDb::in_memory();
        db.deployed(&v("one"), Duration::ZERO).unwrap();
        db.deployed(&v("two"), Duration::ZERO).unwrap();
        assert_eq!(db.events(Some("a"), 1).unwrap()[0].message, "Deployed, live in 0 ms");
        db.deployed(&NewVersion { owner: "Growth team <growth@example.com>".into(), ..v("three") }, Duration::ZERO).unwrap();
        assert_eq!(db.events(Some("a"), 1).unwrap()[0].message, "Deployed, live in 0 ms; the owner is now Growth team <growth@example.com>");
        let owners: Vec<String> = db.versions("a", 10).unwrap().into_iter().map(|v| v.owner).collect();
        assert_eq!(owners, ["Growth team <growth@example.com>", "Jane Doe <jane@example.com>", "Jane Doe <jane@example.com>"]);
        db.deployed(&NewVersion { owner: "Growth team <growth@example.com>".into(), group: "Sales".into(), ..v("four") }, Duration::ZERO).unwrap();
        assert_eq!(db.events(Some("a"), 1).unwrap()[0].message, "Deployed, live in 0 ms; the group is now Sales");
        assert_eq!(db.versions("a", 1).unwrap()[0].group, "Sales");
    }

    /// Off from turned-off until turned-on, per app.
    #[test]
    fn an_app_is_off_until_turned_on() {
        let db = PlatformDb::in_memory();
        assert!(db.off("a").unwrap().is_none());
        db.turned(&[Target { risk: Some(8), ..target("a") }], false, Actor::Admin, None, Some("leaks"), false).unwrap();
        db.turned(&[target("b")], false, Actor::Cli, None, None, false).unwrap();
        let e = db.off("a").unwrap().unwrap();
        assert_eq!((e.kind, e.actor, e.reason.as_deref(), e.severity), (Kind::TurnedOff, Actor::Admin, Some("leaks"), Severity::High));
        assert_eq!(db.off_apps().unwrap(), ["a", "b"]);
        assert_eq!(db.off_all().unwrap().keys().collect::<Vec<_>>(), ["a", "b"]);
        let on = db.turned(&[target("a")], true, Actor::Admin, None, None, false).unwrap();
        assert!(db.off("a").unwrap().is_none());
        let newest = db.newest_of(&[Kind::TurnedOff, Kind::TurnedOn]).unwrap();
        assert_eq!((newest["a"].id, newest["a"].kind, newest["b"].kind), (on[0], Kind::TurnedOn, Kind::TurnedOff));
        assert_eq!(db.off_apps().unwrap(), ["b"]);
        db.turned(&[target("a")], false, Actor::Cli, None, None, false).unwrap();
        assert_eq!(db.off_apps().unwrap(), ["a", "b"]);
    }

    #[test]
    fn prune_keeps_the_newest_per_app() {
        let db = PlatformDb::in_memory();
        let u = |app, at| usage(app, at, Outcome::Ok);
        let batch: Vec<Usage> = (0..10).map(|i| u("a", i)).chain((0..3).map(|i| u("b", i))).collect();
        db.record(&batch).unwrap();
        db.prune(4).unwrap();
        let by_app = db.requests(0).unwrap();
        assert_eq!(by_app["a"].iter().map(|r| r.at).collect::<Vec<_>>(), vec![9, 8, 7, 6]);
        assert_eq!(by_app["b"].len(), 3);
        assert_eq!(db.requests(7).unwrap().into_keys().collect::<Vec<_>>(), ["a"]);
        db.prune(0).unwrap();
        assert!(db.requests(0).unwrap().is_empty());
    }

    /// One step up at high or critical risk; unrated counts as not risky.
    #[test]
    fn severity_follows_kind_and_risk() {
        assert_eq!(Severity::of(Kind::Deployed, None), Severity::Info);
        assert_eq!(Severity::of(Kind::Deployed, Some(6)), Severity::Info);
        assert_eq!(Severity::of(Kind::Deployed, Some(7)), Severity::Low);
        assert_eq!(Severity::of(Kind::Refused, Some(10)), Severity::High);
        assert_eq!(Severity::of(Kind::AutoRolledBack, Some(3)), Severity::High);
        assert_eq!(Severity::of(Kind::AutoRolledBack, Some(9)), Severity::Critical);
        assert!(Severity::ALL.windows(2).all(|w| w[0] < w[1]));
    }

    /// Every outcome the shell writes reads back; the limit hits are what
    /// counts toward a rollback, and stopped is a kind of failed.
    #[test]
    fn outcomes_read_back_and_agree() {
        let db = PlatformDb::in_memory();
        db.record(&Outcome::ALL.map(|o| usage("a", 0, o))).unwrap();
        let read: Vec<Outcome> = db.requests(0).unwrap()["a"].iter().rev().map(|r| r.outcome).collect();
        assert_eq!(read, Outcome::ALL);
        for o in Outcome::ALL {
            assert!(!o.stopped() || o.failed(), "{o}");
            assert_eq!(o.counts_toward_rollback(), matches!(o, Outcome::Timeout | Outcome::OverBudget | Outcome::Crashed), "{o}");
        }
        assert!(!Outcome::Denied.failed(), "a denial is the policy at work, not the app failing");
        assert_eq!(Outcome::of_status(303), Outcome::Ok);
        assert_eq!(Outcome::of_status(422), Outcome::ClientError);
        assert_eq!(Outcome::of_status(500), Outcome::ServerError);
    }

    /// Cells shorter than an hour count the kept requests; an hour or
    /// longer, the hourly counts, which are kept for good.
    #[test]
    fn cells_count_requests_per_stretch() {
        let db = PlatformDb::in_memory();
        let from = 100 * HOUR_MS;
        let usage = |at: i64, outcome: Outcome, wall_us: u64| Usage { at: from + at, label: "one".into(), wall_us, fuel: 0, ..usage("a", 0, outcome) };
        db.record(&[
            usage(10_000, Outcome::Ok, 300_000),
            usage(70_000, Outcome::ServerError, 9_000_000),
            usage(75_000, Outcome::Timeout, 2_000_000),
            usage(80_000, Outcome::ClientError, 100_000),
        ])
        .unwrap();
        db.record(&[usage(-1, Outcome::Ok, 1), usage(3 * 60_000, Outcome::Ok, 1)]).unwrap();
        let cells = &db.cells(from, 60_000, 3).unwrap()["a"];
        assert_eq!(cells[0], Cell { requests: 1, failures: 0, stops: 0, slowest_us: 300_000 });
        assert_eq!(cells[1], Cell { requests: 3, failures: 2, stops: 1, slowest_us: 100_000 });
        assert_eq!(cells[2], Cell::default());
        let hours = &db.cells(from - HOUR_MS, HOUR_MS, 2).unwrap()["a"];
        assert_eq!((hours[0].requests, hours[1].requests, hours[1].failures, hours[1].stops), (1, 5, 2, 1));
        assert!(db.cells(from + 24 * HOUR_MS, 60_000, 3).unwrap().is_empty());
    }

    /// A freeze and Stop all are switches: set with their event, cleared
    /// by the opposite one.
    #[test]
    fn a_freeze_and_stop_all_are_switches() {
        let db = PlatformDb::in_memory();
        db.deployed(&v("one"), Duration::ZERO).unwrap();
        assert!(db.frozen().unwrap().is_none());
        db.froze(true, Actor::Admin, None).unwrap();
        assert_eq!(db.frozen().unwrap().unwrap().kind, Kind::Frozen);
        db.froze(false, Actor::Admin, None).unwrap();
        assert!(db.frozen().unwrap().is_none());
        db.turned(&[target("a"), target("b")], false, Actor::Admin, None, None, true).unwrap();
        assert_eq!(db.stopped_all().unwrap(), ["a", "b"]);
        assert_eq!(db.off("a").unwrap().unwrap().message, "Turned off with Stop all");
        db.turned(&[target("a")], true, Actor::Admin, None, None, false).unwrap();
        assert_eq!(db.stopped_all().unwrap(), ["b"]);
        // Turned off by hand after Stop all: not Stop all's to turn on.
        db.turned(&[target("b")], false, Actor::Admin, None, None, false).unwrap();
        assert!(db.stopped_all().unwrap().is_empty());
        assert_eq!(db.off_apps().unwrap(), ["b"]);
    }

    /// Only grants whose level changed since the last start get an event.
    #[test]
    fn changed_risk_levels_are_events() {
        let db = PlatformDb::in_memory();
        let levels = |l: &[(&str, u8)]| l.iter().map(|(g, n)| (g.to_string(), *n)).collect::<BTreeMap<_, _>>();
        assert_eq!(db.risk_levels(&levels(&[("kv", 2), ("net", 7)])).unwrap(), 2);
        assert_eq!(db.risk_levels(&levels(&[("kv", 2), ("net", 7)])).unwrap(), 0);
        assert_eq!(db.risk_levels(&levels(&[("kv", 4), ("mail", 5)])).unwrap(), 3);
        let mut said: Vec<String> = db.events(None, 3).unwrap().into_iter().map(|e| e.message).collect();
        said.sort();
        assert_eq!(said, ["kv: risk 2 → 4", "mail: rated 5", "net: no longer rated, was 7"]);
    }

    /// Requests add up per app and hour, and stay when requests are pruned.
    #[test]
    fn requests_add_up_per_hour() {
        use Outcome::*;
        let db = PlatformDb::in_memory();
        let batch = [usage("a", 0, Ok), usage("a", 1, Timeout), usage("a", 2, ClientError), usage("a", HOUR_MS, Crashed), usage("b", 3, OverBudget)];
        db.record(&batch).unwrap();
        db.record(&[usage("a", 4, Ok)]).unwrap();
        db.prune(0).unwrap();
        let hours: Vec<(String, i64, i64, i64, i64)> = {
            let conn = db.conn();
            let mut stmt = conn.prepare("SELECT app_id, hour, requests, failures, stops FROM hours ORDER BY hour, app_id").unwrap();
            stmt.query_map([], |r| std::result::Result::Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap().map(Result::unwrap).collect()
        };
        let row = |a: &str, h, r, f, s| (a.to_string(), h, r, f, s);
        assert_eq!(hours, [row("a", 0, 4, 1, 1), row("b", 0, 1, 1, 1), row("a", 1, 1, 1, 0)]);
        let stops: Vec<_> = db.hours(Some("a"), HOUR_MS + 5).unwrap().into_iter().map(|h| (h.hour, h.stops)).collect();
        assert_eq!(stops, [(1, 0)]);
    }

    /// The log narrows by app, page, time, kind, severity, actor and text.
    #[test]
    fn the_log_narrows() {
        let db = PlatformDb::in_memory();
        let ev = |app_id: &str, kind: Kind, actor: Actor, message: &str, reason: Option<&str>, risk: Option<u8>| {
            db.event(&NewEvent { reason, risk, ..NewEvent::new(app_id, "0.1.0-ab", kind, actor, message) }).unwrap();
        };
        ev("a", Kind::Deployed, Actor::Cli, "Deployed, live in 3 ms", None, Some(1));
        ev("a", Kind::LimitHit, Actor::Shell, "Timed out on GET /", Some("wall-clock limit"), Some(1));
        ev("b", Kind::Refused, Actor::Cli, "Refused", Some("uses kv::set"), Some(9));
        ev("", Kind::RiskChanged, Actor::Shell, "kv-write: rated 2", None, Some(2));
        let ids = |q: LogQuery| db.log(&q, 10).unwrap().iter().map(|e| e.message.clone()).collect::<Vec<_>>();
        assert_eq!(ids(LogQuery::default()), ["kv-write: rated 2", "Refused", "Timed out on GET /", "Deployed, live in 3 ms"]);
        assert_eq!(db.log(&LogQuery::default(), 2).unwrap().len(), 2);
        assert_eq!(ids(LogQuery { app_id: Some("a".into()), ..Default::default() }), ["Timed out on GET /", "Deployed, live in 3 ms"]);
        assert_eq!(ids(LogQuery { app_id: Some("".into()), ..Default::default() }), ["kv-write: rated 2"]);
        let newest = db.log(&LogQuery::default(), 1).unwrap()[0].id;
        assert_eq!(ids(LogQuery { before: Some(newest - 1), ..Default::default() }), ["Timed out on GET /", "Deployed, live in 3 ms"]);
        assert_eq!(ids(LogQuery { kinds: vec![Kind::Refused, Kind::Deployed], ..Default::default() }), ["Refused", "Deployed, live in 3 ms"]);
        // Refused at risk 9 is one step up: high.
        assert_eq!(ids(LogQuery { min_severity: Some(Severity::High), ..Default::default() }), ["Refused"]);
        assert_eq!(ids(LogQuery { actor: Some(Actor::Shell), ..Default::default() }), ["kv-write: rated 2", "Timed out on GET /"]);
        assert_eq!(ids(LogQuery { text: Some("KV::SET".into()), ..Default::default() }), ["Refused"]);
        assert_eq!(ids(LogQuery { from: Some(now_ms() + 1000), ..Default::default() }), Vec::<String>::new());
        assert_eq!(ids(LogQuery { until: Some(now_ms() + 1000), app_id: Some("b".into()), ..Default::default() }), ["Refused"]);
    }

    #[test]
    fn events_since_filters_kind_and_time() {
        let db = PlatformDb::in_memory();
        db.deployed(&v("one"), Duration::ZERO).unwrap();
        db.refused(&v("bad"), "no", Duration::ZERO).unwrap();
        let kinds = |k: &[Kind]| db.events_since(None, 0, k).unwrap().into_iter().map(|e| e.kind).collect::<Vec<_>>();
        assert_eq!(kinds(&[Kind::Refused, Kind::Deployed]), [Kind::Deployed, Kind::Refused]);
        assert_eq!(kinds(&[Kind::AutoRolledBack]), []);
        assert!(db.events_since(None, now_ms() + 1000, &Kind::ALL).unwrap().is_empty());
        assert_eq!(db.events_since(Some("a"), 0, &Kind::ALL).unwrap().len(), 2);
        assert!(db.events_since(Some("b"), 0, &Kind::ALL).unwrap().is_empty());
        assert_eq!(db.first_deploys().unwrap().len(), 1);
    }
}
