//! The platforms (AppOS shells) this CLI knows, like kubeconfig contexts.
//! Kept in ~/.config/appos/config.toml, or $APPOS_CONFIG.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    pub current: Option<String>,
    #[serde(default)]
    pub platforms: Vec<Platform>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Platform {
    pub name: String,
    /// Where the shell's platform API listens.
    pub url: String,
    /// The token `appos login` was given, for a platform that takes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

// The platform API's answers, the same types the shell sends.
pub use appos_shell::api::{Deployed, LivePackages, LogPage, Me, News, NewsEvent, RolledBack, Spec, Version};
use appos_shell::api::{RollbackRequest, SwitchRequest, Switched};

/// Platform responses may provide contract text, never paths outside wit/
/// or source/configuration files that execute when the new app is built.
fn check_contract_files(files: &BTreeMap<String, String>) -> anyhow::Result<()> {
    for name in files.keys() {
        if !name.ends_with(".wit")
            || name.contains(['\\', ':'])
            || name.chars().any(char::is_control)
            || name.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        {
            anyhow::bail!("unsafe platform contract filename {name:?}");
        }
    }
    Ok(())
}

/// A fake's file, below the app's fakes/: <kit>/kit.toml, <kit>/kit.wasm
/// or <kit>/wit/<file>.wit.
pub fn check_fake_path(path: &str) -> anyhow::Result<()> {
    let parts: Vec<&str> = path.split('/').collect();
    let safe = |part: &str| !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', ':']) && !part.chars().any(char::is_control);
    let fits = match parts[..] {
        [_, "kit.toml" | "kit.wasm"] => true,
        [_, "wit", file] => file.ends_with(".wit"),
        _ => false,
    };
    anyhow::ensure!(fits && parts.iter().all(|p| safe(p)), "unsafe fake filename {path:?}");
    Ok(())
}

/// The fakes of a platform's kits (`/_platform/fakes`), by path below the
/// app's fakes/: what `appos dev` runs in place of the kits.
pub fn fetch_fakes(url: &str) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    use base64::Engine as _;
    let text = call(url, None, "/_platform/fakes", None, "")?;
    let files: BTreeMap<String, String> = serde_json::from_str(&text).context("the platform's fakes are no JSON")?;
    files
        .into_iter()
        .map(|(path, data)| {
            check_fake_path(&path)?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(data).with_context(|| format!("fake {path}"))?;
            Ok((path, bytes))
        })
        .collect()
}

/// The config file; it is created by the first `appos login`.
pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("APPOS_CONFIG") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/appos/config.toml")
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&path())
    }

    fn load_from(p: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(p) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("{} is not valid", p.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("could not read {}", p.display())),
        }
    }

    pub fn update(edit: impl FnOnce(&mut Self) -> anyhow::Result<()>) -> anyhow::Result<()> {
        Self::update_from(&path(), edit)
    }

    fn update_from(p: &Path, edit: impl FnOnce(&mut Self) -> anyhow::Result<()>) -> anyhow::Result<()> {
        use sha2::Digest;

        let p = writable_config_path(p)?;
        // Lock a stable neighbor: replacing config.toml changes its inode.
        // Resolve symlinks first so both names use the same lock. Hash the
        // filename to keep the lock name below the filesystem's length limit.
        let name = p.file_name().context("config path needs a filename")?;
        let hash: String = sha2::Sha256::digest(name.as_encoded_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let lock_path = p.with_file_name(format!(".appos-config-{hash}.lock"));
        let lock = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("could not open config lock {}", lock_path.display()))?;
        lock.lock().with_context(|| format!("could not lock {}", p.display()))?;
        let mut config = Self::load_from(&p)?;
        edit(&mut config)?;
        config.save_to(&p)
    }

    fn save_to(&self, p: &Path) -> anyhow::Result<()> {
        write_private(p, &toml::to_string(self)?)
    }

    pub fn current(&self) -> Option<&Platform> {
        let name = self.current.as_ref()?;
        self.platforms.iter().find(|p| &p.name == name)
    }

    /// Adds or updates a platform and makes it the current one.
    pub fn upsert(&mut self, platform: Platform) {
        self.current = Some(platform.name.clone());
        match self.platforms.iter_mut().find(|p| p.name == platform.name) {
            Some(existing) => *existing = platform,
            None => self.platforms.push(platform),
        }
    }
}

/// Follow a final config symlink even before its target exists. Parent
/// canonicalization also makes paths through directory aliases share a lock.
fn writable_config_path(p: &Path) -> anyhow::Result<PathBuf> {
    let mut target = std::path::absolute(p)?;
    for _ in 0..40 {
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = std::fs::read_link(&target)?;
                target = target.parent().context("config symlink needs a parent")?.join(link);
            }
            Ok(_) => return target.canonicalize().with_context(|| format!("could not resolve {}", p.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let parent = target.parent().context("config path needs a parent")?;
                private_dirs(parent)?;
                return Ok(parent.canonicalize()?.join(target.file_name().context("config path needs a filename")?));
            }
            Err(e) => return Err(e).with_context(|| format!("could not resolve {}", p.display())),
        }
    }
    anyhow::bail!("too many config symlinks at {}", p.display())
}

/// A reader sees one complete generation. NamedTempFile is private to the
/// current user on Unix, including when an older config was world-readable.
fn write_private(p: &Path, text: &str) -> anyhow::Result<()> {
    use std::io::Write;

    let parent = p.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    private_dirs(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(text.as_bytes())?;
    tmp.as_file_mut().sync_all()?;
    tmp.persist(p).map_err(|e| e.error).with_context(|| format!("could not write {}", p.display()))?;
    Ok(())
}

/// Existing directories keep their permissions; new configuration
/// directories must not let other users replace private files inside them.
fn private_dirs(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// When this CLI last told each app's news, per platform, on the
/// platform's clock. Normally kept next to the config in seen.toml; a
/// different sidecar is used if that name aliases the config. Losing it
/// only means hearing the last week again.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seen(BTreeMap<String, BTreeMap<String, i64>>);

impl Seen {
    fn path() -> anyhow::Result<PathBuf> {
        Self::path_from(&path())
    }

    fn path_from(config: &Path) -> anyhow::Result<PathBuf> {
        let canonical = config.canonicalize().ok();
        for n in 0..100 {
            let name = match n {
                0 => "seen.toml".into(),
                1 => "seen-news.toml".into(),
                _ => format!("seen-news-{n}.toml"),
            };
            let same_name = config.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.trim_end_matches(['.', ' ']).eq_ignore_ascii_case(&name));
            let candidate = config.with_file_name(name);
            let same_target = canonical.as_ref().is_some_and(|target| candidate.canonicalize().ok().as_ref() == Some(target));
            if !same_name && !same_target {
                return Ok(candidate);
            }
        }
        anyhow::bail!("news state paths alias the platform configuration")
    }

    fn platform_key(platform: &Platform) -> String {
        use sha2::Digest;
        // Names can be reused for another server. Hash the endpoint so
        // state stays separate without copying URL credentials to seen.toml.
        sha2::Sha256::digest(platform.url.trim_end_matches('/').as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn load() -> Self {
        Self::path().ok().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn get(&self, platform: &Platform, app_id: &str) -> Option<i64> {
        self.0.get(&Self::platform_key(platform))?.get(app_id).copied()
    }

    pub fn set(&mut self, platform: &Platform, app_id: &str, now: i64) {
        self.0.entry(Self::platform_key(platform)).or_default().insert(app_id.to_string(), now);
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&path())
    }

    fn save_to(&self, config: &Path) -> anyhow::Result<()> {
        let p = Self::path_from(config)?;
        // Replacing the directory entry also breaks a hard link or a
        // sidecar symlink instead of truncating the linked configuration.
        write_private(&p, &toml::to_string(self)?)
    }
}

/// What a POST sends.
enum Body<'a> {
    Json(serde_json::Value),
    /// app.zip.
    Zip(&'a [u8]),
}

/// Calls the platform API at `base`, with the token if there is one: GET,
/// or POST with a body. A refusal comes back with the reason; read it,
/// don't stop at the status.
fn call(base: &str, token: Option<&str>, path: &str, body: Option<Body>, refused: &str) -> anyhow::Result<String> {
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().http_status_as_error(false).build());
    let bearer = token.map(|t| format!("Bearer {t}")).unwrap_or_default();
    let resp = match body {
        Some(Body::Json(json)) => with(agent.post(&url), &bearer).header("content-type", "application/json").send(json.to_string().as_str()),
        Some(Body::Zip(zip)) => with(agent.post(&url), &bearer).header("content-type", "application/zip").send(zip),
        None => with(agent.get(&url), &bearer).call(),
    };
    let mut resp = resp.with_context(|| format!("could not reach {url}"))?;
    let status = resp.status();
    let text = resp.body_mut().read_to_string()?;
    if !status.is_success() {
        anyhow::bail!("{refused}{text} ({status})");
    }
    Ok(text)
}

/// The token, when there is one.
fn with<B>(req: ureq::RequestBuilder<B>, bearer: &str) -> ureq::RequestBuilder<B> {
    if bearer.is_empty() { req } else { req.header("authorization", bearer) }
}

/// `call`, with the answer read as JSON.
fn call_json<T: serde::de::DeserializeOwned>(p: &Platform, path: &str, body: Option<Body>, refused: &str) -> anyhow::Result<T> {
    let text = call(&p.url, p.token.as_deref(), path, body, refused)?;
    serde_json::from_str(&text).context("the platform answered something unexpected")
}

/// Sends an app as app.zip to a shell, which checks it and makes it live.
pub fn deploy(p: &Platform, zip: &[u8]) -> anyhow::Result<Deployed> {
    call_json(p, "/_platform/deploy", Some(Body::Zip(zip)), "the platform refused the app: ")
}

pub fn versions(p: &Platform, app_id: &str) -> anyhow::Result<Vec<Version>> {
    call_json(p, &format!("/_platform/apps/{app_id}/versions"), None, "")
}

/// What each live app is built with, for `appos audit`.
pub fn packages(p: &Platform) -> anyhow::Result<Vec<LivePackages>> {
    call_json(p, "/_platform/packages", None, "")
}

/// What happened to an app after `since` (a `News::now`); without it, the
/// last week.
pub fn news(p: &Platform, app_id: &str, since: Option<i64>) -> anyhow::Result<News> {
    let query = since.map(|s| format!("?since={s}")).unwrap_or_default();
    call_json(p, &format!("/_platform/apps/{app_id}/news{query}"), None, "")
}

/// An app's log, `limit` events older than `before`; only refusals,
/// automatic rollbacks and limit hits with `failures`.
pub fn log(p: &Platform, app_id: &str, failures: bool, before: Option<i64>, limit: u32) -> anyhow::Result<LogPage> {
    // App ids are lowercase letters, digits and dashes: nothing to encode.
    let mut path = format!("/_platform/log?app={app_id}&limit={limit}");
    if failures {
        path += "&kind=failures";
    }
    if let Some(b) = before {
        path += &format!("&before={b}");
    }
    call_json(p, &path, None, "")
}

/// Turns an app off or on, saying why if `reason` is given.
pub fn turn(p: &Platform, app_id: &str, on: bool, reason: Option<&str>) -> anyhow::Result<()> {
    let body = serde_json::to_value(SwitchRequest { reason: reason.map(str::to_string) })?;
    let _: Switched = call_json(p, &format!("/_platform/apps/{app_id}/{}", if on { "on" } else { "off" }), Some(Body::Json(body)), "")?;
    Ok(())
}

/// Makes an earlier version live again, saying why if `reason` is given.
pub fn rollback(p: &Platform, app_id: &str, label: Option<&str>, reason: Option<&str>) -> anyhow::Result<RolledBack> {
    let body = serde_json::to_value(RollbackRequest { label: label.map(str::to_string), reason: reason.map(str::to_string) })?;
    call_json(p, &format!("/_platform/apps/{app_id}/rollback"), Some(Body::Json(body)), "")
}

/// Whom `token` signs in to the platform at `url`.
pub fn me(url: &str, token: &str) -> anyhow::Result<Me> {
    let text = call(url, Some(token), "/_platform/me", None, "the platform did not take the token: ")?;
    serde_json::from_str(&text).context("the platform answered something unexpected")
}

/// Ends `token` on the platform at `url`; a token it doesn't know ended already.
pub fn logout(url: &str, token: &str) -> anyhow::Result<()> {
    match call(url, Some(token), "/_platform/logout", Some(Body::Json(serde_json::json!({}))), "") {
        Err(e) if e.to_string().ends_with("(404 Not Found)") => Ok(()),
        result => result.map(drop),
    }
}

/// Asks a shell what it offers.
pub fn fetch_spec(url: &str) -> anyhow::Result<Spec> {
    let text = call(url, None, "/_platform/spec", None, "")?;
    let spec: Spec = serde_json::from_str(&text).with_context(|| format!("{}/_platform/spec is not an AppOS platform", url.trim_end_matches('/')))?;
    check_contract_files(&spec.wit)?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_platforms_fakes_stay_in_fakes() {
        for ok in ["acme-crm/kit.toml", "acme-crm/kit.wasm", "acme-crm/wit/crm.wit"] {
            assert!(check_fake_path(ok).is_ok(), "{ok}");
        }
        for bad in
            ["kit.toml", "../kit.toml", "acme-crm/../../x/kit.toml", "acme-crm/run.sh", "acme-crm/wit/x.sh", "/etc/kit.toml", "a/b/kit.toml", "C:/kit.toml"]
        {
            assert!(check_fake_path(bad).is_err(), "{bad}");
        }
    }

    fn add_platform(config: &mut Config, name: &str) -> anyhow::Result<()> {
        config.upsert(Platform { name: name.into(), url: format!("https://{name}.example"), token: None });
        Ok(())
    }

    #[test]
    fn concurrent_config_updates_keep_both_connected_platforms() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::default().save_to(&path).unwrap();
        let (entered, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let first_path = path.clone();
        let first = std::thread::spawn(move || {
            Config::update_from(&first_path, |config| {
                add_platform(config, "first")?;
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        #[cfg(unix)]
        let second_path = {
            let alias = dir.path().join("linked-config.toml");
            std::os::unix::fs::symlink("config.toml", &alias).unwrap();
            alias
        };
        #[cfg(not(unix))]
        let second_path = path.clone();
        let (entered, ready) = mpsc::channel();
        let second = std::thread::spawn(move || {
            Config::update_from(&second_path, |config| {
                add_platform(config, "second")?;
                entered.send(()).unwrap();
                Ok(())
            })
        });
        // The second updater must wait for the first read/edit/write to finish.
        let overlapped = ready.recv_timeout(Duration::from_millis(50)).is_ok();
        release.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        let config = Config::load_from(&path).unwrap();
        let names: Vec<_> = config.platforms.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["first", "second"], "a successful connection was lost");
        assert!(!overlapped, "configuration mutations overlapped");
        assert_eq!(config.current.as_deref(), Some("second"));
    }

    #[test]
    fn a_config_reader_never_combines_two_generations() {
        use std::io::Read;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::update_from(&path, |c| add_platform(c, "first")).unwrap();
        let mut reader = std::fs::File::open(&path).unwrap();
        let mut prefix = [0; 14];
        reader.read_exact(&mut prefix).unwrap();
        Config::update_from(&path, |c| {
            c.platforms.clear();
            add_platform(c, "second")
        })
        .unwrap();
        let mut bytes = prefix.to_vec();
        reader.read_to_end(&mut bytes).unwrap();
        let config: Config = toml::from_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert!(config.current().is_some(), "the reader combined an old current name with the new platforms");
        assert_eq!(config.current.as_deref(), Some("first"));
        assert_eq!(Config::load_from(&path).unwrap().current.as_deref(), Some("second"));
    }

    #[cfg(unix)]
    #[test]
    fn tokens_are_kept_in_the_private_config_and_only_when_given() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Config::update_from(&path, |config| {
            config.upsert(Platform { name: "acme".into(), url: "https://ops.acme.com".into(), token: Some("appos_secret".into()) });
            config.upsert(Platform { name: "local".into(), url: "http://127.0.0.1:8080".into(), token: None });
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.platforms.iter().map(|p| p.token.as_deref()).collect::<Vec<_>>(), [Some("appos_secret"), None]);
        assert_eq!(std::fs::read_to_string(&path).unwrap().matches("token").count(), 1);
    }

    #[test]
    fn config_urls_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appos/config.toml");
        Config::update_from(&path, |config| {
            config.upsert(Platform { name: "private".into(), url: "https://user:secret@platform.example".into(), token: None });
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o077, 0);
        // Existing files from earlier CLI versions must become private too.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
        Config::update_from(&path, |c| add_platform(c, "another")).unwrap();
        assert_eq!(std::fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
    }

    #[cfg(unix)]
    #[test]
    fn updating_config_preserves_existing_and_dangling_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nested/config.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let link = dir.path().join("config.toml");
        std::os::unix::fs::symlink("nested/config.toml", &link).unwrap();
        for name in ["first", "second"] {
            Config::update_from(&link, |c| add_platform(c, name)).unwrap();
            assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
            assert_eq!(Config::load_from(&target).unwrap().current.as_deref(), Some(name));
        }
    }

    #[test]
    fn invalid_config_and_failed_edits_are_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "broken = [").unwrap();
        assert!(Config::update_from(&path, |_| panic!("an invalid config was reset")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken = [");
        Config::default().save_to(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(
            Config::update_from(&path, |c| {
                add_platform(c, "must-not-persist")?;
                anyhow::bail!("invalid selection")
            })
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn long_config_filenames_leave_room_for_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("{}.toml", "c".repeat(250)));
        Config::update_from(&path, |c| add_platform(c, "first")).unwrap();
        assert_eq!(Config::load_from(&path).unwrap().current.as_deref(), Some("first"));
    }

    #[cfg(unix)]
    #[test]
    fn a_config_symlink_cycle_fails_without_running_the_edit() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.toml");
        let b = dir.path().join("b.toml");
        std::os::unix::fs::symlink("b.toml", &a).unwrap();
        std::os::unix::fs::symlink("a.toml", &b).unwrap();
        assert!(Config::update_from(&a, |_| panic!("edited a symlink cycle")).is_err());
        assert_eq!(std::fs::read_link(a).unwrap(), Path::new("b.toml"));
        assert_eq!(std::fs::read_link(b).unwrap(), Path::new("a.toml"));
    }

    #[test]
    fn news_state_does_not_overwrite_a_custom_config_named_seen() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("seen.toml");
        let platform = Platform { name: "demo".into(), url: "http://localhost:9090".into(), token: None };
        let config = Config { current: Some(platform.name.clone()), platforms: vec![platform.clone()] };
        let original = toml::to_string(&config).unwrap();
        std::fs::write(&config_path, &original).unwrap();
        let mut seen = Seen::default();
        seen.set(&platform, "app", 100);
        seen.save_to(&config_path).unwrap();
        assert_eq!(std::fs::read_to_string(config_path).unwrap(), original, "news state destroyed the platform configuration");
    }

    #[test]
    fn a_same_name_platform_at_another_url_does_not_inherit_the_news_cursor() {
        let first = Platform { name: "local".into(), url: "http://first.example".into(), token: None };
        let second = Platform { name: "local".into(), url: "http://second.example".into(), token: None };
        let mut seen = Seen::default();
        seen.set(&first, "app", 10000);
        assert_eq!(seen.get(&first, "app"), Some(10000));
        assert_eq!(seen.get(&second, "app"), None, "the new server's earlier events would be skipped");
        let renamed = Platform { name: "renamed".into(), ..first };
        assert_eq!(seen.get(&renamed, "app"), Some(10000));
    }

    #[test]
    fn news_state_round_trips_without_storing_url_credentials() {
        let p = Platform { name: "demo".into(), url: "https://user:private-secret@platform.example".into(), token: None };
        let mut seen = Seen::default();
        seen.set(&p, "app", 123);
        let text = toml::to_string(&seen).unwrap();
        assert!(!text.contains("private-secret"));
        let restored: Seen = toml::from_str(&text).unwrap();
        assert_eq!(restored.get(&p, "app"), Some(123));
        for filename in ["seen.toml", "SEEN.TOML", "seen.toml."] {
            let sidecar = Seen::path_from(Path::new(filename)).unwrap();
            assert!(!sidecar.file_name().unwrap().to_str().unwrap().eq_ignore_ascii_case(filename.trim_end_matches('.')));
        }
    }

    #[cfg(unix)]
    #[test]
    fn news_state_cannot_overwrite_a_config_aliased_by_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let original = "current = 'demo'\nplatforms = []\n";
        std::fs::write(dir.path().join("seen.toml"), original).unwrap();
        std::os::unix::fs::symlink("seen.toml", &config).unwrap();
        std::os::unix::fs::symlink("seen.toml", dir.path().join("seen-news.toml")).unwrap();
        Seen::default().save_to(&config).unwrap();
        assert_eq!(std::fs::read_to_string(config).unwrap(), original);
    }

    #[test]
    fn news_state_cannot_overwrite_a_config_aliased_by_a_hard_link() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        let original = "current = 'demo'\nplatforms = []\n";
        std::fs::write(&config, original).unwrap();
        std::fs::hard_link(&config, dir.path().join("seen.toml")).unwrap();
        Seen::default().save_to(&config).unwrap();
        assert_eq!(std::fs::read_to_string(config).unwrap(), original);
    }

    #[test]
    fn platform_contract_files_stay_inside_the_wit_directory() {
        let files = |name: &str| BTreeMap::from([(name.to_string(), "untrusted content".to_string())]);
        for name in ["../Cargo.toml", "/tmp/escape.wit", "../escape.wit", "nested/../../escape.wit", r"..\escape.wit", "build.rs", ""] {
            assert!(check_contract_files(&files(name)).is_err(), "accepted unsafe contract filename {name:?}");
        }
        for name in ["appos.wit", "deps/types.wit"] {
            assert!(check_contract_files(&files(name)).is_ok(), "refused {name:?}");
        }
    }
}
