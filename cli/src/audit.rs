//! `appos audit`: which live apps use a package with a known
//! vulnerability, from the deps.json `appos pack` put in their app.zip
//! (deps/) and the advisories OSV publishes for each ecosystem.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use appos_shell::api::LivePackages;
use appos_shell::deps::{self, Advisories, ECOSYSTEMS, Ecosystem, Finding, Purl};
use appos_shell::{terminal_eprintln as eprintln, terminal_println as println};
use serde::Serialize;

use crate::platform;

/// How long a download of the advisories is used before the next.
const FRESH: Duration = Duration::from_secs(60 * 60);
/// The largest advisories zip of an ecosystem; crates.io's is about 4 MB.
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;

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
            let (name, version) = Purl::parse(&f.package).map_or((f.package.as_str(), ""), |p| (p.name, p.version));
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

/// Where downloads of an ecosystem's advisories are kept:
/// $XDG_CACHE_HOME/appos, or ~/.cache/appos.
fn cache(e: &Ecosystem) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        home.join(".cache")
    });
    base.join(format!("appos/osv-{}.zip", e.osv))
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

/// OSV's advisories for an ecosystem and how old they are: downloaded at
/// most once an hour; without a connection, the last download, with a
/// warning.
fn ecosystem_advisories(e: &Ecosystem) -> anyhow::Result<(Advisories, Duration)> {
    let path = cache(e);
    let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().map(|t| SystemTime::now().duration_since(t).unwrap_or_default());
    if let Some(age) = age.filter(|a| *a < FRESH)
        && let Ok(advisories) = cached(&path)
    {
        return Ok((advisories, age));
    }
    match download(e) {
        Ok((advisories, bytes)) => {
            if let Err(e) = keep(&path, &bytes) {
                eprintln!("warning: could not keep the advisories in {}: {e:#}", path.display());
            }
            Ok((advisories, Duration::ZERO))
        }
        Err(err) => match age.map(|age| (age, cached(&path))) {
            Some((age, Ok(advisories))) => {
                eprintln!("warning: could not download the advisories for {} ({err:#}); using those{}", e.osv, ago(age));
                Ok((advisories, age))
            }
            _ => Err(err),
        },
    }
}

fn cached(path: &Path) -> anyhow::Result<Advisories> {
    Advisories::from_zip(&appos_shell::bundle::read_file(path)?)
}

fn download(e: &Ecosystem) -> anyhow::Result<(Advisories, Vec<u8>)> {
    let url = e.advisories();
    let agent = ureq::Agent::new_with_config(ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(120))).build());
    let mut resp = agent.get(&url).call().with_context(|| format!("could not download {url}"))?;
    let bytes = resp.body_mut().with_config().limit(MAX_DOWNLOAD).read_to_vec().with_context(|| format!("could not download {url}"))?;
    let advisories = Advisories::from_zip(&bytes)?;
    if advisories.is_empty() {
        anyhow::bail!("{url} has no advisories for {}", e.osv);
    }
    Ok((advisories, bytes))
}

/// Writes the download whole or not at all.
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
