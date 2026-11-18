//! The admin screen's changes: an app off and on, a version made live,
//! Seen, Freeze, Stop all, Start all, an app's limits, and Undo. Like a
//! control on a device, each acts at once, no page asks first. It is logged
//! as admin and goes back to the console with a toast that says what
//! happened and offers Undo for ten minutes.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::auth::Who;
use crate::deploy::Ask;
use crate::platform::{Actor, Kind, NewEvent, now_ms};
use crate::runtime::{AdminLimits, Budget};
use crate::shell::Shell;
use crate::terminal_logln as println;

use super::format::{capitalize, fuel, plural};
use super::model::{ProblemKind, seen_message};
use super::undo::{Reverse, reverse};
use super::{S, admin_path};

type Form = axum::Form<BTreeMap<String, String>>;

/// What a change did: the toast's message and the event Undo reverses; or
/// why it didn't happen.
pub(super) type Done = Result<(String, Option<i64>), String>;

/// Back after a change: to the admin page the form names in `then`, else
/// `fallback`, at the console with the toast.
pub(super) fn back(form: &BTreeMap<String, String>, fallback: &str, done: Done) -> Response {
    let to = form.get("then").and_then(|t| admin_path(t)).unwrap_or(fallback);
    let mut q = form_urlencoded::Serializer::new(String::new());
    q.append_pair("done", &now_ms().to_string());
    match &done {
        Ok((msg, undo)) => {
            q.append_pair("msg", msg);
            if let Some(id) = undo {
                q.append_pair("undo", &id.to_string());
            }
        }
        Err(msg) => {
            q.append_pair("msg", msg);
            q.append_pair("err", "1");
        }
    }
    let sep = if to.contains('?') { '&' } else { '?' };
    (StatusCode::SEE_OTHER, [(header::LOCATION, format!("{to}{sep}{}#console", q.finish()))]).into_response()
}

/// The app's page, where a change without `then` goes back to.
fn page_of(id: &str) -> String {
    format!("/admin/apps/{}", form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>())
}

/// An app's name and its live label (empty for none): running, or else
/// from what platform.db kept.
async fn app_of(shell: &Shell, id: &str) -> Result<(String, String), String> {
    if let Some(a) = shell.state.app(id) {
        return Ok((a.manifest.app.name.clone(), a.label.clone()));
    }
    let app = id.to_string();
    let (newest, live) =
        shell.platform.run(move |db| Ok((db.versions(&app, 1)?, db.live_of(&app)?))).await.map_err(|e| format!("Could not read {id}: {e:#}"))?;
    let name = newest.first().map(|v| v.name.clone()).ok_or_else(|| "No such app.".to_string())?;
    Ok((name, live.map(|v| v.label).unwrap_or_default()))
}

/// `POST /admin/apps/{id}/off`: visitors get the "turned off" page.
pub(super) async fn turn_off(State(shell): S, who: Who, Path(id): Path<String>, axum::Form(form): Form) -> Response {
    let done = turn(&shell, &id, false, who.person()).await;
    back(&form, &page_of(&id), done)
}

/// `POST /admin/apps/{id}/on`.
pub(super) async fn turn_on(State(shell): S, who: Who, Path(id): Path<String>, axum::Form(form): Form) -> Response {
    let done = turn(&shell, &id, true, who.person()).await;
    back(&form, &page_of(&id), done)
}

async fn turn(shell: &Shell, id: &str, on: bool, person: Option<String>) -> Done {
    let (name, _) = app_of(shell, id).await?;
    let word = if on { "on" } else { "off" };
    match shell.state.turn(id, on, Actor::Admin, person, None).await {
        Ok(event) => {
            println!("turned {id} {word}");
            Ok((format!("{name} is {word}"), Some(event)))
        }
        Err(e) => Err(format!("Could not turn {name} {word}: {e:#}")),
    }
}

/// `POST /admin/apps/{id}/rollback`: the version `label` goes live, an
/// earlier one again or one held for an admin.
pub(super) async fn make_live(State(shell): S, who: Who, Path(id): Path<String>, axum::Form(form): Form) -> Response {
    let done = make_live_now(&shell, &id, form.get("label").map(String::as_str), who.person()).await;
    back(&form, &page_of(&id), done)
}

async fn make_live_now(shell: &Shell, id: &str, label: Option<&str>, person: Option<String>) -> Done {
    match crate::deploy::rollback(shell, id, label, Ask::Admin { person }).await {
        Ok((v, event)) => {
            println!("made {id} {} live", v.label);
            Ok((format!("{} {} is live", v.name, v.version), Some(event)))
        }
        Err(e) => Err(format!("Could not make it live: {e:#}")),
    }
}

/// `POST /admin/apps/{id}/seen`: the problems named in `kinds`
/// ("failing,busy") leave the inbox until something new happens. None
/// named: they come back.
pub(super) async fn seen(State(shell): S, who: Who, Path(id): Path<String>, axum::Form(form): Form) -> Response {
    let kinds: Vec<ProblemKind> = form.get("kinds").map_or_else(Vec::new, |k| k.split(',').filter_map(|k| ProblemKind::parse(k.trim())).collect());
    let done = mark_seen(&shell, &id, &kinds, who.person()).await;
    back(&form, &page_of(&id), done)
}

async fn mark_seen(shell: &Shell, id: &str, kinds: &[ProblemKind], person: Option<String>) -> Done {
    let (name, label) = app_of(shell, id).await?;
    let message = seen_message(kinds);
    let (app, said) = (id.to_string(), message.clone());
    let event = shell
        .platform
        .run(move |db| db.event(&NewEvent { person: person.as_deref(), ..NewEvent::new(&app, &label, Kind::Seen, Actor::Admin, &said) }))
        .await
        .map_err(|e| format!("Could not mark {name} seen: {e:#}"))?;
    println!("{id}: {message}");
    if kinds.is_empty() { Ok((format!("{name} needs you again"), None)) } else { Ok((format!("{name} seen"), Some(event))) }
}

/// `POST /admin/freeze`: `frozen=1` holds every new deploy for an admin,
/// `frozen=0` lets them go live again.
pub(super) async fn freeze(State(shell): S, who: Who, axum::Form(form): Form) -> Response {
    let done = freeze_now(&shell, form.get("frozen").map(String::as_str) != Some("0"), who.person()).await;
    back(&form, "/admin", done)
}

async fn freeze_now(shell: &Shell, frozen: bool, person: Option<String>) -> Done {
    let word = if frozen { "frozen" } else { "unfrozen" };
    match shell.state.freeze(frozen, person).await {
        Ok(event) => {
            println!("deploys {word}");
            Ok((format!("Deploys {word}"), Some(event)))
        }
        Err(e) => Err(format!("Could not {} deploys: {e:#}", if frozen { "freeze" } else { "unfreeze" })),
    }
}

/// `POST /admin/stop-all`: turns every running app off.
pub(super) async fn stop_all(State(shell): S, who: Who, axum::Form(form): Form) -> Response {
    let done = match shell.state.stop_all(who.person()).await {
        Ok(ids) => {
            println!("stop all: turned off {}", plural(ids.len(), "app"));
            // Undo is Start all; the event of any app it turned off says so.
            let first = ids.first().cloned();
            let undo = match first {
                Some(id) => shell.platform.run(move |db| db.off(&id)).await.ok().flatten().map(|e| e.id),
                None => None,
            };
            Ok((format!("Stop all turned {} off", plural(ids.len(), "app")), undo))
        }
        Err(e) => Err(format!("Could not stop all: {e:#}")),
    };
    back(&form, "/admin", done)
}

/// `POST /admin/start-all`: turns on what Stop all turned off.
pub(super) async fn start_all(State(shell): S, who: Who, axum::Form(form): Form) -> Response {
    let done = start_all_now(&shell, who.person()).await;
    back(&form, "/admin", done)
}

async fn start_all_now(shell: &Shell, person: Option<String>) -> Done {
    match shell.state.start_all(person).await {
        Ok(ids) => {
            println!("start all: turned on {}", plural(ids.len(), "app"));
            Ok((format!("Start all turned {} on", plural(ids.len(), "app")), None))
        }
        Err(e) => Err(format!("Could not start all: {e:#}")),
    }
}

/// `POST /admin/apps/{id}/limits`: what one request of the app may use.
pub(super) async fn limits(State(shell): S, who: Who, Path(id): Path<String>, axum::Form(form): Form) -> Response {
    let done = set_limits(&shell, &id, who.person(), |old| from_form(&form, old)).await;
    back(&form, &page_of(&id), done)
}

/// The limits a form asks for: `memory` (MB), `seconds` and `fuel`
/// (millions of instructions). A field left out keeps its limit, an empty
/// one is the default: a fader posts only its own.
fn from_form(form: &BTreeMap<String, String>, old: AdminLimits) -> anyhow::Result<AdminLimits> {
    let field = |key: &str, keep: Option<u32>| -> anyhow::Result<Option<u32>> {
        match form.get(key).map(|v| v.trim()) {
            None => Ok(keep),
            Some("") => Ok(None),
            Some(v) => v.parse().map(Some).map_err(|_| anyhow::anyhow!("{key} = {v:?} is not a whole number")),
        }
    };
    Ok(AdminLimits { memory_mb: field("memory", old.memory_mb)?, seconds: field("seconds", old.seconds)?, fuel_m: field("fuel", old.fuel_m)? })
}

/// Sets an app's limits to what `to` makes of the old ones; one at the
/// app's default is left unset, so it follows the default.
async fn set_limits(shell: &Shell, id: &str, person: Option<String>, to: impl FnOnce(AdminLimits) -> anyhow::Result<AdminLimits> + Send) -> Done {
    let (name, label, toml_time) = match shell.state.app(id) {
        Some(app) => (app.manifest.app.name.clone(), app.label.clone(), app.manifest.time_limit()),
        None => (app_of(shell, id).await?.0, String::new(), Duration::from_secs(crate::manifest::SECONDS)),
    };
    let mut said = String::new();
    let change = |old: AdminLimits| -> anyhow::Result<(AdminLimits, String)> {
        let asked = to(old)?;
        let defaults = Budget::with(toml_time, AdminLimits::default());
        let new = AdminLimits {
            memory_mb: asked.memory_mb.filter(|&mb| mb as usize * 1024 * 1024 != defaults.memory),
            seconds: asked.seconds.filter(|&s| u64::from(s) != defaults.time.as_secs()),
            fuel_m: asked.fuel_m.filter(|&m| u64::from(m) * 1_000_000 != defaults.fuel),
        };
        new.check().map_err(anyhow::Error::msg)?;
        let (was, now) = (Budget::with(toml_time, old), Budget::with(toml_time, new));
        let mut changes = vec![];
        if was.memory != now.memory {
            changes.push(format!("memory {} → {} MB", was.memory >> 20, now.memory >> 20));
        }
        if was.time != now.time {
            changes.push(format!("time {} → {} s", was.time.as_secs(), now.time.as_secs()));
        }
        if was.fuel != now.fuel {
            changes.push(format!("fuel {} → {}", fuel(was.fuel), fuel(now.fuel)));
        }
        if changes.is_empty() {
            anyhow::bail!("{name} has those limits already");
        }
        said = changes.join(", ");
        Ok((new, format!("Limits per request: {said}")))
    };
    let event = shell.state.set_limits(id, &label, person, change).await.map_err(|e| capitalize(&format!("{e:#}")))?;
    println!("{id}: Limits per request: {said}");
    Ok((format!("{name}: {said}"), Some(event)))
}

/// `POST /admin/undo`: reverses `event`, an admin's change of the last ten
/// minutes, if nothing changed the same thing since.
pub(super) async fn undo(State(shell): S, who: Who, axum::Form(form): Form) -> Response {
    let done = undo_now(&shell, form.get("event").and_then(|e| e.parse().ok()), who.person()).await;
    back(&form, "/admin", done)
}

async fn undo_now(shell: &Shell, event: Option<i64>, person: Option<String>) -> Done {
    let id = event.ok_or("Nothing to undo.")?;
    let reversed =
        shell.platform.run(move |db| Ok(db.event_by_id(id)?.and_then(|e| reverse(db, &e, now_ms())))).await.map_err(|e| format!("Could not undo: {e:#}"))?;
    let done = match reversed.ok_or("Nothing to undo: it changed since, or it is older than ten minutes.")? {
        Reverse::Turn { app, on } => turn(shell, &app, on, person).await,
        Reverse::StartAll => start_all_now(shell, person).await,
        Reverse::Freeze(frozen) => freeze_now(shell, frozen, person).await,
        Reverse::Unseen { app } => mark_seen(shell, &app, &[], person).await,
        Reverse::Limits { app, to } => set_limits(shell, &app, person, move |_| Ok(to)).await,
        Reverse::MakeLive { app, label } => make_live_now(shell, &app, Some(&label), person).await,
    };
    // An undo has no undo of its own.
    done.map(|(msg, _)| (format!("Undone: {msg}"), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test app, loaded from a folder at startup.
    async fn startup_app(shell: &Shell) -> std::sync::Arc<crate::runtime::LoadedApp> {
        let checked = crate::testing::bundle("1", None).checked().unwrap();
        let app = std::sync::Arc::new(crate::runtime::load(&shell.engine, shell.db.clone(), &shell.kits, &checked).await.unwrap());
        shell.state.insert_loaded([app.clone()]);
        app
    }

    fn form(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn location(r: &Response) -> &str {
        r.headers()[header::LOCATION].to_str().unwrap()
    }

    #[tokio::test]
    async fn startup_apps_can_be_marked_seen_and_back() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let app = startup_app(&shell).await;
        shell.state.turn("test", false, Actor::Admin, None, None).await.unwrap();
        let (message, undo) = mark_seen(&shell, "test", &[ProblemKind::TurnedOff], None).await.unwrap();
        assert_eq!(message, "Test seen");
        let events = shell.platform.events(Some("test"), 1).unwrap();
        assert_eq!((events[0].kind, events[0].message.as_str(), events[0].label.as_str()), (Kind::Seen, "Seen: turned-off", app.label.as_str()));
        assert_eq!(undo, Some(events[0].id));
        assert_eq!(undo_now(&shell, undo, None).await.unwrap().0, "Undone: Test needs you again");
        assert_eq!(shell.platform.events(Some("test"), 1).unwrap()[0].message, "Needs you again");
    }

    #[tokio::test]
    async fn limits_are_set_logged_and_undone() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let running = startup_app(&shell).await;
        let set = |pairs: &'static [(&'static str, &'static str)]| {
            let shell = &shell;
            async move { set_limits(shell, "test", None, |old| from_form(&form(pairs), old)).await }
        };
        // The default time stays unset; only what differs is kept.
        let (message, undo) = set(&[("memory", "128"), ("seconds", "2"), ("fuel", "")]).await.unwrap();
        assert_eq!(message, "Test: memory 64 → 128 MB");
        let limits = AdminLimits { memory_mb: Some(128), ..Default::default() };
        assert_eq!(shell.platform.limits().unwrap().get("test"), Some(&limits));
        assert_eq!(shell.state.budget(&running).memory, 128 << 20);
        let events = shell.platform.events(Some("test"), 1).unwrap();
        assert_eq!((events[0].kind, events[0].actor, events[0].id), (Kind::LimitsSet, Actor::Admin, undo.unwrap()));
        assert_eq!(events[0].message, "Limits per request: memory 64 → 128 MB");
        assert_eq!(set(&[("memory", "128")]).await.unwrap_err(), "Test has those limits already");
        // A fader posts only its own field; the others stay.
        let (message, fuel_undo) = set(&[("fuel", "2000")]).await.unwrap();
        assert_eq!(message, "Test: fuel 1 B → 2 B");
        assert_eq!(shell.platform.limits().unwrap().get("test"), Some(&AdminLimits { fuel_m: Some(2000), ..limits }));
        assert!(set(&[("memory", "4")]).await.is_err());
        // Only the newest change can be undone: the fuel, back to 1 B.
        assert!(undo_now(&shell, undo, None).await.unwrap_err().starts_with("Nothing to undo"));
        assert_eq!(undo_now(&shell, fuel_undo, None).await.unwrap().0, "Undone: Test: fuel 2 B → 1 B");
        assert_eq!(shell.state.limits_of("test"), limits);
    }

    #[tokio::test]
    async fn undo_turns_on_what_was_turned_off_and_not_twice() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        startup_app(&shell).await;
        let (message, undo) = turn(&shell, "test", false, None).await.unwrap();
        assert_eq!(message, "Test is off");
        assert_eq!(undo_now(&shell, undo, None).await.unwrap(), ("Undone: Test is on".to_string(), None));
        assert!(!shell.state.is_off("test"));
        assert!(undo_now(&shell, undo, None).await.is_err(), "it is on already");
        let (_, frozen) = freeze_now(&shell, true, None).await.unwrap();
        freeze_now(&shell, false, None).await.unwrap();
        assert!(undo_now(&shell, frozen, None).await.is_err(), "unfrozen since");
    }

    #[test]
    fn back_goes_to_the_console_with_its_toast() {
        let r = back(&form(&[("then", "/admin/apps/calc?page=log")]), "/admin/apps/calc", Ok(("Calc seen".into(), Some(7))));
        let to = location(&r);
        assert!(to.starts_with("/admin/apps/calc?page=log&done=") && to.contains("&msg=Calc+seen&undo=7") && to.ends_with("#console"), "{to}");
        let r = back(&form(&[("then", "/admin")]), "/admin", Err("Could not stop all".into()));
        assert!(location(&r).contains("?done=") && location(&r).contains("&err=1"), "{}", location(&r));
    }

    #[test]
    fn back_goes_only_to_the_admin_screen() {
        for then in ["https://evil.example", "//evil.example/admin", "/admin/../x", "/admin/x\\y", "/admin#x", "/administrator", "/admin/a b"] {
            let r = back(&form(&[("then", then)]), "/admin/apps/calc", Ok(("Calc is off".into(), None)));
            assert!(location(&r).starts_with("/admin/apps/calc?done="), "{then}: {}", location(&r));
        }
    }
}
