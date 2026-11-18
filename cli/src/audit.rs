//! `appos audit`: which live apps use a package with a known
//! vulnerability, from the deps.json `appos pack` put in their app.zip
//! (deps/) and the advisories OSV publishes for each ecosystem.

use std::collections::HashSet;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use appos_shell::api::LivePackages;
use appos_shell::deps::{self, Advisories, ECOSYSTEMS, Ecosystem, Finding, Purl};
use appos_shell::{terminal_eprintln as eprintln, terminal_println as println};
use serde::Serialize;

use crate::platform;

/// How long the advisories are used before the changes since are fetched.
const FRESH: Duration = Duration::from_secs(60 * 60);
/// How long a download of all advisories is kept up to date with the
/// changes before it is downloaded again: npm has about 1,200 changes a
/// week.
const FULL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// The most changes fetched one by one; with more, all advisories are
/// downloaded again.
const MAX_CHANGES: usize = 1000;
/// The largest advisories zip of an ecosystem, or list of changes; npm's
/// zip is 211 MB, crates.io's 4 MB (2026).
const MAX_DOWNLOAD: u64 = 1024 * 1024 * 1024;

/// What `--json` prints for each live app.
#[derive(Serialize)]
struct Report<'a> {
    id: &'a str,
    name: &'a str,
    version: &'a str,
    label: &'a str,
    owner: &'a str,
    group: &'a str,
    /// How many packages were checked; null without deps.json.
    packages: Option<usize>,
    findings: Vec<Finding>,
}

pub fn audit(json: bool) -> anyhow::Result<()> {
    let p = crate::commands::current()?;
    let live = platform::packages(&p)?;
    let used = used(&live);
    let (advisories, age) = advisories(&used)?;
    let reports: Vec<Report> = live.iter().map(|app| report(app, &advisories)).collect();
    let vulnerable = reports.iter().filter(|r| r.findings.iter().any(|f| !f.kind.is_warning())).count();
    let ecosystems: Vec<&str> = used.iter().map(|e| e.osv).collect();
    if json {
        let all = serde_json::json!({ "platform": p.name, "ecosystems": ecosystems, "advisories": advisories.len(), "apps": reports });
        println!("{}", serde_json::to_string_pretty(&all)?);
    } else {
        print(&p.name, &reports, &ecosystems, &advisories, age, vulnerable);
    }
    match vulnerable {
        0 => Ok(()),
        1 => anyhow::bail!("1 live app uses a package with a known vulnerability"),
        n => anyhow::bail!("{n} live apps use a package with a known vulnerability"),
    }
}

/// The ecosystems of the live apps' packages: whose advisories are needed.
fn used(live: &[LivePackages]) -> Vec<&'static Ecosystem> {
    let kinds: HashSet<&str> = live.iter().flat_map(|app| app.packages.iter().flatten()).filter_map(|p| Purl::parse(p).ok()).map(|p| p.kind).collect();
    ECOSYSTEMS.iter().filter(|e| kinds.contains(e.purl)).collect()
}

fn report<'a>(app: &'a LivePackages, advisories: &Advisories) -> Report<'a> {
    let mut findings: Vec<Finding> = app.packages.iter().flatten().flat_map(|p| advisories.of(p)).collect();
    findings.sort_by(|a, b| (a.kind.is_warning(), &a.package, &a.id).cmp(&(b.kind.is_warning(), &b.package, &b.id)));
    Report {
        id: &app.id,
        name: &app.name,
        version: &app.version,
        label: &app.label,
        owner: &app.owner,
        group: &app.group,
        packages: app.packages.as_ref().map(Vec::len),
        findings,
    }
}

fn print(platform: &str, reports: &[Report], ecosystems: &[&str], advisories: &Advisories, age: Duration, vulnerable: usize) {
    let apps = match reports.len() {
        1 => "1 live app".to_string(),
        n => format!("{n} live apps"),
    };
    let checked = match ecosystems {
        [] => String::new(),
        names => format!(", checked against {} advisories for {} from osv.dev{}", advisories.len(), names.join(" and "), ago(age)),
    };
    println!("{platform}: {apps}{checked}");
    if advisories.unreadable > 0 {
        eprintln!("warning: {} advisories could not be read and were left out", advisories.unreadable);
    }
    for r in reports.iter().filter(|r| !r.findings.is_empty()) {
        println!();
        println!("{} {}, {}, {}", r.id, r.version, r.owner, r.group);
        for f in &r.findings {
            let (name, version) = Purl::parse(&f.package).map_or((f.package.as_str().into(), ""), |p| (p.package(), p.version));
            println!("  {name} {version}  {}  {}", f.kind, f.id);
            println!("    {}", f.summary);
            let fix = match f.fixed.as_slice() {
                _ if f.kind == deps::Kind::Malicious => "Remove it.".to_string(),
                [] => "No fixed version.".to_string(),
                fixed => format!("Fixed in {}.", fixed.join(" and ")),
            };
            println!("    {fix} {}", f.url);
        }
    }
    let clean: Vec<String> = reports.iter().filter(|r| r.findings.is_empty()).filter_map(|r| Some(format!("{} ({} packages)", r.id, r.packages?))).collect();
    let unchecked: Vec<&str> = reports.iter().filter(|r| r.packages.is_none()).map(|r| r.id).collect();
    println!();
    if !clean.is_empty() {
        println!("Nothing known against {}.", clean.join(", "));
    }
    if !unchecked.is_empty() {
        println!("Not checked, no deps.json (appos lists no packages for its language yet, or packed by an older appos): {}.", unchecked.join(", "));
    }
    // With a vulnerability, the error says so; with nothing checked, there
    // is nothing to say.
    let warned = reports.iter().filter(|r| !r.findings.is_empty()).count();
    match (vulnerable, warned) {
        _ if unchecked.len() == reports.len() => {}
        (0, 0) => println!("No live app uses a package with a known advisory."),
        (0, 1) => println!("No live app uses a package with a known vulnerability; 1 has warnings (above)."),
        (0, n) => println!("No live app uses a package with a known vulnerability; {n} have warnings (above)."),
        _ => {}
    }
}

fn ago(age: Duration) -> String {
    match age.as_secs() / 60 {
        0 => String::new(),
        m if m < 120 => format!(", downloaded {m} min ago"),
        m => format!(", downloaded {} hours ago", m / 60),
    }
}

/// Where an ecosystem's advisories are kept, in $XDG_CACHE_HOME/appos or
/// ~/.cache/appos: all of them as OSV's zip, the ones that changed since
/// as their own files, and when the newest change fetched was made.
struct Cache {
    zip: PathBuf,
    changed: PathBuf,
    since: PathBuf,
}

impl Cache {
    fn of(e: &Ecosystem) -> Cache {
        let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            home.join(".cache")
        });
        Cache::in_dir(&base.join("appos"), e)
    }

    fn in_dir(dir: &Path, e: &Ecosystem) -> Cache {
        Cache {
            zip: dir.join(format!("osv-{}.zip", e.osv)),
            changed: dir.join(format!("osv-{}.changed", e.osv)),
            since: dir.join(format!("osv-{}.since", e.osv)),
        }
    }

    /// The advisories kept: the zip, with the changed ones in place of
    /// theirs.
    fn load(&self) -> anyhow::Result<Advisories> {
        let mut changed = vec![];
        for entry in std::fs::read_dir(&self.changed).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".json") {
                changed.push((id.to_string(), read_at_most(&entry.path(), deps::MAX_ADVISORY as u64 + 1)?));
            }
        }
        let ids: HashSet<String> = changed.iter().map(|(id, _)| id.clone()).collect();
        let zip = read_at_most(&self.zip, MAX_DOWNLOAD)?;
        if zip.len() as u64 > MAX_DOWNLOAD {
            anyhow::bail!("{} is larger than {} MB", self.zip.display(), MAX_DOWNLOAD >> 20);
        }
        let mut advisories = Advisories::from_zip_except(&zip, &ids)?;
        for (_, json) in changed {
            if json.len() > deps::MAX_ADVISORY || advisories.add(&json).is_err() {
                advisories.unreadable += 1;
            }
        }
        Ok(advisories)
    }

    /// When the newest change fetched was made.
    fn since(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let text = std::fs::read_to_string(&self.since).ok()?;
        chrono::DateTime::parse_from_rfc3339(text.trim()).ok().map(|t| t.to_utc())
    }

    /// Keeps a download of all advisories, made `newest`, and drops the
    /// changes kept from the last.
    fn keep_all(&self, zip: &[u8], newest: Option<chrono::DateTime<chrono::Utc>>) -> anyhow::Result<()> {
        keep(&self.zip, zip)?;
        match std::fs::remove_dir_all(&self.changed) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        // A zip is written while advisories change: one changed in its last
        // hour may be in it as it was, so those are fetched again.
        match newest {
            Some(newest) => keep(&self.since, (newest - chrono::Duration::hours(1)).to_rfc3339().as_bytes()),
            None => match std::fs::remove_file(&self.since) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            },
        }
    }
}

/// A file's first `limit` bytes and one more, so a longer file shows.
fn read_at_most(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(std::fs::File::open(path)?, limit + 1), &mut bytes)?;
    Ok(bytes)
}

/// How long ago a file was written.
fn age(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    Some(SystemTime::now().duration_since(modified).unwrap_or_default())
}

/// OSV's advisories for these ecosystems, and how old the oldest are.
fn advisories(ecosystems: &[&Ecosystem]) -> anyhow::Result<(Advisories, Duration)> {
    let mut all = Advisories::default();
    let mut oldest = Duration::ZERO;
    for e in ecosystems {
        let (advisories, age) = ecosystem_advisories(e)?;
        all.append(advisories);
        oldest = oldest.max(age);
    }
    Ok((all, oldest))
}

/// OSV's advisories for an ecosystem and how old they are: all of them
/// downloaded once a week, the changes since fetched at most once an hour;
/// without a connection, those kept, with a warning.
fn ecosystem_advisories(e: &Ecosystem) -> anyhow::Result<(Advisories, Duration)> {
    let cache = Cache::of(e);
    let checked = age(&cache.since);
    if let Some(age) = checked.filter(|a| *a < FRESH)
        && let Ok(advisories) = cache.load()
    {
        return Ok((advisories, age));
    }
    let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(300))).build());
    let fetched = match age(&cache.zip) {
        Some(age) if age < FULL => fetch_changes(&agent, e, &cache).and_then(|changed| match changed {
            Some(advisories) => Ok(advisories),
            None => download(&agent, e, &cache),
        }),
        _ => download(&agent, e, &cache),
    };
    match fetched {
        Ok(advisories) => Ok((advisories, Duration::ZERO)),
        Err(err) => match checked.map(|age| (age, cache.load())) {
            Some((age, Ok(advisories))) => {
                eprintln!("warning: could not download the advisories for {} ({err:#}); using those{}", e.osv, ago(age));
                Ok((advisories, age))
            }
            _ => Err(err),
        },
    }
}

/// Fetches the advisories that changed since the last fetch, one by one,
/// and loads them all; None when that's too many to fetch one by one, or
/// nothing tells since when.
fn fetch_changes(agent: &ureq::Agent, e: &Ecosystem, cache: &Cache) -> anyhow::Result<Option<Advisories>> {
    let Some(since) = cache.since() else { return Ok(None) };
    let url = e.changes();
    let mut resp = agent.get(&url).call().with_context(|| format!("could not download {url}"))?;
    let lines = std::io::BufReader::new(resp.body_mut().with_config().limit(MAX_DOWNLOAD).reader()).lines();
    let Some(changes) = changed_since(lines, since).with_context(|| format!("could not download {url}"))? else { return Ok(None) };
    if !changes.odd.is_empty() {
        eprintln!("warning: {url} names advisories appos doesn't fetch, which are left as they were: {}", changes.odd.join(", "));
    }
    for id in changes.ids {
        let json = get(agent, &e.advisory(&id), deps::MAX_ADVISORY as u64)?;
        keep(&cache.changed.join(format!("{id}.json")), &json)?;
    }
    keep(&cache.since, changes.newest.to_rfc3339().as_bytes())?;
    cache.load().map(Some)
}

/// The advisories that changed since a time.
#[derive(Debug, PartialEq)]
struct Changes {
    /// When the newest change was made.
    newest: chrono::DateTime<chrono::Utc>,
    ids: Vec<String>,
    /// Lines that name no file an id can: not fetched.
    odd: Vec<String>,
}

/// Of OSV's list of changes (`2026-11-17T17:30:17.9Z,GHSA-…`, newest
/// first), the advisories that changed after `since`; None with more than
/// MAX_CHANGES. Times are compared as times: the list can drop a
/// fraction's trailing zero.
fn changed_since(lines: impl Iterator<Item = std::io::Result<String>>, since: chrono::DateTime<chrono::Utc>) -> std::io::Result<Option<Changes>> {
    let mut changes = Changes { newest: since, ids: vec![], odd: vec![] };
    for line in lines {
        let line = line?;
        let Some((at, id)) = line.trim().split_once(',') else { continue };
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(at).map(|t| t.to_utc()) else { continue };
        if at <= since {
            break;
        }
        if changes.ids.len() + changes.odd.len() == MAX_CHANGES {
            return Ok(None);
        }
        changes.newest = changes.newest.max(at);
        // An id names a file: only what OSV's ids are made of.
        if !id.is_empty() && id.len() <= 128 && !id.starts_with('.') && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
            changes.ids.push(id.to_string());
        } else {
            changes.odd.push(id.chars().take(128).collect());
        }
    }
    Ok(Some(changes))
}

/// Downloads all advisories and keeps them.
fn download(agent: &ureq::Agent, e: &Ecosystem, cache: &Cache) -> anyhow::Result<Advisories> {
    let url = e.advisories();
    let bytes = get(agent, &url, MAX_DOWNLOAD)?;
    let advisories = Advisories::from_zip(&bytes)?;
    if advisories.is_empty() {
        anyhow::bail!("{url} has no advisories for {}", e.osv);
    }
    if let Err(err) = cache.keep_all(&bytes, advisories.newest) {
        eprintln!("warning: could not keep the advisories in {}: {err:#}", cache.zip.display());
    }
    Ok(advisories)
}

fn get(agent: &ureq::Agent, url: &str, limit: u64) -> anyhow::Result<Vec<u8>> {
    let mut resp = agent.get(url).call().with_context(|| format!("could not download {url}"))?;
    resp.body_mut().with_config().limit(limit).read_to_vec().with_context(|| format!("could not download {url}"))
}

/// Writes a file whole or not at all.
fn keep(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().context("no cache directory")?;
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisories_are_needed_for_the_ecosystems_the_live_apps_use() {
        let app = |packages: Option<&[&str]>| LivePackages {
            id: "nps".into(),
            name: "NPS".into(),
            version: "0.3.0".into(),
            label: "0.3.0-ab".into(),
            owner: "Jane Doe <jane@example.com>".into(),
            group: "Marketing".into(),
            packages: packages.map(|p| p.iter().map(|p| p.to_string()).collect()),
        };
        assert_eq!(used(&[app(None), app(Some(&[]))]), Vec::<&Ecosystem>::new(), "no deps.json, nothing to download");
        let rust = app(Some(&["pkg:cargo/serde@1.0.228", "pkg:cargo/itoa@1.0.15"]));
        assert_eq!(used(&[rust, app(Some(&["pkg:golang/x@1.0.0"]))]).iter().map(|e| e.osv).collect::<Vec<_>>(), ["crates.io"], "golang isn't matched");
    }

    fn time(text: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(text).unwrap().to_utc()
    }

    #[test]
    fn the_changes_since_the_last_fetch_are_those_newer_up_to_a_limit() {
        let lines = |text: &str| text.lines().map(|l| Ok(l.to_string())).collect::<Vec<_>>().into_iter();
        let list = "2026-11-17T17:30:17.901276722Z,MAL-2026-20076\n\
                    2026-11-13T22:30:20.666251571Z,GHSA-aaaa-bbbb-cccc\n\
                    2026-11-13T22:30:20.66625157Z,MAL-2026-19828\n\
                    no time,GHSA-x\n\
                    2026-11-12T10:00:00Z,../escape\n\
                    2026-11-12T09:00:00Z,.hidden\n\
                    2026-11-12T08:00:00Z,a/b\n\
                    2026-11-12T07:00:00Z,\n\
                    2026-11-01T00:00:00Z,GHSA-old\n\
                    2026-11-17T18:00:00Z,GHSA-after-the-mark\n";
        let changes = changed_since(lines(list), time("2026-11-02T00:00:00Z")).unwrap().unwrap();
        assert_eq!(changes.newest, time("2026-11-17T17:30:17.901276722Z"));
        assert_eq!(changes.ids, ["MAL-2026-20076", "GHSA-aaaa-bbbb-cccc", "MAL-2026-19828"], "a dropped zero is no older");
        assert_eq!(changes.odd, ["../escape", ".hidden", "a/b", ""]);

        let none = changed_since(lines(list), time("2026-11-17T17:30:17.901276722Z")).unwrap().unwrap();
        assert_eq!((none.newest, none.ids.len(), none.odd.len()), (time("2026-11-17T17:30:17.901276722Z"), 0, 0), "the mark itself is no change");

        let many: String = (0..=MAX_CHANGES).map(|i| format!("2026-11-17T17:00:00Z,GHSA-{i}\n")).collect();
        assert_eq!(changed_since(lines(&many), time("2026-11-01T00:00:00Z")).unwrap(), None, "too many to fetch one by one");
        let enough: String = many.lines().take(MAX_CHANGES).map(|l| format!("{l}\n")).collect();
        assert_eq!(changed_since(lines(&enough), time("2026-11-01T00:00:00Z")).unwrap().unwrap().ids.len(), MAX_CHANGES);
        let broken = vec![Ok("2026-11-17T17:00:00Z,GHSA-1".to_string()), Err(std::io::Error::other("cut"))].into_iter();
        assert!(changed_since(broken, time("2026-11-01T00:00:00Z")).is_err(), "a cut download is no list");
    }

    #[test]
    fn the_cache_holds_the_zip_with_the_changed_advisories_in_place_of_theirs() {
        let dir = tempfile::tempdir().unwrap();
        let npm = deps::ECOSYSTEMS.iter().find(|e| e.osv == "npm").unwrap();
        let cache = Cache::in_dir(&dir.path().join("appos"), npm);
        assert!(cache.load().is_err(), "nothing kept");
        assert_eq!(cache.since(), None);
        let advisory = |id: &str, name: &str, fixed: &str, modified: &str| {
            format!(
                r#"{{"id":"{id}","modified":"{modified}","summary":"{id}","affected":[{{"package":{{"ecosystem":"npm","name":"{name}"}},"ranges":[{{"type":"SEMVER","events":[{{"introduced":"0"}},{{"fixed":"{fixed}"}}]}}]}}]}}"#
            )
        };
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        for (id, json) in
            [("GHSA-1", advisory("GHSA-1", "ms", "2.0.0", "2026-11-10T00:00:00Z")), ("GHSA-2", advisory("GHSA-2", "@s/b", "1.0.0", "2026-11-16T12:00:00Z"))]
        {
            zip.start_file(format!("{id}.json"), zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut zip, json.as_bytes()).unwrap();
        }
        let zip = zip.finish().unwrap().into_inner();
        let all = Advisories::from_zip(&zip).unwrap();
        assert_eq!(all.newest, Some(time("2026-11-16T12:00:00Z")));
        std::fs::create_dir_all(&cache.changed).unwrap();
        std::fs::write(cache.changed.join("stale.json"), "from the last download").unwrap();
        cache.keep_all(&zip, all.newest).unwrap();
        assert!(!cache.changed.exists(), "a new download has every change");
        assert_eq!(cache.since(), Some(time("2026-11-16T11:00:00Z")), "an hour before its newest, which may have been written as it changed");
        let ids = |a: &Advisories, purl: &str| a.of(purl).into_iter().map(|f| f.id).collect::<Vec<_>>();
        let loaded = cache.load().unwrap();
        assert_eq!((ids(&loaded, "pkg:npm/ms@1.0.0"), ids(&loaded, "pkg:npm/%40s/b@0.9.0")), (vec!["GHSA-1".to_string()], vec!["GHSA-2".to_string()]));

        keep(&cache.changed.join("GHSA-1.json"), advisory("GHSA-1", "ms", "0.5.0", "2026-11-17T00:00:00Z").as_bytes()).unwrap();
        keep(&cache.changed.join("GHSA-3.json"), advisory("GHSA-3", "ms", "3.0.0", "2026-11-17T00:00:00Z").as_bytes()).unwrap();
        keep(&cache.changed.join("GHSA-4.json"), b"not json").unwrap();
        keep(&cache.changed.join("GHSA-5.json"), &vec![b' '; deps::MAX_ADVISORY + 1]).unwrap();
        keep(&cache.changed.join("notes.txt"), b"ignored").unwrap();
        let loaded = cache.load().unwrap();
        assert_eq!(ids(&loaded, "pkg:npm/ms@1.0.0"), ["GHSA-3"], "the changed GHSA-1 is fixed in 0.5.0, and GHSA-3 is new");
        assert_eq!(ids(&loaded, "pkg:npm/%40s/b@0.9.0"), ["GHSA-2"], "unchanged");
        assert_eq!(loaded.unreadable, 2);

        cache.keep_all(&zip, None).unwrap();
        assert_eq!(cache.since(), None, "no newest, no mark: the next fetch downloads it all");

        // npm's zip is over 200 MB, more than an app.zip may be.
        std::fs::write(&cache.zip, vec![0; 65 << 20]).unwrap();
        let err = format!("{:#}", cache.load().err().unwrap());
        assert!(err.contains("not a zip file"), "{err}");
    }

    #[test]
    fn a_report_puts_vulnerabilities_before_warnings_and_tells_apps_without_deps_json() {
        let app = |packages: Option<Vec<String>>| LivePackages {
            id: "nps".into(),
            name: "NPS".into(),
            version: "0.3.0".into(),
            label: "0.3.0-ab".into(),
            owner: "Jane Doe <jane@example.com>".into(),
            group: "Marketing".into(),
            packages,
        };
        let advisories = Advisories::default();
        let unchecked = app(None);
        assert_eq!(report(&unchecked, &advisories).packages, None);
        let checked = app(Some(vec!["pkg:cargo/instant@0.1.13".into(), "pkg:cargo/time@0.1.43".into()]));
        assert_eq!(report(&checked, &advisories).findings, []);
        let mut advisories = Advisories::default();
        let osv = |id: &str, krate: &str, events: &str, extra: &str| {
            format!(
                r#"{{"id":"{id}","summary":"s","affected":[{{"package":{{"ecosystem":"crates.io","name":"{krate}"}},"ranges":[{{"type":"SEMVER","events":{events}}}]{extra}}}]}}"#
            )
        };
        advisories
            .add(osv("RUSTSEC-2024-0384", "instant", r#"[{"introduced":"0"}]"#, r#","database_specific":{"informational":"unmaintained"}"#).as_bytes())
            .unwrap();
        advisories.add(osv("RUSTSEC-2020-0071", "time", r#"[{"introduced":"0"},{"fixed":"0.2.23"}]"#, "").as_bytes()).unwrap();
        let r = report(&checked, &advisories);
        assert_eq!(r.packages, Some(2));
        let found: Vec<_> = r.findings.iter().map(|f| (f.id.as_str(), f.kind, f.fixed.clone())).collect();
        assert_eq!(
            found,
            [("RUSTSEC-2020-0071", deps::Kind::Vulnerability, vec!["0.2.23".to_string()]), ("RUSTSEC-2024-0384", deps::Kind::Unmaintained, vec![])]
        );
    }
}
