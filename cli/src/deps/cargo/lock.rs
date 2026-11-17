//! Cargo.lock, as cargo's encode.rs decodes it.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, bail};
use serde::Deserialize;

use crate::deps::files::Files;

/// The packages in Cargo.lock, and what each depends on.
pub struct Lock {
    pub packages: Vec<Locked>,
    /// Each package's dependencies, as indexes into `packages`.
    pub deps: Vec<Vec<usize>>,
    by_name: HashMap<String, Vec<usize>>,
}

#[derive(Deserialize)]
struct LockFile {
    #[serde(default)]
    package: Vec<Locked>,
    /// Lock files before 2017 keep the root package apart.
    root: Option<Locked>,
}

#[derive(Deserialize)]
pub struct Locked {
    pub name: String,
    pub version: String,
    /// None for a path crate.
    pub source: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    /// What [replace] put in its place.
    replace: Option<String>,
}

impl Lock {
    /// Cargo.lock at `path`. [replace] is refused: it swaps crates in that
    /// the manifests don't name.
    pub fn read(files: &dyn Files, path: &Path) -> anyhow::Result<Lock> {
        let text = files.read(path).with_context(|| format!("could not read {}; build the app first", path.display()))?;
        let lock = Lock::parse(&text).with_context(|| format!("{} is not a Cargo.lock", path.display()))?;
        if let Some(p) = lock.packages.iter().find(|p| p.replace.is_some()) {
            bail!("{} replaces {} {} with [replace], which appos doesn't read", path.display(), p.name, p.version);
        }
        Ok(lock)
    }

    fn parse(text: &str) -> Result<Lock, toml::de::Error> {
        let file: LockFile = toml::from_str(text)?;
        let packages: Vec<Locked> = file.package.into_iter().chain(file.root).collect();
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, p) in packages.iter().enumerate() {
            by_name.entry(p.name.clone()).or_default().push(i);
        }
        let mut lock = Lock { packages, deps: vec![], by_name };
        let deps = lock.packages.iter().map(|p| p.dependencies.iter().filter_map(|d| lock.lookup(d)).collect()).collect();
        lock.deps = deps;
        Ok(lock)
    }

    /// The path package of this name: the app, a member, a patch.
    pub fn path_package(&self, name: &str) -> Option<usize> {
        self.by_name.get(name)?.iter().copied().find(|&i| self.packages[i].source.is_none())
    }

    /// The package a `dependencies` entry means: "name", "name version" or
    /// "name version (source)", looked up as cargo's encode.rs does. None
    /// where cargo ignores the entry.
    fn lookup(&self, entry: &str) -> Option<usize> {
        let mut parts = entry.splitn(3, ' ');
        let name = parts.next()?;
        let version = parts.next();
        let source = match parts.next() {
            Some(s) => Some(s.strip_prefix('(')?.strip_suffix(')')?),
            None => None,
        };
        let p = &self.packages;
        let named = self.by_name.get(name)?;
        let versioned: Vec<usize> = match version {
            Some(v) => named.iter().copied().filter(|&i| p[i].version == v).collect(),
            // Without a version there must be one.
            None if named.iter().all(|&i| p[i].version == p[named[0]].version) => named.clone(),
            None => return None,
        };
        match source {
            Some(s) => versioned.into_iter().find(|&i| p[i].source.as_deref() == Some(s)),
            // Path crates list no source, so one of them; else the only one.
            None => match (versioned.iter().filter(|&&i| p[i].source.is_none()).collect::<Vec<_>>().as_slice(), versioned.as_slice()) {
                ([one], _) => Some(**one),
                ([], [one]) => Some(*one),
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::cargo::CRATES_IO;

    #[test]
    fn cargo_lock_entries_are_looked_up_as_cargo_does() {
        let io = CRATES_IO[0];
        let git = "git+https://github.com/o/c#0123456789abcdef";
        let text = format!(
            r#"
[root]
name = "app"
version = "0.1.0"
dependencies = ["a"]

[[package]]
name = "a"
version = "1.0.0"
source = "{io}"

[[package]]
name = "a"
version = "1.0.0"

[[package]]
name = "b"
version = "1.0.0"
source = "{io}"

[[package]]
name = "b"
version = "2.0.0"
source = "{io}"

[[package]]
name = "c"
version = "1.0.0"
source = "{io}"

[[package]]
name = "c"
version = "1.0.0"
source = "{git}"

[[package]]
name = "d"
version = "1.0.0"
source = "{git}"
"#
        );
        let lock = Lock::parse(&text).unwrap();
        assert_eq!(lock.path_package("app"), Some(lock.packages.len() - 1), "lock files before 2017 keep the root apart");
        assert_eq!(lock.deps[lock.packages.len() - 1], [1], "the root's a: the path crate");
        let find =
            |entry: &str| lock.lookup(entry).map(|i| (lock.packages[i].name.as_str(), lock.packages[i].version.as_str(), lock.packages[i].source.as_deref()));
        assert_eq!(find(&format!("a 1.0.0 ({io})")), Some(("a", "1.0.0", Some(io))));
        assert_eq!(find("a 1.0.0"), Some(("a", "1.0.0", None)), "without a source, the path crate");
        assert_eq!(find("a"), Some(("a", "1.0.0", None)));
        assert_eq!(find("b"), None, "two versions: cargo ignores the entry");
        assert_eq!(find("b 2.0.0"), Some(("b", "2.0.0", Some(io))));
        assert_eq!(find("c 1.0.0"), None, "two sources, no path crate: cargo ignores the entry");
        assert_eq!(find(&format!("c 1.0.0 ({git})")), Some(("c", "1.0.0", Some(git))));
        assert_eq!(find("d"), Some(("d", "1.0.0", Some(git))), "the only one");
        assert_eq!(find(&format!("a 1.0.0 {io}")), None, "the source goes in parentheses");
        assert_eq!(find("e"), None);
    }
}
