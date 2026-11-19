//! Who may call what: `appos-shell --policy policy.csv`, read by casbin.
//!
//! An app's grants (app.toml) say what the app may do; the policy says
//! which people may make it do that. A host call goes through only if both
//! allow it, under the same names: `kv-read`, `kv-write`.
//!
//! ```text
//! # policy.csv
//! p, anyone, webinars, kv-*
//! p, group:sales, discounts, kv-*
//! p, group:marketing, apps:Marketing, kv-read
//! p, group:platform, *, admin-*
//! p, group:support, *, admin-read
//! p, group:marketing, apps:Marketing, admin-deploy
//! g, bob@acme.com, group:platform
//! ```
//!
//! `p, who, where, what`. Who: an email, `group:<name>` (from the identity
//! provider, or given with `g`), `signed-in` or `anyone`. Where: an app id,
//! `apps:<group>` (the group in app.toml) or `*`. What: a grant, or a
//! pattern like `kv-*`. AdminKit's grants are the admin screen and the
//! platform API: admin-read to look, admin-write to change, with `*` as
//! where. admin-deploy (deploy and roll back) and admin-off (turn off and
//! on) give that much of admin-write, through the platform API, for the
//! apps their where names: marketing deploys its apps and can't touch the
//! CRM team's, or Freeze.
//! `g, who, group:<name>` puts someone in a group. What no line allows is
//! denied; with `--no-auth` there is no policy and everything is allowed.
//!
//! The grants are the kits' (kit.rs). The policy is worked out once per
//! request, before the app runs, as the grants this person may use in this
//! app (`Access`); a kit call tests a bit. The file is checked every 2 s and read again when it changes; a
//! file with a mistake leaves the policy as it was.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use casbin::function_map::key_match;
use casbin::{CoreApi, DefaultModel, Enforcer, MemoryAdapter, MgmtApi};
use parking_lot::{Mutex, RwLock};

use crate::auth::{ANYONE, SIGNED_IN, Who};
use crate::kit::{ADMIN_DEPLOY, ADMIN_OFF, ADMIN_READ, ADMIN_WRITE, User};
use crate::manifest::{MAX_GRANTS, validate_app_id};
use crate::platform::{Actor, Kind, NewEvent, now_ms};
use crate::runtime::LoadedApp;
use crate::shell::Shell;
use crate::{terminal_errorln as eprintln, terminal_logln as println};

/// Built in, not configurable: the policy file has the rules only.
const MODEL: &str = r#"
[request_definition]
r = sub, app, group, act
[policy_definition]
p = sub, app, act
[role_definition]
g = _, _
[policy_effect]
e = some(where (p.eft == allow))
[matchers]
m = g(r.sub, p.sub) && (p.app == "*" || p.app == r.app || p.app == r.group) && keyMatch(r.act, p.act)
"#;

/// AdminKit's grants start so: the admin screen's.
pub const ADMIN_PREFIX: &str = "admin-";
/// How often the file is checked for changes.
const POLL: Duration = Duration::from_secs(2);
/// Requests worked out and kept, per policy. A cache this full starts over.
const CACHED: usize = 10_000;

/// The grants a request may use: a bit per grant the app holds, by its
/// place in app.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Access(u64);

const _: () = assert!(MAX_GRANTS <= 64);

impl Access {
    pub const ALL: Access = Access(u64::MAX);
    pub const NONE: Access = Access(0);

    /// Whether the app's grant number `bit` may be used.
    pub fn allows(self, bit: u32) -> bool {
        bit < 64 && self.0 & (1 << bit) != 0
    }

    fn with(self, bit: usize) -> Access {
        Access(self.0 | 1 << bit)
    }

    #[cfg(test)]
    pub fn without(self, bit: u32) -> Access {
        Access(self.0 & !(1 << bit))
    }
}

/// What a request may use in `app`: everything without a policy.
pub fn of(shell: &Shell, who: &Who, app: &LoadedApp) -> Access {
    let Some(policies) = &shell.policy else { return Access::ALL };
    let m = &app.manifest;
    policies.current().access(who, &m.app.id, m.app.group.as_deref().unwrap_or("").trim(), &m.capabilities.grant)
}

/// Who is signed in, for kits and apps (kit.rs): the identity, with the
/// groups the sign-in gave and those policy.csv puts them in.
pub fn user(shell: &Shell, who: &Who) -> Option<User> {
    let id = who.identity()?;
    let groups = match &shell.policy {
        Some(policies) => policies.current().groups(who),
        None => id.groups.clone(),
    };
    Some(User { id: id.id.clone(), email: id.email.clone(), name: id.name.clone(), groups })
}

/// A request's subjects, the app, its group and the grants it holds.
type Asked = (Vec<String>, String, String, Vec<String>);

/// policy.csv as casbin holds it.
pub struct Policy {
    enforcer: Enforcer,
    rules: usize,
    /// The `p` lines (who, app, what) and the `g` lines (who, group), for
    /// the admin screen.
    lines: (Vec<Vec<String>>, Vec<Vec<String>>),
    cache: Mutex<HashMap<Asked, Access>>,
}

impl Policy {
    /// Checks every line against the kits' `grants`, then hands them to
    /// casbin. A mistake anywhere refuses the whole file.
    pub async fn parse(text: &str, grants: &[String]) -> anyhow::Result<Policy> {
        Policy::parse_with(text, grants, false).await
    }

    async fn parse_with(text: &str, grants: &[String], built_in: bool) -> anyhow::Result<Policy> {
        let (mut p, mut g) = (Vec::<Vec<String>>::new(), Vec::<Vec<String>>::new());
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split(',').map(str::trim).collect();
            let rule = match fields[..] {
                ["p", who, app, what] => rule(who, app, what, grants, built_in).map(|r| (&mut p, r)),
                ["g", who, group] => member(who, group).map(|r| (&mut g, r)),
                _ => Err("give `p, who, app, what` or `g, who, group:<name>`".to_string()),
            };
            match rule {
                Ok((rules, r)) if !rules.contains(&r) => rules.push(r),
                Ok(_) => {}
                Err(e) => anyhow::bail!("line {}: {line}: {e}", n + 1),
            }
        }
        let rules = p.len() + g.len();
        let lines = (p.clone(), g.clone());
        let mut enforcer = Enforcer::new(DefaultModel::from_str(MODEL).await?, MemoryAdapter::default()).await?;
        if !p.is_empty() {
            enforcer.add_policies(p).await?;
        }
        if !g.is_empty() {
            enforcer.add_grouping_policies(g).await?;
        }
        Ok(Policy { enforcer, rules, lines, cache: Mutex::default() })
    }

    /// The rules and memberships in the file.
    pub fn rules(&self) -> usize {
        self.rules
    }

    /// The rules (who, app, what) and memberships (who, group).
    pub fn lines(&self) -> (&[Vec<String>], &[Vec<String>]) {
        (&self.lines.0, &self.lines.1)
    }

    /// The grants of `held` (the app's, in app.toml's order) that someone
    /// may use in this app.
    pub fn access(&self, who: &Who, app_id: &str, group: &str, held: &[String]) -> Access {
        let key = (who.subjects(), app_id.to_string(), format!("apps:{group}"), held.to_vec());
        if let Some(access) = self.cache.lock().get(&key) {
            return *access;
        }
        let (subjects, app, group, _) = &key;
        let allowed = |g: &String| subjects.iter().any(|s| self.allows(s, app, group, g));
        let access = held.iter().enumerate().take(MAX_GRANTS).filter(|(_, g)| allowed(g)).fold(Access::NONE, |a, (bit, _)| a.with(bit));
        let mut cache = self.cache.lock();
        if cache.len() >= CACHED {
            cache.clear();
        }
        cache.insert(key, access);
        access
    }

    /// The groups someone is in: the sign-in's, and those the `g` lines
    /// put them in, also through other groups. Sorted, without `group:`.
    pub fn groups(&self, who: &Who) -> Vec<String> {
        let mut subjects: BTreeSet<String> = who.subjects().into_iter().collect();
        loop {
            let before = subjects.len();
            for m in &self.lines.1 {
                if subjects.contains(&m[0]) {
                    subjects.insert(m[1].clone());
                }
            }
            if subjects.len() == before {
                break;
            }
        }
        subjects.iter().filter_map(|s| s.strip_prefix("group:")).map(str::to_string).collect()
    }

    /// Whether someone may use a grant of the platform's, one that `*`
    /// gives: admin-read, admin-write.
    pub fn may(&self, who: &Who, act: &str) -> bool {
        who.subjects().iter().any(|s| self.allows(s, "", "", act))
    }

    /// Whether a line gives someone `act` for some app, by their email or
    /// one of their groups.
    pub fn may_somewhere(&self, who: &Who, act: &str) -> bool {
        let mut subjects: BTreeSet<String> = who.subjects().into_iter().collect();
        subjects.extend(self.groups(who).into_iter().map(|g| format!("group:{g}")));
        self.lines.0.iter().any(|l| subjects.contains(&l[0]) && key_match(act, &l[2]))
    }

    /// Whether someone may use admin-deploy or admin-off on the app
    /// `app_id` of `group`: by a line for the app, its group or `*`.
    pub fn may_in(&self, who: &Who, act: &str, app_id: &str, group: &str) -> bool {
        let group = format!("apps:{}", group.trim());
        who.subjects().iter().any(|s| self.allows(s, app_id, &group, act))
    }

    /// Fails closed: casbin failing is a no.
    fn allows(&self, sub: &str, app: &str, group: &str, act: &str) -> bool {
        self.enforcer.enforce((sub, app, group, act)).unwrap_or_else(|e| {
            eprintln!("policy: {e}");
            false
        })
    }
}

/// The admin- grants a policy line may give for some apps only.
pub const PER_APP: [&str; 2] = [ADMIN_DEPLOY, ADMIN_OFF];

/// A change the policy doesn't let this person make: the platform API
/// answers 403.
#[derive(Debug)]
pub struct Forbidden(pub String);

impl std::fmt::Display for Forbidden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Forbidden {}

/// Whether `who` may use `grant` (admin-deploy, admin-off) on the app
/// `id` through the platform API: admin-write gives it for every app,
/// `grant` for the app or the group it has here. A new app has no group
/// yet. Without a policy everyone may.
pub async fn may_change(shell: &Shell, who: &Who, grant: &str, id: &str) -> anyhow::Result<()> {
    let Some(policies) = &shell.policy else { return Ok(()) };
    let p = policies.current();
    if p.may(who, ADMIN_WRITE) {
        return Ok(());
    }
    let group = group_here(shell, id).await?;
    if p.may_in(who, grant, id, group.as_deref().unwrap_or_default()) {
        return Ok(());
    }
    Err(Forbidden(format!(
        "{} may not {} {id}: it needs admin-write, or {grant} for {id} or its group, which policy.csv doesn't give them",
        person(who),
        doing(grant)
    ))
    .into())
}

/// Whether `who` may look at the app `id`'s versions and news: with
/// admin-read, or admin-deploy or admin-off for it.
pub async fn may_look(shell: &Shell, who: &Who, id: &str) -> anyhow::Result<()> {
    let Some(policies) = &shell.policy else { return Ok(()) };
    let p = policies.current();
    if p.may(who, ADMIN_READ) || p.may(who, ADMIN_WRITE) {
        return Ok(());
    }
    let group = group_here(shell, id).await?;
    if PER_APP.iter().any(|g| p.may_in(who, g, id, group.as_deref().unwrap_or_default())) {
        return Ok(());
    }
    Err(Forbidden(format!("{} may not look at {id}: it needs admin-read, or admin-deploy or admin-off for {id} or its group", person(who))).into())
}

/// Whether `who` may deploy what app.toml (or kit.toml) `m` says: an app
/// with admin-deploy for its id and the group in `m`, and also the group
/// the app has here, so no one moves another team's app into their own
/// group by deploying it. A kit needs admin-write.
pub async fn may_deploy(shell: &Shell, who: &Who, m: &crate::manifest::Manifest) -> anyhow::Result<()> {
    let Some(policies) = &shell.policy else { return Ok(()) };
    let p = policies.current();
    if p.may(who, ADMIN_WRITE) {
        return Ok(());
    }
    let id = &m.app.id;
    if m.is_kit() {
        return Err(Forbidden(format!("{} may not deploy the kit {id}: kits need admin-write, which policy.csv doesn't give them", person(who))).into());
    }
    let to = m.app.group.as_deref().unwrap_or_default();
    let here = group_here(shell, id).await?;
    let groups = std::iter::once(to).chain(here.as_deref());
    if let Some(group) = groups.into_iter().find(|g| !p.may_in(who, ADMIN_DEPLOY, id, g)) {
        let which = if group.trim().is_empty() { String::new() } else { format!(" or apps:{}", group.trim()) };
        return Err(Forbidden(format!(
            "{} may not deploy {id} to group {group:?}: it needs admin-write, or admin-deploy for {id}{which}, which policy.csv doesn't give them",
            person(who)
        ))
        .into());
    }
    Ok(())
}

/// The group the app `id` has here: its live version's, else its newest
/// stored one's. None for an app never deployed.
async fn group_here(shell: &Shell, id: &str) -> anyhow::Result<Option<String>> {
    let id = id.to_string();
    shell.platform.run(move |db| db.group_of(&id)).await
}

fn person(who: &Who) -> String {
    who.person().unwrap_or_else(|| "someone not signed in".into())
}

fn doing(grant: &str) -> &'static str {
    if grant == ADMIN_OFF { "turn off or on" } else { "deploy or roll back" }
}

/// `p, who, app, what`. `what` is a grant or a pattern; a grant no kit
/// offers now is fine unless it looks like a misspelt one of `grants`, the
/// platform's own (manifest.rs `misspelt`). The admin- grants go with `*`
/// as the app, and by name: a pattern like `*` gives them only in a
/// built-in policy.
fn rule(who: &str, app: &str, what: &str, grants: &[String], built_in: bool) -> Result<Vec<String>, String> {
    let who = subject(who)?;
    match app {
        "*" => {}
        _ if app.starts_with("apps:") => {
            if app["apps:".len()..].trim().is_empty() {
                return Err("apps: needs the group from app.toml, like apps:Marketing".into());
            }
        }
        _ => validate_app_id(app).map_err(|e| format!("{app:?} is no app: {e}; give an app id, apps:<group> or *"))?,
    }
    let admin: Vec<&str> = grants_of(what, grants).into_iter().filter(|g| g.starts_with(ADMIN_PREFIX)).collect();
    if what.starts_with(ADMIN_PREFIX) {
        if app != "*" && (admin.is_empty() || admin.iter().any(|g| !PER_APP.contains(g))) {
            return Err(format!(
                "{what} is the admin screen's, for the whole platform: give * as the app; admin-deploy and admin-off can name an app or apps:<group>"
            ));
        }
    } else if !admin.is_empty() && !built_in {
        let given = admin.join(" and ");
        return Err(format!("{what:?} would also give {given}: name the grants (kv-*), and admin-* on a line of its own"));
    }
    if grants_of(what, grants).is_empty()
        && let Some(why) = crate::manifest::misspelt(what, grants).filter(|_| !built_in)
    {
        return Err(why);
    }
    Ok(vec![who, app.to_string(), what.to_string()])
}

/// The grants of `grants` a rule's what names: the one, or those its
/// pattern matches.
pub fn grants_of<'a>(what: &str, grants: &'a [String]) -> Vec<&'a str> {
    grants.iter().map(String::as_str).filter(|g| key_match(g, what)).collect()
}

/// `g, who, group:<name>`.
fn member(who: &str, group: &str) -> Result<Vec<String>, String> {
    let who = subject(who)?;
    if !who.contains('@') && !who.starts_with("group:") {
        return Err(format!("{who:?}: only people (an email) and groups join groups"));
    }
    if !group.starts_with("group:") || group.len() == "group:".len() {
        return Err(format!("{group:?}: give the group as group:<name>"));
    }
    Ok(vec![who, group.to_string()])
}

/// An email (in lower case, as sign-in keeps them), a group, `signed-in`
/// or `anyone`.
fn subject(who: &str) -> Result<String, String> {
    match who {
        ANYONE | SIGNED_IN => Ok(who.to_string()),
        "group:" => Err("group: needs a name".into()),
        _ if who.starts_with("group:") => Ok(who.to_string()),
        _ if who.contains('@') => Ok(who.to_lowercase()),
        _ => Err(format!("{who:?} is no one: give an email, group:<name>, signed-in or anyone")),
    }
}

/// The policy in force, and the file it comes from; none for the token
/// sign-in's, which is built in (auth/token.rs).
pub struct Policies {
    path: Option<PathBuf>,
    /// The kits' grants, which the file may name.
    grants: Vec<String>,
    current: RwLock<Arc<Policy>>,
    /// The file's mtime and length when last read; none while it's gone.
    seen: Mutex<Option<(SystemTime, u64)>>,
    /// When the policy in force was read (unix ms).
    since: Mutex<i64>,
    /// The last change not taken, when and why; none once one is.
    refused: Mutex<Option<(i64, String)>>,
}

impl Policies {
    /// Reads the file, which may name the kits' `grants`; a mistake stops
    /// the shell from starting.
    pub async fn load(path: &Path, grants: Vec<String>) -> anyhow::Result<Policies> {
        let seen = stamp(path);
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let policy = Policy::parse(&text, &grants).await.map_err(|e| anyhow::anyhow!("{}: {e:#}", path.display()))?;
        Ok(Policies {
            path: Some(path.to_path_buf()),
            grants,
            current: RwLock::new(Arc::new(policy)),
            seen: Mutex::new(seen),
            since: Mutex::new(now_ms()),
            refused: Mutex::default(),
        })
    }

    pub fn current(&self) -> Arc<Policy> {
        self.current.read().clone()
    }

    /// A policy no file holds: read once, never again. It may give `*`.
    pub async fn built_in(text: &str, grants: Vec<String>) -> anyhow::Result<Policies> {
        let policy = Policy::parse_with(text, &grants, true).await?;
        Ok(Policies {
            path: None,
            grants,
            current: RwLock::new(Arc::new(policy)),
            seen: Mutex::default(),
            since: Mutex::new(now_ms()),
            refused: Mutex::default(),
        })
    }

    pub fn file_name(&self) -> String {
        match &self.path {
            Some(path) => path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
            None => "the built-in policy".into(),
        }
    }

    /// The file; none for a built-in policy.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// When the policy in force was read (unix ms).
    pub fn since(&self) -> i64 {
        *self.since.lock()
    }

    /// The last change not taken, when (unix ms) and why, until one is.
    pub fn refused(&self) -> Option<(i64, String)> {
        self.refused.lock().clone()
    }

    /// Reads the file again if it changed: `None` if it didn't, the number
    /// of rules if the new one is in force, an error if the old one stays.
    pub async fn reload(&self) -> anyhow::Result<Option<usize>> {
        let Some(path) = &self.path else { return Ok(None) };
        let now = stamp(path);
        if std::mem::replace(&mut *self.seen.lock(), now) == now {
            return Ok(None);
        }
        let read = match now {
            Some(_) => match std::fs::read_to_string(path) {
                Ok(text) => Policy::parse(&text, &self.grants).await,
                Err(e) => Err(e.into()),
            },
            None => Err(anyhow::anyhow!("the file is gone")),
        };
        match read {
            Ok(policy) => {
                let rules = policy.rules();
                *self.current.write() = Arc::new(policy);
                *self.since.lock() = now_ms();
                *self.refused.lock() = None;
                Ok(Some(rules))
            }
            Err(e) => {
                *self.refused.lock() = Some((now_ms(), format!("{e:#}")));
                Err(e)
            }
        }
    }
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Checks the file every 2 s and logs each change as an event.
pub fn watch(shell: Arc<Shell>) {
    if shell.policy.as_ref().and_then(|p| p.path()).is_none() {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            check(&shell).await;
        }
    });
}

/// One check of the file.
pub async fn check(shell: &Shell) {
    let Some(policies) = &shell.policy else { return };
    let file = policies.file_name();
    let event = match policies.reload().await {
        Ok(None) => return,
        Ok(Some(rules)) => {
            let message = format!("{file}: {rules} rules");
            println!("{message}, in force");
            (Kind::PolicyChanged, message, None)
        }
        Err(e) => {
            let reason = format!("{e:#}");
            eprintln!("{file}: {reason}; the policy stays as it was");
            (Kind::PolicyRefused, format!("{file} not taken, the policy stays as it was"), Some(reason))
        }
    };
    let (kind, message, reason) = event;
    let e = NewEvent { reason: reason.as_deref(), ..NewEvent::new("", "", kind, Actor::Shell, &message) };
    if let Err(e) = shell.platform.event(&e) {
        eprintln!("could not log the policy change: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Identity;

    fn person(email: &str, groups: &[&str]) -> Who {
        Who::signed_in(Identity::new("", email, "", groups.iter().map(|g| g.to_string())).unwrap())
    }

    fn kv() -> Vec<String> {
        crate::kit::Kits::builtin().names()
    }

    fn may(p: &Policy, who: &Who, app: &str, group: &str) -> (bool, bool) {
        let a = p.access(who, app, group, &kv());
        (a.allows(0), a.allows(1))
    }

    #[tokio::test]
    async fn people_groups_signed_in_and_anyone() {
        let p = Policy::parse(
            "# who may do what\n\
             p, anyone, webinars, kv-read\n\
             p, signed-in, webinars, kv-write\n\
             p, group:sales, discounts, kv-*\n\
             p, Ann@Acme.com, pricing, kv-read\n\
             g, bob@acme.com, group:sales\n",
            &kv(),
        )
        .await
        .unwrap();
        let nobody = Who::default();
        assert_eq!(may(&p, &nobody, "webinars", "Marketing"), (true, false));
        assert_eq!(may(&p, &person("eve@acme.com", &[]), "webinars", "Marketing"), (true, true));
        assert_eq!(may(&p, &person("eve@acme.com", &["sales"]), "discounts", "Sales"), (true, true));
        assert_eq!(may(&p, &person("bob@acme.com", &[]), "discounts", "Sales"), (true, true), "g puts bob in sales");
        assert_eq!(may(&p, &person("eve@acme.com", &[]), "discounts", "Sales"), (false, false));
        assert_eq!(may(&p, &person("ann@acme.com", &[]), "pricing", "Sales"), (true, false), "emails in any case");
        assert_eq!(may(&p, &nobody, "pricing", "Sales"), (false, false));
        assert_eq!(p.rules(), 5);
    }

    #[tokio::test]
    async fn a_users_groups_come_from_the_sign_in_and_the_policy() {
        let p = Policy::parse("g, bob@acme.com, group:sales\ng, group:sales, group:revenue\ng, group:ops, group:platform\n", &kv()).await.unwrap();
        assert_eq!(p.groups(&person("Bob@Acme.com", &["ops"])), ["ops", "platform", "revenue", "sales"]);
        assert_eq!(p.groups(&person("eve@acme.com", &[])), Vec::<String>::new());
        assert_eq!(p.groups(&Who::default()), Vec::<String>::new());
    }

    #[tokio::test]
    async fn apps_by_id_group_or_all() {
        let p = Policy::parse("p, group:marketing, apps:Marketing, kv-read\np, group:ops, *, kv-*\n", &kv()).await.unwrap();
        let marketing = person("m@acme.com", &["marketing"]);
        assert_eq!(may(&p, &marketing, "webinars", "Marketing"), (true, false));
        assert_eq!(may(&p, &marketing, "discounts", "Sales"), (false, false));
        assert_eq!(may(&p, &person("o@acme.com", &["ops"]), "discounts", "Sales"), (true, true));
        // Only the grants the app holds.
        assert_eq!(p.access(&marketing, "webinars", "Marketing", &["kv-write".into()]), Access::NONE);
    }

    #[tokio::test]
    async fn the_admin_grants_go_by_name_for_the_platform() {
        use crate::kit::{ADMIN_READ, ADMIN_WRITE};
        let csv = "p, group:platform, *, admin-*\np, group:support, *, admin-read\np, group:ops, *, kv-*\ng, bob@acme.com, group:platform\n";
        let p = Policy::parse(csv, &kv()).await.unwrap();
        let (bob, support, ops) = (person("bob@acme.com", &[]), person("s@acme.com", &["support"]), person("o@acme.com", &["ops"]));
        assert!(p.may(&bob, ADMIN_READ) && p.may(&bob, ADMIN_WRITE));
        assert!(p.may(&support, ADMIN_READ) && !p.may(&support, ADMIN_WRITE), "looks, doesn't change");
        assert!(!p.may(&ops, ADMIN_READ), "every kv grant is no admin grant");
        assert!(!p.may(&Who::default(), ADMIN_READ));
        assert_eq!(may(&p, &bob, "webinars", "Marketing"), (false, false), "admin-* gives no app's grant");
    }

    #[tokio::test]
    async fn admin_deploy_and_admin_off_go_by_app_or_group() {
        use crate::kit::{ADMIN_DEPLOY, ADMIN_OFF, ADMIN_WRITE};
        let csv = "p, group:marketing, apps:Marketing, admin-deploy\np, sam@acme.com, discounts, admin-off\np, group:platform, *, admin-*\n";
        let p = Policy::parse(csv, &kv()).await.unwrap();
        let (mo, sam, bob) = (person("mo@acme.com", &["marketing"]), person("sam@acme.com", &[]), person("bob@acme.com", &["platform"]));
        assert!(p.may_in(&mo, ADMIN_DEPLOY, "webinars", "Marketing") && !p.may_in(&mo, ADMIN_DEPLOY, "discounts", "Sales"));
        assert!(!p.may_in(&mo, ADMIN_OFF, "webinars", "Marketing"), "deploying is no turning off");
        assert!(!p.may_in(&mo, ADMIN_DEPLOY, "webinars", ""), "an app without a group is in none");
        assert!(p.may_in(&sam, ADMIN_OFF, "discounts", "Sales") && !p.may_in(&sam, ADMIN_OFF, "webinars", "Sales"));
        assert!(!p.may(&mo, ADMIN_DEPLOY) && !p.may(&mo, ADMIN_WRITE), "no platform-wide grant");
        assert!(p.may_in(&bob, ADMIN_DEPLOY, "webinars", "Marketing"), "admin-* gives it for every app");
        assert!(p.may_somewhere(&mo, ADMIN_DEPLOY) && !p.may_somewhere(&mo, ADMIN_OFF) && !p.may_somewhere(&Who::default(), ADMIN_DEPLOY));
        let err = |text: &'static str| async move { Policy::parse(text, &kv()).await.err().map(|e| e.to_string()).unwrap_or_default() };
        assert!(err("p, anyone, apps:Marketing, admin-*").await.contains("give * as the app; admin-deploy and admin-off can name an app"));
        assert!(err("p, anyone, webinars, admin-write").await.contains("give * as the app"));
    }

    #[tokio::test]
    async fn mistakes_refuse_the_file_with_the_line() {
        let err = |text: &'static str| async move { Policy::parse(text, &kv()).await.err().map(|e| e.to_string()).unwrap_or_default() };
        assert!(err("p, anyone, webinars, kv-read\np, anyone, webinars, kv-fly").await.contains("line 2"));
        assert!(err("p, anyone, webinars, kv-fly").await.contains("\"kv-fly\" is no grant; the kv- grants are kv-read, kv-write"));
        assert!(err("p, anyone, webinars, Kv_Read").await.contains("is no grant: lowercase letters, digits and dashes"));
        assert!(err("p, anyone, webinars, score-read\np, anyone, *, crm-*").await.is_empty(), "a Wasm kit's grants may come later");
        assert!(err("p, anyone, webinars, *").await.contains("would also give admin-read and admin-write"));
        assert!(err("p, anyone, *, a*").await.contains("admin-* on a line of its own"));
        assert!(err("p, anyone, webinars, admin-read").await.contains("give * as the app"));
        assert!(err("p, anyone, *, admin").await.contains("the admin- grants are admin-read, admin-write"));
        assert!(err("p, anyone, *, admin-rad").await.contains("the admin- grants are admin-read, admin-write"));
        assert!(err("p, bob, webinars, kv-read").await.contains("is no one"));
        assert!(err("p, anyone, Web Inars, kv-read").await.contains("is no app"));
        assert!(err("p, anyone, apps:, kv-read").await.contains("needs the group"));
        assert!(err("g, bob@acme.com, platform").await.contains("group:<name>"));
        assert!(err("g, anyone, group:x").await.contains("only people"));
        assert!(err("p, anyone, webinars").await.contains("give `p, who, app, what`"));
        assert!(err("r, anyone, webinars, kv-read").await.contains("give `p, who, app, what`"));
        // Twice the same line is fine.
        assert!(Policy::parse("p, anyone, webinars, kv-read\np, anyone, webinars, kv-read", &kv()).await.is_ok());
        assert!(Policy::parse("", &kv()).await.is_ok());
    }

    #[tokio::test]
    async fn a_changed_file_is_read_again_and_a_bad_one_leaves_the_old() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.csv");
        std::fs::write(&path, "p, anyone, webinars, kv-read\n").unwrap();
        let policies = Policies::load(&path, kv()).await.unwrap();
        assert_eq!(policies.reload().await.unwrap(), None, "unchanged");

        std::fs::write(&path, "p, anyone, webinars, kv-*\np, anyone, pricing, kv-read\n").unwrap();
        assert_eq!(policies.reload().await.unwrap(), Some(2));
        assert_eq!(may(&policies.current(), &Who::default(), "webinars", "M"), (true, true));

        std::fs::write(&path, "p, anyone, webinars, kv-fly\n").unwrap();
        assert!(policies.reload().await.unwrap_err().to_string().contains("kv-fly"));
        assert_eq!(policies.current().rules(), 2, "the old one stays");
        assert_eq!(policies.reload().await.unwrap(), None, "a bad file is reported once");

        std::fs::remove_file(&path).unwrap();
        assert!(policies.reload().await.unwrap_err().to_string().contains("gone"));
        assert_eq!(policies.current().rules(), 2);
    }

    #[tokio::test]
    async fn a_bad_file_stops_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.csv");
        assert!(Policies::load(&path, kv()).await.is_err());
        std::fs::write(&path, "p, anyone, webinars, kv-fly\n").unwrap();
        assert!(Policies::load(&path, kv()).await.err().unwrap().to_string().contains("policy.csv: line 1"));
    }
}
