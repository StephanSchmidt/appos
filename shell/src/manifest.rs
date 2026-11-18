//! The app manifest (`app.toml`): who the app is, who owns it, which group
//! it belongs to and which capabilities it is granted. Routes and menu entries come from the app's
//! `init`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub app: AppInfo,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Who to ask about the app: a person or team, like "Jane Doe
    /// <jane@example.com>". A deploy needs one (`Manifest::owner`); stored
    /// versions and `appos dev` don't.
    #[serde(default)]
    pub owner: Option<String>,
    /// The team or department the app belongs to, like "Marketing": the
    /// admin screen groups apps by it. A deploy needs one
    /// (`Manifest::group`), like the owner.
    #[serde(default)]
    pub group: Option<String>,
    /// The component, relative to the manifest.
    #[serde(default = "default_wasm")]
    pub wasm: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub grant: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Limits {
    /// Seconds one request may take, from its start to page::done.
    pub seconds: Option<u64>,
}

/// Seconds a request may take unless app.toml says otherwise, and the most
/// it may ask for.
pub const SECONDS: u64 = 2;
pub const MAX_SECONDS: u64 = 30;

fn default_wasm() -> PathBuf {
    PathBuf::from("app.wasm")
}

/// Grants one app may hold.
pub const MAX_GRANTS: usize = 64;
/// Bytes of a grant's name.
const MAX_GRANT_NAME: usize = 60;

/// A grant's name, as kits offer it and app.toml asks for it: lowercase
/// letters, digits and dashes, like "kv-read". The kits say which exist
/// (kit.rs). `admin` is the admin screen's, in the policy.
pub fn validate_grant_name(name: &str) -> Result<(), String> {
    let ok = name.len() <= MAX_GRANT_NAME
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(format!("grant {name:?} must be up to {MAX_GRANT_NAME} lowercase letters, digits and dashes, starting with a letter"));
    }
    if name == crate::access::ADMIN {
        return Err("admin is the admin screen's, not a grant".into());
    }
    Ok(())
}

/// The highest risk level.
pub const MAX_RISK: u8 = 10;

/// How much harm each grant can do, from 0 (none) to 10, as the company
/// running the shell rates it (`--risk risk.toml`); apps can't change it. An
/// app is as risky as its riskiest grant.
#[derive(Debug, Clone, Default)]
pub struct Risk(BTreeMap<String, u8>);

impl Risk {
    /// Reads a file of `grant = level` lines.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    }

    /// Reads `grant = level` lines; `check` then holds them against the
    /// kits.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let levels: BTreeMap<String, u8> = toml::from_str(text)?;
        for (grant, level) in &levels {
            if *level > MAX_RISK {
                anyhow::bail!("{grant} = {level}: risk goes from 0 to {MAX_RISK}");
            }
        }
        Ok(Self(levels))
    }

    /// Refuses grants no kit offers.
    pub fn check(&self, kits: &crate::kit::Kits) -> anyhow::Result<()> {
        match self.0.keys().find(|g| kits.grant(g).is_none()) {
            Some(grant) => anyhow::bail!("risk file: unknown grant {grant:?}; known: {}", kits.names().join(", ")),
            None => Ok(()),
        }
    }

    /// Every rated grant and its level.
    pub fn levels(&self) -> &BTreeMap<String, u8> {
        &self.0
    }

    /// A grant's level, if the company rated it.
    pub fn of(&self, grant: &str) -> Option<u8> {
        self.0.get(grant).copied()
    }

    /// An app's level: its riskiest grant, 0 without grants, none while one
    /// of its grants is unrated.
    pub fn of_app(&self, m: &Manifest) -> Option<u8> {
        self.of_grants(&m.capabilities.grant)
    }

    /// The level of these grants together, as for an app.
    pub fn of_grants(&self, grants: &[String]) -> Option<u8> {
        grants.iter().try_fold(0, |max, g| self.of(g).map(|l| max.max(l)))
    }
}

/// Risk bands, fixed: Low 0-3, Medium 4-6, High 7-8, Critical 9-10.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Band {
    Low,
    Medium,
    High,
    Critical,
}

impl Band {
    pub const ALL: [Band; 4] = [Band::Low, Band::Medium, Band::High, Band::Critical];

    pub fn of(level: u8) -> Band {
        match level {
            0..=3 => Band::Low,
            4..=6 => Band::Medium,
            7..=8 => Band::High,
            _ => Band::Critical,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Band::Low => "Low",
            Band::Medium => "Medium",
            Band::High => "High",
            Band::Critical => "Critical",
        }
    }

    /// The levels in the band.
    pub fn levels(self) -> &'static str {
        match self {
            Band::Low => "0-3",
            Band::Medium => "4-6",
            Band::High => "7-8",
            Band::Critical => "9-10",
        }
    }
}

/// The longest owner, in characters.
pub const MAX_OWNER: usize = 200;
/// The longest group, in characters.
pub const MAX_GROUP: usize = 60;

/// App ids the shell keeps for itself. Ids have no underscore, so
/// `_platform` needn't be here.
const RESERVED_IDS: &[&str] = &["assets", "admin"];

/// The same namespace rules apply to startup manifests and CLI commands.
/// IDs also become directory names in the version store.
pub fn validate_app_id(id: &str) -> Result<(), String> {
    if id.len() > 255 {
        return Err("app id is longer than 255 bytes".into());
    }
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(format!("app id {id:?} must be lowercase letters, digits and dashes"));
    }
    if RESERVED_IDS.contains(&id) {
        return Err(format!("app id {id:?} is reserved for the shell"));
    }
    Ok(())
}

impl Manifest {
    /// Reads `app.toml`, or the `app.toml` in a directory. Returns it, its
    /// text and where its .wasm is.
    pub fn load(path: &Path) -> anyhow::Result<(Self, String, PathBuf)> {
        let file = if path.is_dir() { path.join("app.toml") } else { path.to_path_buf() };
        let text = std::fs::read_to_string(&file).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let m = Self::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let wasm = file.parent().unwrap_or(Path::new(".")).join(&m.app.wasm);
        Ok((m, text, wasm))
    }

    /// Reads and checks the text of an `app.toml`.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let m: Manifest = toml::from_str(text)?;
        m.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(m)
    }

    fn validate(&self) -> Result<(), String> {
        validate_app_id(&self.app.id)?;
        // The version becomes part of a directory name (<version>-<hash>).
        // Keep it a portable filename, including room for the hash suffix.
        let version = &self.app.version;
        if version.is_empty() || version.len() > 200 || !version.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-+".contains(&c)) {
            return Err("app version must be 1 to 200 ASCII letters, digits, dots, underscores, dashes or plus signs".to_string());
        }
        if let Some(s) = self.limits.seconds
            && !(1..=MAX_SECONDS).contains(&s)
        {
            return Err(format!("[limits] seconds = {s}: must be 1 to {MAX_SECONDS}"));
        }
        // Which grants exist, the kits say when the app loads (runtime.rs).
        for cap in &self.capabilities.grant {
            validate_grant_name(cap)?;
        }
        if self.capabilities.grant.len() > MAX_GRANTS {
            return Err(format!("[capabilities] grant: at most {MAX_GRANTS} grants"));
        }
        Ok(())
    }

    /// Who to ask about the app, trimmed. A deploy without one is refused.
    pub fn owner(&self) -> anyhow::Result<&str> {
        let owner = self.app.owner.as_deref().unwrap_or("").trim();
        if owner.is_empty() {
            anyhow::bail!("app.toml needs an owner under [app]: the person or team to ask about this app, like owner = \"Jane Doe <jane@example.com>\"");
        }
        if owner.chars().count() > MAX_OWNER {
            anyhow::bail!("app.toml: owner is longer than {MAX_OWNER} characters");
        }
        if owner.chars().any(char::is_control) {
            anyhow::bail!("app.toml: owner is one line of text");
        }
        Ok(owner)
    }

    /// The group the app belongs to, trimmed. A deploy without one is refused.
    pub fn group(&self) -> anyhow::Result<&str> {
        let group = self.app.group.as_deref().unwrap_or("").trim();
        if group.is_empty() {
            anyhow::bail!("app.toml needs a group under [app]: the team the app belongs to, like group = \"Marketing\"");
        }
        if group.chars().count() > MAX_GROUP {
            anyhow::bail!("app.toml: group is longer than {MAX_GROUP} characters");
        }
        if group.chars().any(char::is_control) {
            anyhow::bail!("app.toml: group is one line of text");
        }
        Ok(group)
    }

    /// How long one request may take.
    pub fn time_limit(&self) -> Duration {
        Duration::from_secs(self.limits.seconds.unwrap_or(SECONDS))
    }

    pub fn grants(&self, grant: &str) -> bool {
        self.capabilities.grant.iter().any(|g| g == grant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(grants: &str) -> Manifest {
        Manifest::parse(&format!("[app]\nid = \"a\"\nname = \"A\"\nversion = \"0.1.0\"\n[capabilities]\ngrant = [{grants}]")).unwrap()
    }

    #[test]
    fn an_app_is_as_risky_as_its_riskiest_grant() {
        let risk = Risk::parse("kv-read = 1\nkv-write = 7").unwrap();
        assert_eq!(risk.of_app(&app(r#""kv-read", "kv-write""#)), Some(7));
        assert_eq!(risk.of_app(&app("")), Some(0));
        assert_eq!(Risk::parse("kv-read = 1").unwrap().of_app(&app(r#""kv-read", "kv-write""#)), None);
    }

    #[test]
    fn every_grant_says_what_it_allows() {
        let kits = crate::kit::Kits::builtin();
        for grant in kits.names() {
            assert_ne!(kits.allows(&grant), kits.allows("unknown"), "{grant}");
        }
    }

    #[test]
    fn risk_files_name_known_grants_from_0_to_10() {
        assert!(Risk::parse("kv-read = 11").is_err());
        assert!(Risk::parse("kv = 1").unwrap().check(&crate::kit::Kits::builtin()).is_err());
        assert!(Risk::parse("kv-read = 10").unwrap().check(&crate::kit::Kits::builtin()).is_ok());
    }

    #[test]
    fn grants_are_names() {
        assert!(Manifest::parse("[app]\nid = 'a'\nname = 'A'\nversion = '1'\n[capabilities]\ngrant = ['leads-read']").is_ok(), "the kits say which exist");
        for bad in ["Kv-Read", "kv read", "kv-*", "", "admin", "1kv"] {
            assert!(Manifest::parse(&format!("[app]\nid = 'a'\nname = 'A'\nversion = '1'\n[capabilities]\ngrant = ['{bad}']")).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn versions_cannot_escape_the_app_directory() {
        let parse = |version: &str| Manifest::parse(&format!("[app]\nid = 'a'\nname = 'A'\nversion = '{version}'"));
        for version in ["../../outside", "/tmp/outside", "a/b", r"a\b", "", "release\t1"] {
            assert!(parse(version).is_err(), "accepted unsafe version {version:?}");
        }
        assert!(parse(&"v".repeat(247)).is_err(), "the label must fit in a filesystem component");
        for version in ["0.1.0", "1.0.0-rc.1+build.2", "release_2026"] {
            assert!(parse(version).is_ok(), "refused version {version:?}");
        }
    }
}
