//! What an app is built with, and what is known to be wrong with it.
//!
//! `appos pack` puts deps.json in app.zip: the packages compiled into the
//! app, as package URLs like pkg:cargo/serde@1.0.228. The shell keeps the
//! list with the version. `appos audit` matches the lists of the live apps
//! against OSV (https://osv.dev), which publishes the advisories of an
//! ecosystem as one zip: for crates.io RustSec's, GitHub's and the known
//! malicious crates.
//!
//!   mod.rs        deps.json and the package URLs in it, of any ecosystem
//!   ecosystem.rs  the ecosystems matched: package URL type, OSV's name
//!   osv.rs        the advisories, and which of them affect a package
//!
//! Matching another ecosystem, like npm for TypeScript apps, is a line in
//! ecosystem.rs; the CLI lists its packages (cli/src/deps).
//!
//! It finds accidents, not intent. An app that means harm doesn't need a
//! vulnerable package; the sandbox and the grants hold it either way.

mod ecosystem;
mod osv;

use std::collections::BTreeSet;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

pub use ecosystem::{ECOSYSTEMS, Ecosystem};
pub use osv::{Advisories, Finding, Kind};

/// The packages in app.zip.
pub const FILE: &str = "deps.json";
/// The largest deps.json.
pub const MAX: usize = 256 * 1024;
const MAX_PACKAGES: usize = 10_000;
const MAX_PURL: usize = 256;
const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
struct DepsFile {
    format: u32,
    packages: Vec<String>,
}

/// deps.json for these package URLs: sorted, each once.
pub fn to_json(packages: impl IntoIterator<Item = String>) -> String {
    let packages: BTreeSet<String> = packages.into_iter().collect();
    let file = DepsFile { format: FORMAT, packages: packages.into_iter().collect() };
    serde_json::to_string_pretty(&file).expect("a struct of strings serializes") + "\n"
}

/// The package URLs in deps.json, checked.
pub fn parse(text: &str) -> anyhow::Result<Vec<String>> {
    if text.len() > MAX {
        bail!("deps.json is {} KB, more than {} KB", text.len() / 1024, MAX / 1024);
    }
    let file: DepsFile = serde_json::from_str(text).map_err(|e| anyhow::anyhow!("deps.json is not a list of packages: {e}"))?;
    if file.format != FORMAT {
        bail!("deps.json is format {}; this shell reads format {FORMAT}", file.format);
    }
    if file.packages.len() > MAX_PACKAGES {
        bail!("deps.json lists {} packages, more than {MAX_PACKAGES}", file.packages.len());
    }
    for p in &file.packages {
        Purl::parse(p).with_context(|| format!("deps.json lists {:?}", p.chars().take(MAX_PURL).collect::<String>()))?;
    }
    Ok(file.packages)
}

/// A package URL as deps.json holds it: `pkg:<type>/<name>@<version>`, like
/// pkg:cargo/serde@1.0.228. Of the purl spec only what an advisory needs:
/// no qualifiers, no subpath.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Purl<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub version: &'a str,
}

impl<'a> Purl<'a> {
    pub fn parse(s: &'a str) -> anyhow::Result<Purl<'a>> {
        if s.len() > MAX_PURL {
            bail!("a package URL longer than {MAX_PURL} characters");
        }
        if !s.chars().all(|c| c.is_ascii_graphic()) || s.contains(['?', '#']) {
            bail!("not a package URL: only letters, digits and punctuation, without qualifiers");
        }
        let rest = s.strip_prefix("pkg:").context("not a package URL: it starts with pkg:")?;
        let (kind, rest) = rest.split_once('/').context("not a package URL: pkg:<type>/<name>@<version>")?;
        let (name, version) = rest.rsplit_once('@').context("a package URL without a version")?;
        let kind_ok =
            kind.starts_with(|c: char| c.is_ascii_lowercase()) && kind.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ".+-".contains(c));
        if !kind_ok || name.is_empty() || version.is_empty() {
            bail!("not a package URL: pkg:<type>/<name>@<version>");
        }
        Ok(Purl { kind, name, version })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deps_json_round_trips_sorted_and_refuses_what_is_not_a_package_list() {
        let text = to_json(["pkg:cargo/serde@1.0.228".to_string(), "pkg:cargo/askama@0.16.0".into(), "pkg:cargo/serde@1.0.228".into()]);
        assert_eq!(parse(&text).unwrap(), ["pkg:cargo/askama@0.16.0", "pkg:cargo/serde@1.0.228"]);
        let err = |t: &str| format!("{:#}", parse(t).unwrap_err());
        assert!(err("[]").contains("not a list of packages"));
        assert!(err(r#"{"format":2,"packages":[]}"#).contains("format 2"));
        assert!(err(r#"{"format":1,"packages":["serde@1"]}"#).contains("starts with pkg:"));
        assert!(err(r#"{"format":1,"packages":["pkg:cargo/serde"]}"#).contains("without a version"));
        assert!(err(r#"{"format":1,"packages":["pkg:cargo/se rde@1"]}"#).contains("not a package URL"));
        assert!(err(r#"{"format":1,"packages":["pkg:cargo/serde@1?x=y"]}"#).contains("qualifiers"));
        assert!(err(&format!("{{\"format\":1,\"packages\":[\"{}\"]}}", " ".repeat(MAX))).contains("more than 256 KB"));
        let p = Purl::parse("pkg:npm/%40scope/name@1.0.0").unwrap();
        assert_eq!((p.kind, p.name, p.version), ("npm", "%40scope/name", "1.0.0"));
    }
}
