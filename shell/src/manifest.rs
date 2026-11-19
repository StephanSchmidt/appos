//! The app manifest (`app.toml`): who the app is, who owns it, which group
//! it belongs to, which capabilities it is granted and what data it keeps.
//! Routes and menu entries come from the app's
//! `init`. A Wasm kit's kit.toml (wasm_kit.rs) reads into the same, with
//! what the kit offers on top: a kit is held like an app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub app: AppInfo,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub limits: Limits,
    /// What data the app keeps, where and why (`[[data]]`), so the people
    /// who answer for the company's data (GDPR) can find it. A deploy of an
    /// app whose grants let it keep data needs it (`Manifest::data`).
    #[serde(default)]
    pub data: Vec<Data>,
    /// What a kit offers, from its kit.toml; none for an app.
    #[serde(skip)]
    pub kit: Option<crate::wasm_kit::Offers>,
}

/// One kind of data an app keeps, in the words of whoever built it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Data {
    /// What it keeps, like "Name and email address of each sign-up".
    #[serde(default)]
    pub what: String,
    /// Where: "kv", or the kit it hands the data to.
    #[serde(default, rename = "where")]
    pub place: String,
    /// Why, like "Hold seats and send the webinar link".
    #[serde(default)]
    pub why: String,
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

/// An app's manifest.
pub const APP_TOML: &str = "app.toml";
/// A Wasm kit's.
pub const KIT_TOML: &str = "kit.toml";

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
    Ok(())
}

/// Why `grant`, which no kit offers now, is a mistake, if it is: a Wasm
/// kit's grant may come with its deploy, but one that looks like a grant
/// of the platform's own kits (`known`) is misspelt: kv-wrte. Patterns
/// (kv-*) are checked the same.
pub fn misspelt(grant: &str, known: &[String]) -> Option<String> {
    let prefix = |g: &str| g.split('-').next().unwrap_or(g).to_string();
    let wellformed = if grant.contains('*') {
        grant.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '*')
    } else {
        validate_grant_name(grant).is_ok()
    };
    if !wellformed {
        return Some(format!("{grant:?} is no grant: lowercase letters, digits and dashes"));
    }
    let like: Vec<&str> = known.iter().filter(|k| prefix(k) == prefix(grant)).map(String::as_str).collect();
    (!like.is_empty()).then(|| format!("{grant:?} is no grant; the {}- grants are {}", prefix(grant), like.join(", ")))
}

/// The highest risk level of a grant.
pub const MAX_RISK: u8 = 10;

/// The highest risk score of an app.
pub const MAX_SCORE: u8 = 100;

/// How much harm each grant can do, from 0 (none) to 10, as the company
/// running the shell rates it (`--risk risk.toml`); apps can't change it.
/// Grants held together can be rated too: `"crm-read+fetch-post" = 9`
/// rates an app that can read the CRM and send to the internet higher
/// than either grant alone. An app scores 0 to 100 from all of them
/// (`score`).
#[derive(Debug, Clone, Default)]
pub struct Risk(BTreeMap<String, u8>);

impl Risk {
    /// Reads a file of `grant = level` lines.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    }

    /// Reads `grant = level` lines, and `"grant+grant" = level` for grants
    /// together; `check` then holds them against the kits.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let read: BTreeMap<String, u8> = toml::from_str(text)?;
        let mut levels = BTreeMap::new();
        for (grant, level) in read {
            if level > MAX_RISK {
                anyhow::bail!("{grant} = {level}: risk goes from 0 to {MAX_RISK}");
            }
            // Grants together in one order, so "b+a" is "a+b".
            let mut parts: Vec<&str> = grant.split('+').map(str::trim).collect();
            parts.sort_unstable();
            parts.dedup();
            if grant.contains('+') && (parts.len() < 2 || parts.contains(&"")) {
                anyhow::bail!("{grant:?}: give two grants or more, like \"crm-read+fetch-post\"");
            }
            let key = parts.join("+");
            if levels.insert(key.clone(), level).is_some() {
                anyhow::bail!("{key} is rated twice");
            }
        }
        Ok(Self(levels))
    }

    /// Refuses grants that look like a misspelt grant of the platform's
    /// own kits (`misspelt`): a Wasm kit's may come later.
    pub fn check(&self, kits: &crate::kit::Kits) -> anyhow::Result<()> {
        let known = kits.names();
        match self.0.keys().flat_map(|k| k.split('+')).filter(|g| kits.grant(g).is_none()).find_map(|g| misspelt(g, &known)) {
            Some(why) => anyhow::bail!("risk file: {why}"),
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

    /// An app's score from 0 to 100 (`score`), 0 without grants, none
    /// while one of its grants is unrated.
    pub fn of_app(&self, m: &Manifest) -> Option<u8> {
        self.of_grants(&m.capabilities.grant)
    }

    /// The score of these grants together, as for an app.
    pub fn of_grants(&self, grants: &[String]) -> Option<u8> {
        Some(score(&self.parts(grants)?.iter().map(|(_, l)| *l).collect::<Vec<_>>()))
    }

    /// What an app's score comes from: each grant and each rated
    /// combination it holds all of, with its level; none while a grant is
    /// unrated.
    pub fn parts<'a>(&'a self, grants: &'a [String]) -> Option<Vec<(&'a str, u8)>> {
        let mut parts: Vec<(&str, u8)> = grants.iter().map(|g| self.of(g).map(|l| (g.as_str(), l))).collect::<Option<_>>()?;
        let holds = |key: &str| key.contains('+') && key.split('+').all(|g| grants.iter().any(|h| h == g));
        parts.extend(self.0.iter().filter(|(k, _)| holds(k)).map(|(k, l)| (k.as_str(), *l)));
        Some(parts)
    }
}

/// An app's score from its levels, 0 to 100. Each level is a chance of
/// harm, (level/10)², and the score the chance that at least one does
/// harm, back on the levels' scale:
/// 100 × √(1 − (1 − (l₁/10)²) × (1 − (l₂/10)²) × …).
/// One grant scores its level × 10. Low levels add up to little (three 1s
/// score 17, less than one 3), high ones push towards 100 (three 9s score
/// 99, more than one 9).
pub fn score(levels: &[u8]) -> u8 {
    let safe: f64 = levels.iter().map(|l| 1.0 - (f64::from(*l.min(&MAX_RISK)) / 10.0).powi(2)).product();
    // A hair up, so a single 9 is 90 and not 89.99…
    (100.0 * (1.0 - safe).sqrt() + 1e-9).floor() as u8
}

/// Risk bands of an app's score, fixed: Low 0-39, Medium 40-69, High
/// 70-89, Critical 90-100. A grant's level is in the band of its level
/// × 10.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Band {
    Low,
    Medium,
    High,
    Critical,
}

impl Band {
    pub const ALL: [Band; 4] = [Band::Low, Band::Medium, Band::High, Band::Critical];

    /// The band of an app's score.
    pub fn of(score: u8) -> Band {
        match score {
            0..=39 => Band::Low,
            40..=69 => Band::Medium,
            70..=89 => Band::High,
            _ => Band::Critical,
        }
    }

    /// The band of a grant's level.
    pub fn of_level(level: u8) -> Band {
        Band::of(level.saturating_mul(10))
    }

    pub fn name(self) -> &'static str {
        match self {
            Band::Low => "Low",
            Band::Medium => "Medium",
            Band::High => "High",
            Band::Critical => "Critical",
        }
    }

    /// The scores in the band.
    pub fn levels(self) -> &'static str {
        match self {
            Band::Low => "0-39",
            Band::Medium => "40-69",
            Band::High => "70-89",
            Band::Critical => "90-100",
        }
    }
}

/// The longest owner, in characters.
pub const MAX_OWNER: usize = 200;
/// The longest group, in characters.
pub const MAX_GROUP: usize = 60;
/// `[[data]]` entries one app may have.
pub const MAX_DATA: usize = 32;
/// The longest what, where or why of one, in characters.
pub const MAX_DATA_TEXT: usize = 300;
/// What a `[[data]]` entry looks like, for messages.
const DATA_EXAMPLE: &str = "[[data]]\nwhat = \"Name and email address of each sign-up\"\nwhere = \"kv\"\nwhy = \"Hold seats and send the webinar link\"";

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
    /// Reads `app.toml` or `kit.toml`, or the one in a directory (kit.toml
    /// if it has one). Returns it, its text and where its .wasm is.
    pub fn load(path: &Path) -> anyhow::Result<(Self, String, PathBuf)> {
        let file = match path.is_dir() {
            true if path.join(KIT_TOML).is_file() => path.join(KIT_TOML),
            true => path.join(APP_TOML),
            false => path.to_path_buf(),
        };
        let text = std::fs::read_to_string(&file).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let kit = file.file_name().is_some_and(|n| n == KIT_TOML);
        let m = if kit { Self::parse_kit(&text) } else { Self::parse(&text) };
        let m = m.map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let wasm = file.parent().unwrap_or(Path::new(".")).join(&m.app.wasm);
        Ok((m, text, wasm))
    }

    /// Reads and checks the text of an `app.toml`.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let m: Manifest = toml::from_str(text)?;
        m.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(m)
    }

    /// Reads and checks the text of a Wasm kit's `kit.toml`.
    pub fn parse_kit(text: &str) -> anyhow::Result<Self> {
        let m = crate::wasm_kit::manifest(text)?;
        m.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(m)
    }

    /// Whether it is a kit's.
    pub fn is_kit(&self) -> bool {
        self.kit.is_some()
    }

    /// "app.toml" or "kit.toml", for messages.
    pub fn file(&self) -> &'static str {
        if self.is_kit() { KIT_TOML } else { APP_TOML }
    }

    /// "[app]" or "[kit]", for messages.
    fn section(&self) -> &'static str {
        if self.is_kit() { "[kit]" } else { "[app]" }
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
            if cap.starts_with(crate::access::ADMIN_PREFIX) {
                return Err(format!("[capabilities] {cap} is for people, in policy.csv: the admin- grants are the admin screen's"));
            }
        }
        if self.capabilities.grant.len() > MAX_GRANTS {
            return Err(format!("[capabilities] grant: at most {MAX_GRANTS} grants"));
        }
        Ok(())
    }

    /// Who to ask about the app, trimmed. A deploy without one is refused.
    pub fn owner(&self) -> anyhow::Result<&str> {
        let (file, section, what) = (self.file(), self.section(), if self.is_kit() { "kit" } else { "app" });
        let owner = self.app.owner.as_deref().unwrap_or("").trim();
        if owner.is_empty() {
            anyhow::bail!("{file} needs an owner under {section}: the person or team to ask about this {what}, like owner = \"Jane Doe <jane@example.com>\"");
        }
        if owner.chars().count() > MAX_OWNER {
            anyhow::bail!("{file}: owner is longer than {MAX_OWNER} characters");
        }
        if owner.chars().any(char::is_control) {
            anyhow::bail!("{file}: owner is one line of text");
        }
        Ok(owner)
    }

    /// The group the app belongs to, trimmed. A deploy without one is refused.
    pub fn group(&self) -> anyhow::Result<&str> {
        let (file, section, what) = (self.file(), self.section(), if self.is_kit() { "kit" } else { "app" });
        let group = self.app.group.as_deref().unwrap_or("").trim();
        if group.is_empty() {
            anyhow::bail!("{file} needs a group under {section}: the team the {what} belongs to, like group = \"Marketing\"");
        }
        if group.chars().count() > MAX_GROUP {
            anyhow::bail!("{file}: group is longer than {MAX_GROUP} characters");
        }
        if group.chars().any(char::is_control) {
            anyhow::bail!("{file}: group is one line of text");
        }
        Ok(group)
    }

    /// Its grants that let it keep data: kv-write, and every kit's grant,
    /// as a kit may keep what it is handed. Reading kv or who is signed in
    /// keeps nothing.
    pub fn keeping(&self) -> Vec<&str> {
        use crate::kit::{KV_READ, USER_READ};
        self.capabilities.grant.iter().map(String::as_str).filter(|g| ![KV_READ, USER_READ].contains(g)).collect()
    }

    /// What data the app keeps, each entry with a what, where and why. A
    /// deploy of an app that can keep data (`keeping`) without one is
    /// refused; a kit says nothing here yet.
    pub fn data(&self) -> anyhow::Result<&[Data]> {
        let file = self.file();
        if self.data.len() > MAX_DATA {
            anyhow::bail!("{file}: at most {MAX_DATA} [[data]] entries");
        }
        let keeping = self.keeping();
        for (n, d) in self.data.iter().enumerate().map(|(i, d)| (i + 1, d)) {
            let fields = [
                ("what", &d.what, "what the app keeps, like \"Name and email address of each sign-up\""),
                ("where", &d.place, "where it keeps it: \"kv\", or the kit it hands it to"),
                ("why", &d.why, "why it keeps it, like \"Hold seats and send the webinar link\""),
            ];
            for (field, text, means) in fields {
                let text = text.trim();
                if text.is_empty() {
                    let unless = if keeping.is_empty() { "; or remove the entry, as no grant lets the app keep data" } else { "" };
                    anyhow::bail!("{file}: [[data]] entry {n} needs {field}: {means}{unless}");
                }
                if text.chars().count() > MAX_DATA_TEXT {
                    anyhow::bail!("{file}: [[data]] entry {n}: {field} is longer than {MAX_DATA_TEXT} characters");
                }
                if text.chars().any(char::is_control) {
                    anyhow::bail!("{file}: [[data]] entry {n}: {field} is one line of text");
                }
            }
        }
        if self.data.is_empty() && !keeping.is_empty() && !self.is_kit() {
            anyhow::bail!(
                "{file} needs a [[data]] entry: its grants ({}) let it keep data. Say what it keeps, where and why, \
                 so the people who answer for the company's data (GDPR) know:\n\n{DATA_EXAMPLE}\n\n\
                 One entry for each kind of data; when it keeps nothing about people, say so in what.",
                keeping.join(", ")
            );
        }
        Ok(&self.data)
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
    fn an_app_scores_its_grants_together() {
        let risk = Risk::parse("kv-read = 1\nkv-write = 7").unwrap();
        assert_eq!(risk.of_app(&app(r#""kv-write""#)), Some(70), "one grant: its level × 10");
        assert_eq!(risk.of_app(&app(r#""kv-read", "kv-write""#)), Some(70));
        assert_eq!(risk.of_app(&app("")), Some(0));
        assert_eq!(Risk::parse("kv-read = 1").unwrap().of_app(&app(r#""kv-read", "kv-write""#)), None);
    }

    #[test]
    fn low_levels_add_up_to_little_and_high_ones_to_more() {
        assert_eq!(score(&[3]), 30);
        assert_eq!(score(&[1, 1, 1]), 17, "less than one 3");
        assert_eq!(score(&[1; 10]), 30);
        assert_eq!(score(&[5, 5]), 66);
        assert_eq!(score(&[9]), 90);
        assert_eq!(score(&[9, 9, 9]), 99, "more than one 9");
        assert_eq!(score(&[10, 0]), 100);
        assert_eq!(score(&[]), 0);
        for l in 0..=MAX_RISK {
            assert_eq!(score(&[l]), l * 10);
        }
    }

    #[test]
    fn grants_held_together_can_be_rated_higher() {
        let risk = Risk::parse("kv-read = 1\nkv-write = 2\nuser-read = 1\n\"user-read + kv-write\" = 6").unwrap();
        assert_eq!(risk.of_app(&app(r#""kv-write", "user-read""#)), Some(score(&[2, 1, 6])), "who the user is, kept");
        assert_eq!(risk.of_app(&app(r#""kv-read", "user-read""#)), Some(score(&[1, 1])), "one of the two");
        assert_eq!(risk.of("kv-write+user-read"), Some(6), "in one order");
        assert_eq!(Risk::parse("\"kv-write+user-read\" = 6").unwrap().of_app(&app(r#""kv-write", "user-read""#)), None, "unrated grants stay unrated");
        assert!(Risk::parse("\"kv-read+\" = 3").unwrap_err().to_string().contains("two grants or more"));
        assert!(Risk::parse("\"kv-read+kv-read\" = 3").is_err());
        assert!(Risk::parse("\"a+b\" = 3\n\"b+a\" = 4").unwrap_err().to_string().contains("a+b is rated twice"));
        assert!(Risk::parse("\"kv-red+user-read\" = 3").unwrap().check(&crate::kit::Kits::builtin()).is_err(), "each grant is checked");
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
        assert!(
            Risk::parse("kv-fly = 2").unwrap().check(&crate::kit::Kits::builtin()).unwrap_err().to_string().contains("the kv- grants are kv-read, kv-write")
        );
        assert!(Risk::parse("score-read = 4").unwrap().check(&crate::kit::Kits::builtin()).is_ok(), "a Wasm kit's grant may come later");
    }

    #[test]
    fn grants_are_names() {
        assert!(Manifest::parse("[app]\nid = 'a'\nname = 'A'\nversion = '1'\n[capabilities]\ngrant = ['leads-read']").is_ok(), "the kits say which exist");
        for bad in ["Kv-Read", "kv read", "kv-*", "", "admin-read", "1kv"] {
            assert!(Manifest::parse(&format!("[app]\nid = 'a'\nname = 'A'\nversion = '1'\n[capabilities]\ngrant = ['{bad}']")).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_app_that_can_keep_data_says_what_where_and_why() {
        let entry = "\n[[data]]\nwhat = 'Email address of each sign-up'\nwhere = 'kv'\nwhy = 'Send the link'";
        let with = |grants: &str, data: &str| {
            Manifest::parse(&format!("[app]\nid = 'a'\nname = 'A'\nversion = '1'\n[capabilities]\ngrant = [{grants}]{data}")).unwrap()
        };
        for grants in ["'kv-write'", "'leads-read'", "'kv-read', 'leads-write'"] {
            let err = with(grants, "").data().unwrap_err().to_string();
            assert!(err.contains("needs a [[data]] entry") && err.contains("where = \"kv\""), "{grants}: {err}");
            assert_eq!(with(grants, entry).data().unwrap()[0].place, "kv", "{grants}");
        }
        assert!(with("'kv-read', 'user-read'", "").data().unwrap().is_empty(), "reading keeps nothing");
        let blank = with("'kv-write'", &entry.replace("'Send the link'", "' '")).data().unwrap_err().to_string();
        assert!(blank.contains("entry 1 needs why"), "{blank}");
        let unneeded = with("", &entry.replace("'kv'", "''")).data().unwrap_err().to_string();
        assert!(unneeded.contains("needs where") && unneeded.contains("or remove the entry"), "{unneeded}");
        assert!(with("'kv-write'", &entry.replace("'kv'", "\"kv\\nand more\"")).data().is_err(), "one line");
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
