//! OSV's advisories (https://ossf.github.io/osv-schema/), and which affect
//! a package: of the ecosystems in ecosystem.rs, the rest left out.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Read};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use super::{Ecosystem, Purl};

/// The most an advisories zip may hold, in files and unpacked bytes.
const MAX_ADVISORIES: usize = 100_000;
const MAX_ADVISORY: usize = 1024 * 1024;
const MAX_UNPACKED: usize = 512 * 1024 * 1024;

names! {
    /// What an advisory says about a package.
    pub enum Kind {
        /// The package is malware.
        Malicious = "malicious",
        /// A known vulnerability.
        Vulnerability = "vulnerability",
        /// It can break memory safety; no exploit is known.
        Unsound = "unsound",
        /// Nobody maintains it: fixes won't come.
        Unmaintained = "unmaintained",
        /// Anything else worth knowing.
        Notice = "notice",
    }
}

impl Kind {
    /// Worth knowing, but nothing known is exploitable.
    pub fn is_warning(self) -> bool {
        matches!(self, Kind::Unsound | Kind::Unmaintained | Kind::Notice)
    }
}

/// An advisory against a package an app is built with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The package URL, as in deps.json.
    pub package: String,
    /// Like RUSTSEC-2020-0071 or GHSA-wcg3-cvx6-7396.
    pub id: String,
    pub kind: Kind,
    pub summary: String,
    pub url: String,
    /// The same advisory elsewhere, like CVE-2020-26235.
    pub aliases: Vec<String>,
    /// The versions to upgrade to: the first fix `cargo update` reaches, if
    /// one does, and the newest fix; none if there is no fix yet.
    pub fixed: Vec<String>,
}

/// What OSV says of an advisory, of what the matching needs.
#[derive(Deserialize)]
struct Osv {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    details: String,
    withdrawn: Option<String>,
    #[serde(default)]
    affected: Vec<OsvAffected>,
}

#[derive(Deserialize)]
struct OsvAffected {
    package: Option<OsvPackage>,
    #[serde(default)]
    ranges: Vec<OsvRange>,
    #[serde(default)]
    versions: Vec<String>,
    database_specific: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct OsvPackage {
    ecosystem: String,
    name: String,
}

#[derive(Deserialize)]
struct OsvRange {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    events: Vec<OsvEvent>,
}

#[derive(Deserialize)]
struct OsvEvent {
    introduced: Option<String>,
    fixed: Option<String>,
    last_affected: Option<String>,
}

struct Meta {
    id: String,
    summary: String,
    aliases: Vec<String>,
}

#[derive(Clone, Copy)]
enum Event {
    Introduced,
    Fixed,
    LastAffected,
}

/// What one advisory says of one package.
struct Affected {
    advisory: usize,
    kind: Kind,
    /// Each sorted by version.
    ranges: Vec<Vec<(semver::Version, Event)>>,
    versions: Vec<semver::Version>,
}

impl Affected {
    /// The OSV spec's ranges: affected from an introduced version until a
    /// fixed one, or up to and with a last_affected one.
    fn affects(&self, v: &semver::Version) -> bool {
        self.versions.contains(v)
            || self.ranges.iter().any(|events| {
                events.iter().fold(false, |affected, (at, event)| match event {
                    Event::Introduced if v >= at => true,
                    Event::Fixed if v >= at => false,
                    Event::LastAffected if v > at => false,
                    _ => affected,
                })
            })
    }

    fn fixed_after<'a>(&'a self, v: &'a semver::Version) -> impl Iterator<Item = &'a semver::Version> {
        self.ranges.iter().flatten().filter(move |(at, e)| matches!(e, Event::Fixed) && at > v).map(|(at, _)| at)
    }
}

/// The advisories of the ecosystems loaded, by package.
#[derive(Default)]
pub struct Advisories {
    meta: Vec<Meta>,
    /// By the ecosystem's package URL type, then the package's name.
    packages: HashMap<&'static str, HashMap<String, Vec<Affected>>>,
    /// Advisories in the zip that could not be read, left out.
    pub unreadable: usize,
}

impl Advisories {
    /// OSV's all.zip of an ecosystem (`Ecosystem::advisories`).
    pub fn from_zip(bytes: &[u8]) -> anyhow::Result<Advisories> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| anyhow::anyhow!("the advisories are not a zip file"))?;
        if zip.len() > MAX_ADVISORIES {
            bail!("the advisories zip has {} files, more than {MAX_ADVISORIES}", zip.len());
        }
        let mut out = Advisories::default();
        let mut total = 0;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).context("the advisories zip is damaged")?;
            if file.is_dir() {
                continue;
            }
            let mut text = Vec::new();
            file.by_ref().take(MAX_ADVISORY as u64 + 1).read_to_end(&mut text).context("the advisories zip is damaged")?;
            total += text.len();
            if total > MAX_UNPACKED {
                bail!("the advisories unpack to more than {} MB", MAX_UNPACKED >> 20);
            }
            if text.len() > MAX_ADVISORY || out.add(&text).is_err() {
                out.unreadable += 1;
            }
        }
        Ok(out)
    }

    /// Adds another ecosystem's advisories.
    pub fn append(&mut self, other: Advisories) {
        let offset = self.meta.len();
        self.meta.extend(other.meta);
        for (ecosystem, packages) in other.packages {
            for (name, affected) in packages {
                let into = self.packages.entry(ecosystem).or_default().entry(name).or_default();
                into.extend(affected.into_iter().map(|a| Affected { advisory: a.advisory + offset, ..a }));
            }
        }
        self.unreadable += other.unreadable;
    }

    /// Adds one advisory in OSV's JSON. Withdrawn ones and what they say of
    /// other ecosystems are left out.
    pub fn add(&mut self, json: &[u8]) -> anyhow::Result<()> {
        let osv: Osv = serde_json::from_slice(json)?;
        if osv.withdrawn.is_some() {
            return Ok(());
        }
        let advisory = self.meta.len();
        let mut any = false;
        for a in osv.affected {
            let Some((ecosystem, name)) = a.package.and_then(|p| Some((Ecosystem::of_osv(&p.ecosystem)?, p.name))) else { continue };
            let informational = a.database_specific.as_ref().and_then(|d| d.get("informational")).and_then(serde_json::Value::as_str);
            let kind = match informational {
                Some("unmaintained") => Kind::Unmaintained,
                Some("unsound") => Kind::Unsound,
                Some(_) => Kind::Notice,
                None if osv.id.starts_with("MAL-") => Kind::Malicious,
                None => Kind::Vulnerability,
            };
            // The versions are semver (ecosystem.rs), whether a range says
            // SEMVER or ECOSYSTEM; a range with a version that doesn't read
            // is left out.
            let ranges = a
                .ranges
                .iter()
                .filter(|r| r.kind == "SEMVER" || r.kind == "ECOSYSTEM")
                .filter_map(|r| {
                    let mut events = r.events.iter().map(event).collect::<Option<Vec<_>>>()?;
                    events.sort_by(|a, b| a.0.cmp(&b.0));
                    Some(events)
                })
                .collect();
            let versions = a.versions.iter().filter_map(|v| version(v)).collect();
            self.packages.entry(ecosystem.purl).or_default().entry(name).or_default().push(Affected { advisory, kind, ranges, versions });
            any = true;
        }
        if any {
            let summary = match osv.summary.trim() {
                "" => osv.details.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().chars().take(160).collect(),
                s => s.to_string(),
            };
            self.meta.push(Meta { id: osv.id, summary, aliases: osv.aliases });
        }
        Ok(())
    }

    /// How many advisories are known.
    pub fn len(&self) -> usize {
        self.meta.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meta.is_empty()
    }

    /// The advisories against a package, by its URL; none for a package of
    /// an ecosystem not loaded. One advisory in several databases
    /// (RUSTSEC-..., GHSA-..., MAL-...) is told once.
    pub fn of(&self, purl: &str) -> Vec<Finding> {
        let Ok(p) = Purl::parse(purl) else { return vec![] };
        let affected = self.packages.get(p.kind).and_then(|packages| packages.get(p.name));
        let (Some(affected), Some(v)) = (affected, version(p.version)) else { return vec![] };
        // An advisory may say several things of one package.
        let mut matched: BTreeMap<usize, (Kind, BTreeSet<&semver::Version>)> = BTreeMap::new();
        for a in affected.iter().filter(|a| a.affects(&v)) {
            matched.entry(a.advisory).or_insert((a.kind, BTreeSet::new())).1.extend(a.fixed_after(&v));
        }
        let mut matched: Vec<_> = matched.into_iter().map(|(i, m)| (&self.meta[i], m)).collect();
        matched.sort_by_key(|(m, _)| (preference(&m.id), m.id.clone()));
        let mut seen = HashSet::new();
        let mut out = vec![];
        for (m, (kind, fixed)) in matched {
            if seen.contains(m.id.as_str()) || m.aliases.iter().any(|a| seen.contains(a.as_str())) {
                continue;
            }
            seen.insert(m.id.as_str());
            seen.extend(m.aliases.iter().map(String::as_str));
            out.push(Finding {
                package: purl.to_string(),
                id: m.id.clone(),
                kind,
                summary: m.summary.clone(),
                url: format!("https://osv.dev/vulnerability/{}", m.id),
                aliases: m.aliases.clone(),
                fixed: upgrades(&v, &fixed),
            });
        }
        out
    }
}

/// Of the fixes newer than `v`, the ones worth telling. Every fix is a
/// version that isn't affected, and RustSec's unaffected versions make
/// many (time 0.2.0 to 0.2.6 for 0.2.23); an upgrade takes the first that
/// `cargo update` (or `npm update`) reaches, or the newest.
fn upgrades(v: &semver::Version, fixed: &BTreeSet<&semver::Version>) -> Vec<String> {
    // The caret, cargo's and npm's: the same leftmost non-zero number.
    let reachable = |f: &semver::Version| match (v.major, v.minor) {
        (0, 0) => f.major == 0 && f.minor == 0 && f.patch == v.patch,
        (0, minor) => f.major == 0 && f.minor == minor,
        (major, _) => f.major == major,
    };
    let mut out: Vec<String> = fixed.iter().find(|f| reachable(f)).into_iter().chain(fixed.last()).map(ToString::to_string).collect();
    out.dedup();
    out
}

/// Of one advisory in several databases, the one told: the malware list
/// knows it is malware, RustSec knows Rust best.
fn preference(id: &str) -> u8 {
    ["MAL-", "RUSTSEC-", "GHSA-"].iter().position(|p| id.starts_with(p)).unwrap_or(3) as u8
}

fn event(e: &OsvEvent) -> Option<(semver::Version, Event)> {
    match (&e.introduced, &e.fixed, &e.last_affected) {
        (Some(v), None, None) => Some((version(v)?, Event::Introduced)),
        (None, Some(v), None) => Some((version(v)?, Event::Fixed)),
        (None, None, Some(v)) => Some((version(v)?, Event::LastAffected)),
        _ => None,
    }
}

/// A version as an advisory or a lock file writes it. "0" is before every
/// version, "0.65" is 0.65.0; build metadata doesn't count.
fn version(s: &str) -> Option<semver::Version> {
    if s == "0" {
        return semver::Version::parse("0.0.0-0").ok();
    }
    let core = s.find(['-', '+']).map_or(s, |i| &s[..i]);
    let padded = format!("{core}{}{}", ".0".repeat(2usize.saturating_sub(core.matches('.').count())), &s[core.len()..]);
    let mut v = semver::Version::parse(&padded).ok()?;
    v.build = semver::BuildMetadata::EMPTY;
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advisories(json: &[&str]) -> Advisories {
        let mut a = Advisories::default();
        for j in json {
            a.add(j.as_bytes()).unwrap();
        }
        a
    }

    fn osv(id: &str, aliases: &str, krate: &str, ranges: &str, extra: &str) -> String {
        format!(
            r#"{{"id":"{id}","aliases":[{aliases}],"summary":"{id} in {krate}","affected":[{{"package":{{"ecosystem":"crates.io","name":"{krate}"}},"ranges":[{ranges}]{extra}}}]}}"#
        )
    }

    fn ids(found: &[Finding]) -> Vec<&str> {
        found.iter().map(|f| f.id.as_str()).collect()
    }

    #[test]
    fn ranges_follow_the_osv_spec() {
        let a = advisories(&[
            &osv("RUSTSEC-1", "", "time", r#"{"type":"SEMVER","events":[{"introduced":"0"},{"fixed":"0.2.23"}]}"#, ""),
            &osv(
                "RUSTSEC-2",
                "",
                "hyper",
                r#"{"type":"SEMVER","events":[{"introduced":"0.14.0"},{"fixed":"0.14.10"},{"introduced":"1.0.0"},{"last_affected":"1.1.0"}]}"#,
                "",
            ),
            &osv("RUSTSEC-3", "", "listed", "", r#","versions":["0.3.1"]"#),
            &osv("RUSTSEC-4", "", "partial", r#"{"type":"ECOSYSTEM","events":[{"introduced":"0.62"},{"fixed":"0.65"}]}"#, ""),
        ]);
        let found = |purl: &str| ids(&a.of(purl)).join(",");
        assert_eq!(found("pkg:cargo/time@0.1.43"), "RUSTSEC-1");
        assert_eq!(found("pkg:cargo/time@0.2.23-alpha.1"), "RUSTSEC-1", "a pre-release of the fix is before it");
        assert_eq!(found("pkg:cargo/time@0.2.23"), "");
        assert_eq!(found("pkg:cargo/time@0.3.0"), "");
        assert_eq!(found("pkg:cargo/hyper@0.13.9"), "");
        assert_eq!(found("pkg:cargo/hyper@0.14.9"), "RUSTSEC-2");
        assert_eq!(found("pkg:cargo/hyper@0.14.10"), "");
        assert_eq!(found("pkg:cargo/hyper@1.1.0"), "RUSTSEC-2", "last_affected is affected");
        assert_eq!(found("pkg:cargo/hyper@1.1.1"), "");
        assert_eq!(found("pkg:cargo/listed@0.3.1"), "RUSTSEC-3");
        assert_eq!(found("pkg:cargo/listed@0.3.2"), "");
        assert_eq!(found("pkg:cargo/partial@0.64.9"), "RUSTSEC-4");
        assert_eq!(found("pkg:cargo/partial@0.65.0"), "");
        assert_eq!(found("pkg:cargo/wasi@0.14.2+wasi-0.2.4"), "");
        assert_eq!(found("pkg:npm/time@0.1.43"), "", "another ecosystem's time");
        assert_eq!(version("0.14.2+wasi-0.2.4"), version("0.14.2"), "build metadata doesn't count");
        assert_eq!(version("1.72"), version("1.72.0"));
        assert_eq!(a.of("pkg:cargo/time@0.1.43")[0].fixed, ["0.2.23"]);
        assert_eq!(a.of("pkg:cargo/hyper@1.0.5")[0].fixed, Vec::<String>::new(), "no fix after 1.0.5 yet");
    }

    #[test]
    fn the_fixes_told_are_the_one_cargo_update_reaches_and_the_newest() {
        let backported = r#"{"type":"SEMVER","events":[{"introduced":"0"},{"fixed":"0.9.5"},{"introduced":"1.0.0"},{"fixed":"1.0.3"}]}"#;
        // RustSec's time advisory: 0.2.0 to 0.2.6 were never affected.
        let gaps = (1..=6).map(|n| format!(r#",{{"introduced":"0.2.{n}-0"}},{{"fixed":"0.2.{n}"}}"#)).collect::<String>();
        let time =
            format!(r#"{{"type":"SEMVER","events":[{{"introduced":"0.0.0-0"}},{{"fixed":"0.2.0"}}{gaps},{{"introduced":"0.2.7-0"}},{{"fixed":"0.2.23"}}]}}"#);
        let a = advisories(&[&osv("RUSTSEC-1", "", "openssl", backported, ""), &osv("RUSTSEC-2", "", "time", &time, "")]);
        assert_eq!(a.of("pkg:cargo/openssl@0.9.2")[0].fixed, ["0.9.5", "1.0.3"]);
        assert_eq!(a.of("pkg:cargo/openssl@1.0.1")[0].fixed, ["1.0.3"]);
        assert_eq!(a.of("pkg:cargo/time@0.1.43")[0].fixed, ["0.2.23"]);
        assert_eq!(a.of("pkg:cargo/time@0.2.10")[0].fixed, ["0.2.23"]);
        assert_eq!(a.of("pkg:cargo/time@0.2.3"), [], "0.2.3 is not affected");
    }

    #[test]
    fn one_advisory_in_several_databases_is_told_once_and_withdrawn_ones_not_at_all() {
        let all = r#"{"type":"SEMVER","events":[{"introduced":"0"}]}"#;
        let a = advisories(&[
            &osv("GHSA-aaaa", r#""RUSTSEC-2022-0042""#, "rustdecimal", all, ""),
            &osv("RUSTSEC-2022-0042", r#""GHSA-aaaa""#, "rustdecimal", all, ""),
            &osv("MAL-2022-1", r#""RUSTSEC-2022-0042""#, "rustdecimal", all, ""),
            &osv("GHSA-bbbb", "", "rustdecimal", all, ""),
            &osv("RUSTSEC-2024-0001", "", "rustdecimal", all, "").replace(r#"{"id""#, r#"{"withdrawn":"2024-02-01T00:00:00Z","id""#),
        ]);
        let found = a.of("pkg:cargo/rustdecimal@1.23.1");
        assert_eq!(ids(&found), ["MAL-2022-1", "GHSA-bbbb"]);
        assert_eq!(found[0].kind, Kind::Malicious);
        assert_eq!(found[1].url, "https://osv.dev/vulnerability/GHSA-bbbb");
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn informational_advisories_are_warnings() {
        let all = r#"{"type":"SEMVER","events":[{"introduced":"0"}]}"#;
        let a = advisories(&[&osv("RUSTSEC-2024-0384", "", "instant", all, r#","database_specific":{"informational":"unmaintained"}"#)]);
        let found = a.of("pkg:cargo/instant@0.1.13");
        assert_eq!(found[0].kind, Kind::Unmaintained);
        assert!(found[0].kind.is_warning() && !Kind::Vulnerability.is_warning() && !Kind::Malicious.is_warning());
    }

    #[test]
    fn the_zip_is_read_and_what_does_not_read_is_counted() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let good = osv("RUSTSEC-1", "", "time", r#"{"type":"SEMVER","events":[{"introduced":"0"},{"fixed":"0.2.23"}]}"#, "");
        for (name, text) in [
            ("RUSTSEC-1.json", good.as_str()),
            ("broken.json", "{"),
            ("GHSA-npm.json", r#"{"id":"GHSA-x","affected":[{"package":{"ecosystem":"npm","name":"time"}}]}"#),
        ] {
            zip.start_file(name, options).unwrap();
            std::io::Write::write_all(&mut zip, text.as_bytes()).unwrap();
        }
        let a = Advisories::from_zip(&zip.finish().unwrap().into_inner()).unwrap();
        assert_eq!((a.len(), a.unreadable), (1, 1), "npm is no ecosystem matched");
        assert_eq!(ids(&a.of("pkg:cargo/time@0.1.0")), ["RUSTSEC-1"]);
        assert!(Advisories::from_zip(b"not a zip").is_err());
    }

    #[test]
    fn appended_advisories_keep_what_they_say() {
        let all = r#"{"type":"SEMVER","events":[{"introduced":"0"},{"fixed":"2.0.0"}]}"#;
        let mut a = advisories(&[&osv("RUSTSEC-1", "", "time", all, ""), &osv("RUSTSEC-2", "", "hyper", all, "")]);
        let mut b = advisories(&[&osv("GHSA-3", "", "ring", all, ""), &osv("RUSTSEC-4", "", "time", all, "")]);
        b.unreadable = 2;
        a.append(b);
        assert_eq!((a.len(), a.unreadable), (4, 2));
        assert_eq!(ids(&a.of("pkg:cargo/ring@1.0.0")), ["GHSA-3"]);
        assert_eq!(ids(&a.of("pkg:cargo/time@1.0.0")), ["RUSTSEC-1", "RUSTSEC-4"]);
        assert_eq!(a.of("pkg:cargo/ring@1.0.0")[0].summary, "GHSA-3 in ring");
    }
}
