//! Deploys, versions and rollback. `appos deploy` sends an app as app.zip
//! (bundle.rs) to the platform API. The shell loads it with the checks of
//! every app (`Bundle::checked`), plus an owner and a group, keeps it as
//! <data>/apps/<id>/<label>/app.zip, and swaps it in (state.rs). Requests
//! already running finish on the old version; kv data stays.
//!
//! Every version stays stored, so any earlier one can be made live again:
//! by `appos rollback`, on the admin screen, or automatically when the live
//! version keeps failing.
//!
//! An automatic rollback pauses the app: deploys are checked and stored but
//! wait, until someone makes a version live on the admin screen. The
//! builder can't ship over a failure an admin hasn't seen.
//!
//! The kill switch turns an app off: it answers with a "turned off" page
//! and runs nothing; deploys and rollbacks still happen. An app an admin
//! turned off goes back on only from the admin screen.
//!
//! An admin can freeze deploys: like the pause, every app's deploys are
//! checked and stored but wait, and the CLI can't roll back, until an admin
//! unfreezes them. Stop all turns every running app off at once; Start all
//! turns those back on.
//!
//! The platform API takes a token from `appos login` and the policy's
//! `admin` (web.rs, `api_only`); with `--no-auth`, whoever reaches it can
//! deploy, roll back and turn apps off.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};

use crate::bundle::{self, Bundle, Checked};
use crate::manifest::Manifest;
use crate::platform::{Actor, NewVersion, Version};
use crate::runtime::{self, LoadedApp};
use crate::shell::Shell;
use crate::{terminal_errorln as eprintln, terminal_logln as println};

/// Failed requests within this window count towards an automatic rollback.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(60);
pub const FAILURES_BEFORE_ROLLBACK: usize = 5;

/// Where a version is kept. Ids and labels come from platform.db: they
/// are checked before they become a path.
fn version_dir(data: &Path, app_id: &str, label: &str) -> anyhow::Result<PathBuf> {
    for part in [app_id, label] {
        let mut components = Path::new(part).components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
            || part.contains(['/', '\\', ':'])
            || part.chars().any(char::is_control)
        {
            bail!("unsafe stored app or version path {part:?}");
        }
    }
    Ok(data.join("apps").join(app_id).join(label))
}

/// A deploy that loaded: live, or held while the app is paused.
pub struct Deployed {
    pub app: Arc<LoadedApp>,
    /// Why it waits instead of going live.
    pub held: Option<String>,
}

/// Loads the app in `zip`, stores it and makes it live, or holds it while
/// the app is paused or deploys are frozen. An app that doesn't load
/// changes nothing and is recorded as refused; a zip without a readable
/// app.toml names no app and is only answered.
#[cfg(test)]
pub async fn deploy(shell: &Shell, zip: &[u8], actor: Actor) -> anyhow::Result<Deployed> {
    deploy_as(shell, zip, actor, None).await
}

/// `deploy`, by `person` (the email of who sent it).
pub async fn deploy_as(shell: &Shell, zip: &[u8], actor: Actor, person: Option<String>) -> anyhow::Result<Deployed> {
    let started = Instant::now();
    if zip.len() > bundle::MAX {
        bail!("app.zip is larger than {} MB", bundle::MAX >> 20);
    }
    // One deploy unzips and compiles at a time, so a tiny zip bomb can't
    // make every queued deploy hold another 64 MB. Changes to what runs
    // don't wait for this.
    let _one_at_a_time = shell.loads.acquire().await?;
    let zip = zip.to_vec();
    let (bundle, m) = tokio::task::spawn_blocking(move || {
        let bundle = Bundle::unpack(&zip)?;
        let m = Manifest::parse(&bundle.manifest)?;
        anyhow::Ok((bundle, m))
    })
    .await??;
    let v = NewVersion {
        app_id: m.app.id.clone(),
        label: bundle.label(&m.app.version),
        version: m.app.version.clone(),
        name: m.app.name.clone(),
        owner: m.app.owner.as_deref().unwrap_or("").trim().to_string(),
        group: m.app.group.as_deref().unwrap_or("").trim().to_string(),
        actor,
        person,
        risk: shell.risk.of_app(&m),
        grants: m.capabilities.grant.clone(),
        routes: vec![],
        packages: None,
    };

    let loaded = async {
        bundle::check(&bundle.manifest)?;
        let checked = bundle.checked()?;
        let app = runtime::load(&shell.engine, shell.db.clone(), &shell.kits, &checked).await?;
        let packages = checked.packages.clone();
        // Kept as `pack` writes it, whatever zipped it: one file per version.
        let (data, stored) = (shell.data.clone(), checked.bundle.clone());
        let (id, label) = (v.app_id.clone(), v.label.clone());
        tokio::task::spawn_blocking(move || {
            let dir = version_dir(&data, &id, &label)?;
            std::fs::create_dir_all(&dir)?;
            store_bundle(&dir.join(bundle::FILE), &stored)
        })
        .await??;
        anyhow::Ok((app, packages))
    }
    .await;
    let (app, packages) = match loaded {
        Ok((app, packages)) => (Arc::new(app), packages),
        Err(e) => {
            let (refused, why) = (v.clone(), format!("{e:#}"));
            shell.platform.run(move |db| db.refused(&refused, &why, started.elapsed())).await?;
            return Err(e);
        }
    };
    let v = NewVersion { routes: app.routes.iter().map(|r| r.pattern()).collect(), packages, ..v };
    let held = shell.state.deployed(app.clone(), v, started).await?;
    Ok(Deployed { app, held })
}

/// Loads a stored version, with the checks of every app, and that it is
/// what was stored under this id and label.
async fn load_stored(shell: &Shell, app_id: &str, label: &str) -> anyhow::Result<LoadedApp> {
    let path = version_dir(&shell.data, app_id, label)?.join(bundle::FILE);
    let (expected_id, expected_label) = (app_id.to_string(), label.to_string());
    let checked: Checked = tokio::task::spawn_blocking(move || {
        let zip = bundle::read_file(&path).with_context(|| format!("{}", path.display()))?;
        let checked = Bundle::unpack(&zip).and_then(Bundle::checked).with_context(|| format!("{}", path.display()))?;
        if checked.manifest.app.id != expected_id {
            bail!("stored bundle belongs to {}, expected {expected_id}", checked.manifest.app.id);
        }
        if checked.label != expected_label {
            bail!("stored bundle content does not match version {expected_label}");
        }
        anyhow::Ok(checked)
    })
    .await??;
    runtime::load(&shell.engine, shell.db.clone(), &shell.kits, &checked).await
}

/// At startup: loads the live version of every deployed app. The shell
/// doesn't start without knowing them; one that doesn't load is told.
pub async fn load_live(shell: &Shell) -> anyhow::Result<()> {
    let live = shell.platform.run(|db| db.live()).await.context("could not read the live versions")?;
    for v in live {
        match load_stored(shell, &v.app_id, &v.label).await {
            Ok(app) => {
                println!("loaded {} {} at /{} ({} routes)", v.app_id, v.label, v.app_id, app.routes.len());
                shell.state.insert_loaded([Arc::new(app)]);
            }
            Err(e) => eprintln!("could not load {} {}: {e:#}", v.app_id, v.label),
        }
    }
    Ok(())
}

/// The longest reason kept, in characters.
const MAX_REASON: usize = 500;

/// A reason as given for a rollback: trimmed, none if empty, at most 500
/// characters.
pub fn reason(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(MAX_REASON).collect())
}

/// Who asks for a rollback.
#[derive(Debug, Clone)]
pub enum Ask {
    /// The appos CLI, with a reason or none, as the person its token is.
    Cli { reason: Option<String>, person: Option<String> },
    /// The admin screen, as whoever is signed in there.
    Admin { person: Option<String> },
    /// The shell: `label` kept failing since `since` (unix ms).
    Failing { label: String, reason: String, since: i64 },
}

/// Makes an earlier version live again: `label`, or else the one deployed
/// before the live one. The shell's own rollback does nothing if the
/// failing version is no longer live. The version loads first; what runs
/// changes only if nothing else did meanwhile. Returns the version and the
/// event's id.
pub async fn rollback(shell: &Shell, app_id: &str, label: Option<&str>, ask: Ask) -> anyhow::Result<(Version, i64)> {
    let (_, to) = shell.state.rollback_target(app_id, label, &ask).await?;
    let app = load_stored(shell, app_id, &to.label).await.with_context(|| format!("could not load {}", to.label))?;
    let event = shell.state.roll_back(app_id, label, ask, &to, Arc::new(app)).await?;
    Ok((to, event))
}

/// Counts a failed request. If the live version keeps failing, the version
/// before it comes back.
pub fn note_failure(shell: &Arc<Shell>, app_id: &str, label: &str) {
    let Some(since) = shell.state.failed(app_id, label) else { return };
    let (shell, app_id, label) = (shell.clone(), app_id.to_string(), label.to_string());
    tokio::spawn(async move {
        // Times of day are the reader's to show (events have `at` and
        // `recovery_ms`); the reason stays the same in every time zone.
        let reason = format!("{FAILURES_BEFORE_ROLLBACK} failed requests in {}", seconds(crate::platform::now_ms() - since));
        let ask = Ask::Failing { label: label.clone(), reason: reason.clone(), since };
        match rollback(&shell, &app_id, None, ask).await {
            Ok((to, _)) => println!("[{app_id}] {label} had {reason}; rolled back to {}", to.label),
            Err(e) => eprintln!("[{app_id}] {label} had {reason}; no rollback: {e:#}"),
        }
    });
}

/// Like "0.4 s" or "38 s".
fn seconds(ms: i64) -> String {
    if ms < 10_000 { format!("{:.1} s", ms as f64 / 1000.0) } else { format!("{} s", (ms + 500) / 1000) }
}

/// A version is immutable. Identical redeploys may reuse it, but a hash
/// collision or a damaged store must never replace an earlier bundle.
fn store_bundle(path: &Path, bundle: &Bundle) -> anyhow::Result<()> {
    let same = |bytes: &[u8]| -> anyhow::Result<()> {
        let stored = Bundle::unpack(bytes).context("existing stored bundle is damaged")?;
        if stored != *bundle {
            bail!("version already stores different content; refusing to overwrite {}", path.display());
        }
        Ok(())
    };
    match bundle::read_file(path) {
        Ok(bytes) => return same(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("could not read {}", path.display())),
    }
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
    tmp.write_all(&bundle.pack()?)?;
    tmp.as_file_mut().sync_all()?;
    match tmp.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => same(&bundle::read_file(path)?),
        Err(e) => Err(e.error).with_context(|| format!("could not store {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn switches_also_control_apps_loaded_at_startup() {
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let checked = crate::testing::bundle("1", None).checked().unwrap();
        let app = runtime::load(&shell.engine, shell.db.clone(), &shell.kits, &checked).await.unwrap();
        shell.state.insert_loaded([Arc::new(app)]);
        assert!(shell.platform.live_of("test").unwrap().is_none());

        assert_eq!(shell.state.stop_all(None).await.unwrap(), ["test"]);
        assert_eq!(shell.platform.stopped_all().unwrap(), ["test"]);
        let response =
            crate::web::router(shell.clone()).oneshot(axum::http::Request::builder().uri("/test").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(shell.state.start_all(None).await.unwrap(), ["test"]);
        assert!(!shell.state.is_off("test"));

        shell.state.turn("test", false, Actor::Admin, None, Some("investigate".into())).await.unwrap();
        assert!(shell.state.turn("test", true, Actor::Cli, None, None).await.is_err());
        shell.state.turn("test", true, Actor::Admin, None, None).await.unwrap();
        assert!(!shell.state.is_off("test"));
        assert_eq!(shell.platform.events(Some("test"), 10).unwrap().len(), 4);
    }

    /// The kill switch and Stop all don't wait for a deploy that is still
    /// loading.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn turning_off_does_not_wait_for_a_running_deploy() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        deploy(&shell, &crate::testing::bundle("1.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        // Its init never ends: the deploy loads until the time limit.
        let mut slow = crate::testing::bundle("2.0", None);
        slow.wasm = String::from_utf8(slow.wasm).unwrap().replace(r#"(func (export "init")"#, r#"(func (export "init") (loop $spin (br $spin))"#).into_bytes();
        let loading = tokio::spawn({
            let shell = shell.clone();
            async move { deploy(&shell, &slow.pack().unwrap(), Actor::Cli).await.map(|_| ()) }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = Instant::now();
        shell.state.turn("test", false, Actor::Admin, None, None).await.unwrap();
        shell.state.turn("test", true, Actor::Admin, None, None).await.unwrap();
        assert_eq!(shell.state.stop_all(None).await.unwrap(), ["test"]);
        assert!(started.elapsed() < Duration::from_millis(500), "took {:?}", started.elapsed());
        assert!(!loading.is_finished(), "the deploy ended before the switches were tried");
        let err = loading.await.unwrap().unwrap_err();
        assert!(format!("{err:#}").contains("init"), "{err:#}");
        assert!(shell.state.is_off("test"));
    }

    #[tokio::test]
    async fn a_freeze_holds_deploys_and_cli_rollbacks_until_unfrozen() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let first = deploy(&shell, &crate::testing::bundle("1.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        shell.state.freeze(true, None).await.unwrap();
        assert!(shell.state.freeze(true, None).await.is_err(), "frozen twice");
        let second = deploy(&shell, &crate::testing::bundle("2.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        assert!(second.held.as_deref().is_some_and(|h| h.contains("froze deploys")), "{:?}", second.held);
        assert_eq!(shell.platform.live_of("test").unwrap().unwrap().label, first.app.label);
        let err = rollback(&shell, "test", None, Ask::Cli { reason: None, person: None }).await.unwrap_err();
        assert!(format!("{err:#}").contains("froze deploys"), "{err:#}");
        // An admin makes the held version live; after the thaw, deploys go live again.
        rollback(&shell, "test", Some(&second.app.label), Ask::Admin { person: None }).await.unwrap();
        assert_eq!(shell.platform.live_of("test").unwrap().unwrap().label, second.app.label);
        shell.state.freeze(false, None).await.unwrap();
        assert!(shell.platform.frozen().unwrap().is_none());
        let third = deploy(&shell, &crate::testing::bundle("3.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        assert!(third.held.is_none());
    }

    /// A rollback records the risk of the version it makes live.
    #[tokio::test]
    async fn a_rollback_records_the_restored_versions_risk() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell_with(dir.path().to_path_buf(), crate::manifest::Risk::parse("kv-read = 9").unwrap());
        let risky = crate::testing::bundle("1.0", None);
        let risky = Bundle { manifest: risky.manifest + "[capabilities]\ngrant = ['kv-read']\n", ..risky };
        let first = deploy(&shell, &risky.pack().unwrap(), Actor::Cli).await.unwrap();
        deploy(&shell, &crate::testing::bundle("2.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        rollback(&shell, "test", Some(&first.app.label), Ask::Cli { reason: None, person: None }).await.unwrap();
        assert_eq!(shell.platform.events(Some("test"), 1).unwrap()[0].risk, Some(9));
    }

    #[tokio::test]
    async fn start_all_turns_on_only_what_stop_all_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        deploy(&shell, &crate::testing::bundle("1.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        let mut other = crate::testing::bundle("1.0", None);
        other.manifest = other.manifest.replace("id = 'test'", "id = 'other'");
        deploy(&shell, &other.pack().unwrap(), Actor::Cli).await.unwrap();
        // Turned off on its own first: Start all leaves it off.
        shell.state.turn("other", false, Actor::Admin, None, Some("leaks data".into())).await.unwrap();
        assert_eq!(shell.state.stop_all(None).await.unwrap(), ["test"]);
        assert!(shell.state.is_off("test"));
        assert_eq!(shell.platform.stopped_all().unwrap(), ["test"]);
        assert_eq!(shell.state.start_all(None).await.unwrap(), ["test"]);
        assert!(!shell.state.is_off("test") && shell.state.is_off("other"));
        assert!(shell.platform.stopped_all().unwrap().is_empty());
        assert!(shell.state.start_all(None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn changing_an_icon_preserves_the_previous_bundle_for_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let first = crate::testing::bundle("1.0", Some("<svg><!-- original --></svg>"));
        let second = crate::testing::bundle("1.0", Some("<svg><!-- replacement --></svg>"));
        let original = deploy(&shell, &first.pack().unwrap(), Actor::Cli).await.unwrap();
        let replacement = deploy(&shell, &second.pack().unwrap(), Actor::Cli).await.unwrap();
        let kept = Bundle::unpack(&std::fs::read(version_dir(dir.path(), "test", &original.app.label).unwrap().join(bundle::FILE)).unwrap()).unwrap();
        assert!(kept == first, "a later deploy overwrote a stored version");
        assert_ne!(original.app.label, replacement.app.label);
        rollback(&shell, "test", Some(&original.app.label), Ask::Cli { reason: None, person: None }).await.unwrap();
        assert_eq!(shell.state.app("test").unwrap().icon, first.icon);
    }

    #[tokio::test]
    async fn colliding_short_hashes_do_not_replace_a_previous_version() {
        use sha2::Digest;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let mut first = crate::testing::bundle("1.0", None);
        let mut second = crate::testing::bundle("1.0", None);
        // These valid manifests collide under the original 32-bit label.
        // If testing.wat or its manifest changes, regenerate the two nonce comments.
        first.manifest += "# nonce 81819\n";
        second.manifest += "# nonce 86113\n";
        let old_hash = |d: &Bundle| sha2::Sha256::new().chain_update(&d.manifest).chain_update(&d.wasm).finalize()[..4].to_vec();
        assert_eq!(old_hash(&first), old_hash(&second));
        let original = deploy(&shell, &first.pack().unwrap(), Actor::Cli).await.unwrap();
        let replacement = deploy(&shell, &second.pack().unwrap(), Actor::Cli).await.unwrap();
        let kept = Bundle::unpack(&std::fs::read(version_dir(dir.path(), "test", &original.app.label).unwrap().join(bundle::FILE)).unwrap()).unwrap();
        assert_eq!(kept.manifest, first.manifest, "a hash collision overwrote a stored version");
        assert_ne!(original.app.label, replacement.app.label);
    }

    #[test]
    fn stored_bundles_are_immutable_even_if_a_label_collides() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(bundle::FILE);
        let first = crate::testing::bundle("1.0", None);
        let second = crate::testing::bundle("2.0", None);
        store_bundle(&path, &first).unwrap();
        let original = std::fs::read(&path).unwrap();
        store_bundle(&path, &first).unwrap();
        assert!(store_bundle(&path, &second).unwrap_err().to_string().contains("refusing to overwrite"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::write(&path, b"damaged").unwrap();
        assert!(store_bundle(&path, &first).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"damaged");
    }

    #[tokio::test]
    async fn rollback_checks_stored_content_and_keeps_the_current_app_if_it_was_changed() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let first = crate::testing::bundle("1.0", None);
        let original = deploy(&shell, &first.pack().unwrap(), Actor::Cli).await.unwrap();
        let live = deploy(&shell, &crate::testing::bundle("2.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        let path = version_dir(dir.path(), "test", &original.app.label).unwrap().join(bundle::FILE);
        let mut altered = first;
        altered.manifest = altered.manifest.replace("name = 'Test'", "name = 'Altered'");
        std::fs::write(&path, altered.pack().unwrap()).unwrap();
        let err = rollback(&shell, "test", Some(&original.app.label), Ask::Cli { reason: None, person: None }).await.unwrap_err();
        assert!(format!("{err:#}").contains("does not match version"));
        assert_eq!(shell.platform.live_of("test").unwrap().unwrap().label, live.app.label);
        assert_eq!(shell.state.app("test").unwrap().label, live.app.label);
        // Nor does another app's bundle load under this one's name.
        let mut other = crate::testing::bundle("1.0", None);
        other.manifest = other.manifest.replace("id = 'test'", "id = 'other'");
        std::fs::write(&path, other.pack().unwrap()).unwrap();
        let err = load_stored(&shell, "test", &original.app.label).await.err().unwrap();
        assert!(err.to_string().contains("belongs to other, expected test"), "{err:#}");
    }

    #[tokio::test]
    async fn the_longest_allowed_id_and_version_still_fit_in_storage() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let id = "a".repeat(255);
        let mut bundle = crate::testing::bundle(&"v".repeat(200), None);
        bundle.manifest = bundle.manifest.replace("id = 'test'", &format!("id = '{id}'"));
        let result = deploy(&shell, &bundle.pack().unwrap(), Actor::Cli).await.unwrap();
        assert!(result.app.label.len() <= 255);
        assert!(version_dir(dir.path(), &id, &result.app.label).unwrap().join(bundle::FILE).is_file());
        bundle.manifest = bundle.manifest.replace(&format!("id = '{id}'"), &format!("id = '{id}a'"));
        let error = deploy(&shell, &bundle.pack().unwrap(), Actor::Cli).await.err().unwrap();
        assert!(error.to_string().contains("longer than 255 bytes"));
        assert_eq!(shell.state.apps().len(), 1);
    }

    #[tokio::test]
    async fn a_storage_failure_is_recorded_as_a_refused_deploy() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        std::fs::write(dir.path().join("apps"), b"not a directory").unwrap();
        assert!(deploy(&shell, &crate::testing::bundle("1.0", None).pack().unwrap(), Actor::Cli).await.is_err());
        let versions = shell.platform.versions("test", 10).unwrap();
        assert_eq!(versions.len(), 1, "a refused deploy disappeared from the app's history");
        assert_eq!(versions[0].status, crate::platform::VersionStatus::Refused);
        assert!(versions[0].reason.is_some());
        assert_eq!(shell.platform.events(Some("test"), 10).unwrap()[0].kind, crate::platform::Kind::Refused);
        assert!(shell.state.apps().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn decoding_a_zip_bomb_yields_to_other_requests() {
        use std::future::Future;
        use std::task::Poll;

        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("app.toml", options).unwrap();
        zip.write_all(crate::testing::bundle("1", None).manifest.as_bytes()).unwrap();
        zip.start_file("app.wasm", options).unwrap();
        zip.write_all(&vec![0; bundle::MAX + 1]).unwrap();
        let zip = zip.finish().unwrap().into_inner();
        let mut future = Box::pin(deploy(&shell, &zip, Actor::Cli));
        let first = std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await;
        assert!(first.is_pending(), "archive decoding blocked the executor until the entire bomb was rejected");
        let err = future.await.err().unwrap();
        assert!(format!("{err:#}").contains("unpacks to more than 64 MB"));
    }

    #[test]
    fn stored_versions_stay_in_the_app_directory() {
        let data = Path::new("data");
        for part in ["../../outside", "/tmp/outside", "a/b", r"a\b", ".", "..", ""] {
            assert!(version_dir(data, "app", part).is_err(), "accepted version {part:?}");
            assert!(version_dir(data, part, "1.0-12345678").is_err(), "accepted app {part:?}");
        }
        assert_eq!(version_dir(data, "app", "1.0-12345678").unwrap(), data.join("apps/app/1.0-12345678"));
    }

    /// The window of an automatic rollback, as its reason says it.
    #[test]
    fn the_window_reads_in_seconds() {
        assert_eq!(seconds(400), "0.4 s");
        assert_eq!(seconds(9_949), "9.9 s");
        assert_eq!(seconds(10_000), "10 s");
        assert_eq!(seconds(38_499), "38 s");
        assert_eq!(seconds(38_500), "39 s");
    }

    #[test]
    fn a_reason_is_trimmed_and_capped() {
        assert_eq!(reason("  the form loses leads \n"), Some("the form loses leads".to_string()));
        assert_eq!(reason(" \n\t "), None);
        let long = "ä".repeat(MAX_REASON + 20);
        assert_eq!(reason(&long).map(|r| r.chars().count()), Some(MAX_REASON));
    }
}
