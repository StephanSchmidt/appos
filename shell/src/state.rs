//! What runs, in one place: the version of each app, the apps that are off,
//! what admins set for their requests and the live versions' recent
//! failures. Every change goes through `State`: one at a time, it writes its
//! rows and its event in one platform.db transaction, then changes memory.
//! Loading an app (unzip, compile, init) happens before and outside, so the
//! kill switch, Stop all and a rollback never wait behind a deploy's
//! compile.
//!
//! Requests read only memory. Frozen deploys and paused apps matter only to
//! changes, so they are read from platform.db under the change lock.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, bail};
use parking_lot::{Mutex, RwLock};

use crate::deploy::{Ask, FAILURE_WINDOW, FAILURES_BEFORE_ROLLBACK};
use crate::manifest::Risk;
use crate::platform::{Actor, Event, NewVersion, PlatformDb, Rollback, Target, Version, VersionStatus, now_ms};
use crate::runtime::{AdminLimits, Budget, LoadedApp};

/// Why a deploy waits while deploys are frozen.
const FROZEN: &str = "Held: an admin froze deploys; an admin makes it live";

/// Why a deploy waits, from the rollback that paused the app.
fn held_reason(pause: &Event) -> String {
    match &pause.reason {
        Some(r) => format!("Held after the automatic rollback ({r}); an admin makes it live"),
        None => "Held after the automatic rollback; an admin makes it live".to_string(),
    }
}

/// Per app: the live version's label, and when it failed lately (for the
/// window, and in unix ms for the event).
type Failures = BTreeMap<String, (String, Vec<(Instant, i64)>)>;

pub struct State {
    platform: PlatformDb,
    risk: Risk,
    apps: RwLock<BTreeMap<String, Arc<LoadedApp>>>,
    off: RwLock<BTreeSet<String>>,
    limits: RwLock<BTreeMap<String, AdminLimits>>,
    failures: Mutex<Failures>,
    /// Changes happen one at a time, each only for its write and swap.
    change: tokio::sync::Mutex<()>,
}

impl State {
    /// The state platform.db keeps: which apps are off and their limits.
    /// The shell doesn't start without it.
    pub fn open(platform: PlatformDb, risk: Risk) -> anyhow::Result<State> {
        let off = platform.off_apps().context("could not read which apps are off")?;
        let limits = platform.limits().context("could not read the apps' limits")?;
        Ok(State {
            platform,
            risk,
            apps: Default::default(),
            off: RwLock::new(off.into_iter().collect()),
            limits: RwLock::new(limits),
            failures: Default::default(),
            change: Default::default(),
        })
    }

    pub fn app(&self, id: &str) -> Option<Arc<LoadedApp>> {
        self.apps.read().get(id).cloned()
    }

    /// Every running app, by id.
    pub fn apps(&self) -> BTreeMap<String, Arc<LoadedApp>> {
        self.apps.read().clone()
    }

    pub fn is_off(&self, id: &str) -> bool {
        self.off.read().contains(id)
    }

    /// What an admin set for the app's requests.
    pub fn limits_of(&self, id: &str) -> AdminLimits {
        self.limits.read().get(id).copied().unwrap_or_default()
    }

    /// What one request of `app` may use, with what an admin set.
    pub fn budget(&self, app: &LoadedApp) -> Budget {
        Budget::of(&app.manifest, self.limits_of(&app.manifest.app.id))
    }

    /// Apps that platform.db needn't hear of: from folders, at startup and
    /// on `appos dev`'s reload, and the live versions loaded at startup.
    pub fn insert_loaded(&self, apps: impl IntoIterator<Item = Arc<LoadedApp>>) {
        for app in apps {
            self.swap_in(app);
        }
    }

    fn swap_in(&self, app: Arc<LoadedApp>) {
        let id = app.manifest.app.id.clone();
        self.failures.lock().remove(&id);
        self.apps.write().insert(id, app);
    }

    /// A loaded deploy, `v`: goes live, or waits while the app is paused or
    /// deploys are frozen. Returns why it waits.
    pub async fn deployed(&self, app: Arc<LoadedApp>, v: NewVersion, started: Instant) -> anyhow::Result<Option<String>> {
        let _change = self.change.lock().await;
        let held = self
            .platform
            .run(move |db| {
                let why = match db.paused(&v.app_id)? {
                    Some(pause) => Some(held_reason(&pause)),
                    None => db.frozen()?.map(|_| FROZEN.to_string()),
                };
                match &why {
                    Some(why) => db.held(&v, why, started.elapsed())?,
                    None => db.deployed(&v, started.elapsed())?,
                }
                Ok(why)
            })
            .await?;
        if held.is_none() {
            self.swap_in(app);
        }
        Ok(held)
    }

    /// The live version and the version a rollback makes live, if `ask`
    /// may: see `deploy::rollback`.
    pub async fn rollback_target(&self, app_id: &str, label: Option<&str>, ask: &Ask) -> anyhow::Result<(Option<Version>, Version)> {
        let (id, label, ask) = (app_id.to_string(), label.map(str::to_string), ask.clone());
        self.platform.run(move |db| target(db, &id, label.as_deref(), &ask)).await
    }

    /// Makes `to`, loaded as `app`, live, if it still is what `ask` gets
    /// for `label`: something may have changed while it loaded. Returns
    /// the event's id.
    pub async fn roll_back(&self, app_id: &str, label: Option<&str>, ask: Ask, to: &Version, app: Arc<LoadedApp>) -> anyhow::Result<i64> {
        let _change = self.change.lock().await;
        let risk = self.risk.of_app(&app.manifest);
        let (id, label, expected) = (app_id.to_string(), label.map(str::to_string), to.id);
        let event = self
            .platform
            .run(move |db| {
                let (from, to) = target(db, &id, label.as_deref(), &ask)?;
                if to.id != expected {
                    bail!("{id} changed while {} loaded; try again", to.label);
                }
                let why = match &ask {
                    Ask::Cli { reason, person } => {
                        Rollback { actor: Actor::Cli, person: person.as_deref(), reason: reason.as_deref(), risk, recovery_ms: None }
                    }
                    Ask::Admin { person } => Rollback { actor: Actor::Admin, person: person.as_deref(), reason: None, risk, recovery_ms: None },
                    Ask::Failing { reason, since, .. } => {
                        Rollback { actor: Actor::Shell, person: None, reason: Some(reason), risk, recovery_ms: Some(now_ms() - since) }
                    }
                };
                db.rolled_back(&id, &to, from.as_ref(), &why)
            })
            .await?;
        self.swap_in(app);
        Ok(event)
    }

    /// What to turn off or on, if it runs.
    fn running(&self, app_id: &str, app: &LoadedApp) -> Target {
        Target { app_id: app_id.to_string(), label: app.label.clone(), risk: self.risk.of_app(&app.manifest) }
    }

    /// Turns an app off or on, by the CLI or an admin. An app an admin
    /// turned off goes back on only by an admin. Returns the event's id.
    pub async fn turn(&self, app_id: &str, on: bool, actor: Actor, person: Option<String>, reason: Option<String>) -> anyhow::Result<i64> {
        let _change = self.change.lock().await;
        let running = self.app(app_id).map(|a| self.running(app_id, &a));
        let (id, risk) = (app_id.to_string(), self.risk.clone());
        let event = self
            .platform
            .run(move |db| {
                let Some(target) = running.map_or_else(|| live_target(db, &risk, &id), |t| Ok(Some(t)))? else { bail!("no app {id} here") };
                match (db.off(&id)?, on) {
                    (None, true) => bail!("{id} is on"),
                    (Some(_), false) => bail!("{id} is already off"),
                    (Some(off), true) if off.actor == Actor::Admin && actor != Actor::Admin => {
                        let why = off.reason.map(|r| format!(" ({r})")).unwrap_or_default();
                        bail!("an admin turned {id} off{why}; it goes back on from the admin screen")
                    }
                    _ => {}
                }
                Ok(db.turned(&[target], on, actor, person.as_deref(), reason.as_deref(), false)?[0])
            })
            .await?;
        let mut off = self.off.write();
        if on {
            off.remove(app_id)
        } else {
            off.insert(app_id.to_string())
        };
        Ok(event)
    }

    /// Stop all: turns every running app that is on off, as an admin, and
    /// says which.
    pub async fn stop_all(&self, person: Option<String>) -> anyhow::Result<Vec<String>> {
        let _change = self.change.lock().await;
        let off = self.off.read().clone();
        let targets: Vec<Target> = self.apps().iter().filter(|(id, _)| !off.contains(*id)).map(|(id, a)| self.running(id, a)).collect();
        let ids: Vec<String> = targets.iter().map(|t| t.app_id.clone()).collect();
        self.platform.run(move |db| db.turned(&targets, false, Actor::Admin, person.as_deref(), None, true)).await?;
        self.off.write().extend(ids.iter().cloned());
        Ok(ids)
    }

    /// Start all: turns on the apps Stop all turned off, and only those.
    pub async fn start_all(&self, person: Option<String>) -> anyhow::Result<Vec<String>> {
        let _change = self.change.lock().await;
        let mut running: BTreeMap<String, Target> = self.apps().iter().map(|(id, a)| (id.clone(), self.running(id, a))).collect();
        let risk = self.risk.clone();
        let ids = self
            .platform
            .run(move |db| {
                let mut targets = vec![];
                for id in db.stopped_all()? {
                    targets.extend(running.remove(&id).map_or_else(|| live_target(db, &risk, &id), |t| Ok(Some(t)))?);
                }
                db.turned(&targets, true, Actor::Admin, person.as_deref(), None, true)?;
                Ok(targets.into_iter().map(|t| t.app_id).collect::<Vec<_>>())
            })
            .await?;
        let mut off = self.off.write();
        for id in &ids {
            off.remove(id);
        }
        Ok(ids)
    }

    /// Freezes or unfreezes deploys, as an admin; returns the event's id.
    pub async fn freeze(&self, frozen: bool, person: Option<String>) -> anyhow::Result<i64> {
        let _change = self.change.lock().await;
        self.platform
            .run(move |db| {
                match (db.frozen()?, frozen) {
                    (Some(_), true) => bail!("deploys are already frozen"),
                    (None, false) => bail!("deploys aren't frozen"),
                    _ => {}
                }
                db.froze(frozen, Actor::Admin, person.as_deref())
            })
            .await
    }

    /// Sets what an admin allows one app's requests: `change` turns the
    /// limits now into the new ones and the event's message, or says why
    /// not. Returns the event's id; the event keeps the limits before.
    pub async fn set_limits(
        &self,
        app_id: &str,
        label: &str,
        person: Option<String>,
        change: impl FnOnce(AdminLimits) -> anyhow::Result<(AdminLimits, String)>,
    ) -> anyhow::Result<i64> {
        let _change = self.change.lock().await;
        let old = self.limits_of(app_id);
        let (new, message) = change(old)?;
        let (id, label) = (app_id.to_string(), label.to_string());
        let event = self
            .platform
            .run(move |db| db.set_limits(&id, &label, new, old, &message, person.as_deref()))
            .await
            .with_context(|| format!("could not set the limits of {app_id}"))?;
        let mut limits = self.limits.write();
        if new == AdminLimits::default() {
            limits.remove(app_id);
        } else {
            limits.insert(app_id.to_string(), new);
        }
        Ok(event)
    }

    /// Counts a failed request of `label`. When the live version failed
    /// FAILURES_BEFORE_ROLLBACK times within FAILURE_WINDOW, says since when
    /// (unix ms), and counts anew.
    pub fn failed(&self, app_id: &str, label: &str) -> Option<i64> {
        let now = (Instant::now(), now_ms());
        let mut failures = self.failures.lock();
        let (l, recent) = failures.entry(app_id.to_string()).or_insert_with(|| (label.to_string(), vec![]));
        if l != label {
            *l = label.to_string();
            recent.clear();
        }
        recent.retain(|(t, _)| now.0.duration_since(*t) < FAILURE_WINDOW);
        recent.push(now);
        if recent.len() < FAILURES_BEFORE_ROLLBACK {
            return None;
        }
        let since = recent[0].1;
        failures.remove(app_id);
        Some(since)
    }
}

/// What to turn off or on of an app that doesn't run: its live version.
fn live_target(db: &PlatformDb, risk: &Risk, app_id: &str) -> anyhow::Result<Option<Target>> {
    Ok(db.live_of(app_id)?.map(|v| Target { app_id: app_id.to_string(), risk: risk.of_grants(&v.grants), label: v.label }))
}

/// The live version and what a rollback goes to: `label`, or else the one
/// deployed before the live one; refused when the shell's own rollback
/// comes too late, the CLI's while deploys are frozen, and a version that
/// waits for an admin unless by one.
fn target(db: &PlatformDb, app_id: &str, label: Option<&str>, ask: &Ask) -> anyhow::Result<(Option<Version>, Version)> {
    let from = db.live_of(app_id)?;
    if let Ask::Failing { label: failing, .. } = ask
        && from.as_ref().is_none_or(|f| f.label != *failing)
    {
        bail!("{failing} is no longer live");
    }
    if matches!(ask, Ask::Cli { .. }) && db.frozen()?.is_some() {
        bail!("an admin froze deploys; rollbacks happen on the admin screen until they are unfrozen");
    }
    let Some(to) = db.rollback_target(app_id, label)? else {
        match label {
            Some(l) if from.as_ref().is_some_and(|f| f.label == l) => bail!("{l} is already live"),
            Some(l) => bail!("{app_id} has no earlier version {l}; see appos versions"),
            None => bail!("{app_id} has no earlier version to go back to"),
        }
    };
    if to.status == VersionStatus::Waiting && !matches!(ask, Ask::Admin { .. }) {
        bail!("{} waits for an admin to make it live on the admin screen", to.label);
    }
    Ok((from, to))
}
