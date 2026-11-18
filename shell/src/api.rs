//! The platform API's JSON (`/_platform/...`), for the shell that answers
//! and for the appos CLI and agents that ask. Every response the shell
//! sends is one of these types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use crate::platform::{Actor, Kind, Outcome, Severity, Version, VersionStatus};

/// The limits apps run within, as the shell enforces them.
pub mod limits {
    pub use crate::bundle::{MAX as MAX_BUNDLE, MAX_ICON};
    pub use crate::deploy::{FAILURE_WINDOW, FAILURES_BEFORE_ROLLBACK};
    pub use crate::manifest::{MAX_SECONDS, SECONDS};
    pub use crate::runtime::{FUEL, IO_BUDGET, MAX_KEY, MAX_VALUE, MEMORY, QUEUE_WAIT};
    pub use crate::tx::{MAX_SCAN, MAX_SCAN_BYTES};
}

/// `POST /_platform/deploy`: the app as deployed, live (200) or held for
/// an admin (202).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deployed {
    pub id: String,
    pub name: String,
    pub version: String,
    pub label: String,
    /// Where the app is.
    pub url: String,
    /// Why it waits for an admin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<String>,
    /// Where an admin makes it live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin: Option<String>,
}

/// `GET /_platform/apps/{id}/news?since=`: what happened to an app since
/// the builder last looked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct News {
    /// The next `since`.
    pub now: i64,
    /// Oldest first, at most 10.
    pub events: Vec<NewsEvent>,
    /// How many older ones were left out.
    pub more: usize,
    /// Deploys wait for an admin after an automatic rollback.
    pub paused: Option<Paused>,
    /// The app is turned off.
    pub off: Option<Off>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsEvent {
    pub at: i64,
    pub label: String,
    pub kind: Kind,
    pub actor: Actor,
    pub message: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paused {
    pub at: i64,
    pub reason: Option<String>,
    /// Where an admin makes a version live.
    pub admin: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Off {
    pub at: i64,
    pub reason: Option<String>,
    pub actor: Actor,
    /// Where an admin turns it on.
    pub admin: String,
}

/// `GET /_platform/log`: events, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogPage {
    pub events: Vec<LogEvent>,
    /// The next page's `before`.
    pub older: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEvent {
    pub id: i64,
    pub at: i64,
    /// Empty for the platform itself.
    pub app: String,
    pub label: String,
    pub kind: Kind,
    pub severity: Severity,
    pub actor: Actor,
    pub message: String,
    pub reason: Option<String>,
    pub risk: Option<u8>,
    pub recovery_ms: Option<i64>,
    /// On the admin screen: the email of who was signed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<String>,
    /// Of a limit hit or an automatic rollback: the failed requests behind it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<Failed>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failed {
    pub count: u32,
    /// The first few, by when they came in.
    pub first: Vec<FailedRequest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailedRequest {
    pub at: i64,
    pub label: String,
    pub route: String,
    pub outcome: Outcome,
    pub wall_us: u64,
    pub error: Option<String>,
}

/// The body of `POST /_platform/apps/{id}/off` and `/on`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SwitchRequest {
    /// Why, in the person's words.
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /_platform/apps/{id}/off` and `/on`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Switched {
    pub id: String,
    pub on: bool,
    pub url: String,
}

/// The body of `POST /_platform/apps/{id}/rollback`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RollbackRequest {
    /// The version to make live; the one before the live one if not given.
    #[serde(default)]
    pub label: Option<String>,
    /// Why, in the person's words.
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /_platform/apps/{id}/rollback`: the version now live.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolledBack {
    pub id: String,
    pub label: String,
    pub url: String,
}

/// `GET /_platform/packages`: what each live app is built with, from the
/// deps.json in its app.zip, for `appos audit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LivePackages {
    pub id: String,
    pub name: String,
    /// From app.toml, like "0.3.0".
    pub version: String,
    pub label: String,
    pub owner: String,
    pub group: String,
    /// Package URLs like pkg:cargo/serde@1.0.228. None: the app has no
    /// deps.json (not a Rust app, or packed by an older appos).
    pub packages: Option<Vec<String>>,
}

/// `GET /_platform/spec`: what this platform offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub name: String,
    pub origin: String,
    /// The WIT package apps build against, like "appos:framework@0.1.0".
    pub framework: String,
    /// The grants an app can ask for.
    pub capabilities: Vec<String>,
    /// Each grant's risk, as the company rates it.
    pub risk: BTreeMap<String, u8>,
    pub apps: Vec<SpecApp>,
    /// Where app repositories live.
    pub git: Option<String>,
    /// The WIT files, by name.
    pub wit: BTreeMap<String, String>,
    /// The WIT of sign-in plugins (`provider = "plugin"` in auth.toml), by
    /// name: apart from `wit`, which apps build against.
    #[serde(default)]
    pub sign_in: BTreeMap<String, String>,
    /// The rest of the platform API takes a token (`appos login` asks for
    /// one at `/admin/token`); false with `--no-auth`.
    #[serde(default)]
    pub token: bool,
}

/// `GET /_platform/me`: whom the token signs in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Me {
    pub email: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecApp {
    pub id: String,
    pub name: String,
    pub version: String,
}
