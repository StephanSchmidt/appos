//! package-lock.json, version 2 or 3: every package npm installed, by the
//! path it put it at (`node_modules/a/node_modules/@s/b`), with its version
//! and where it came from.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, bail};
use serde::Deserialize;

use crate::deps::files::Files;

/// The npm registry. `resolved` names a tarball in it.
pub const NPM_REGISTRY: &str = "https://registry.npmjs.org/";

pub struct Lock {
    pub packages: HashMap<String, Entry>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// The package's name when it's installed under another (an alias,
    /// `"x": "npm:y@1"`).
    pub name: Option<String>,
    pub version: Option<String>,
    /// The tarball, git URL or directory; npm can leave out its registry's.
    pub resolved: Option<String>,
    /// A symlink to `resolved`, a directory: a workspace or a `file:` one.
    #[serde(default)]
    pub link: bool,
    /// In the tarball of the package it's in.
    #[serde(default)]
    pub in_bundle: bool,
}

/// Where a package came from.
#[derive(Debug, PartialEq)]
pub enum Source {
    /// The npm registry, or a mirror of it.
    Registry,
    /// A directory of the project.
    Local,
    /// Anywhere else: a git repository, a tarball, another registry.
    Other(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    lockfile_version: u32,
    #[serde(default)]
    packages: HashMap<String, Entry>,
}

impl Lock {
    pub fn read(files: &dyn Files, path: &Path) -> anyhow::Result<Lock> {
        let text = files.read(path).with_context(|| format!("could not read {}; run npm install", path.display()))?;
        let file: File = serde_json::from_str(&text).with_context(|| format!("{} is not a package-lock.json", path.display()))?;
        if !matches!(file.lockfile_version, 2 | 3) {
            bail!("{} is lockfileVersion {}, from npm 6 or older: run npm install with npm 7 or newer", path.display(), file.lockfile_version);
        }
        Ok(Lock { packages: file.packages })
    }

    /// The directory a package's link points to, relative to the project.
    pub fn link(&self, key: &str) -> Option<&str> {
        self.packages.get(key).filter(|e| e.link).and_then(|e| e.resolved.as_deref())
    }

    /// The local package (a link's target) the project path is in.
    pub fn local_package_of(&self, path: &str) -> Option<&str> {
        let targets = self.packages.values().filter(|e| e.link).filter_map(|e| e.resolved.as_deref());
        targets.filter(|t| path.strip_prefix(t).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))).max_by_key(|t| t.len())
    }

    /// Where the package at `key` came from: a bundled package from where
    /// the package bundling it did. `mirrors` are URL prefixes, each with
    /// a trailing slash, of registries that serve what the npm registry
    /// does.
    pub fn source(&self, key: &str, mirrors: &[String]) -> Source {
        let mut key = key;
        loop {
            let Some(at) = key.rfind("node_modules/") else { return Source::Local };
            let entry = self.packages.get(key);
            if entry.is_some_and(|e| e.in_bundle) {
                key = key[..at].trim_end_matches('/');
                continue;
            }
            return match entry.and_then(|e| e.resolved.as_deref()) {
                None => Source::Registry,
                Some(url) if url.starts_with(NPM_REGISTRY) || mirrors.iter().any(|m| url.starts_with(m.as_str())) => Source::Registry,
                Some(other) => Source::Other(other.to_string()),
            };
        }
    }
}

/// The name a lockfile key installs a package under: its last
/// `node_modules/` part.
pub fn name_of_key(key: &str) -> &str {
    key.rfind("node_modules/").map_or(key, |at| &key[at + "node_modules/".len()..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::files::MapFs;

    fn lock(json: &str) -> anyhow::Result<Lock> {
        let files = MapFs([(Path::new("/a/package-lock.json").to_path_buf(), json.to_string())].into());
        Lock::read(&files, Path::new("/a/package-lock.json"))
    }

    #[test]
    fn packages_come_from_the_registry_its_mirrors_the_project_or_elsewhere() {
        let lock = lock(
            r#"{"lockfileVersion": 3, "packages": {
                "": {"name": "app"},
                "node_modules/ms": {"version": "2.1.3", "resolved": "https://registry.npmjs.org/ms/-/ms-2.1.3.tgz"},
                "node_modules/omitted": {"version": "1.0.0"},
                "node_modules/mirrored": {"version": "1.0.0", "resolved": "https://npm.example.com/mirrored/-/mirrored-1.0.0.tgz"},
                "node_modules/lookalike": {"version": "1.0.0", "resolved": "https://npm.example.com.evil/x.tgz"},
                "node_modules/git": {"version": "1.0.0", "resolved": "git+ssh://git@github.com/a/git.git#abc"},
                "node_modules/tarball": {"version": "1.0.0", "resolved": "file:vendor/tarball-1.0.0.tgz"},
                "node_modules/http": {"version": "1.0.0", "resolved": "http://registry.npmjs.org/http/-/http-1.0.0.tgz"},
                "node_modules/ws": {"resolved": "packages/ws", "link": true},
                "packages/ws": {"name": "ws", "version": "0.0.0"},
                "node_modules/git/node_modules/bundled": {"version": "2.0.0", "inBundle": true},
                "node_modules/ms/node_modules/bundled": {"version": "2.0.0", "inBundle": true},
                "node_modules/ms/node_modules/bundled/node_modules/deeper": {"version": "3.0.0", "inBundle": true}
            }}"#,
        )
        .unwrap();
        let mirrors = ["https://npm.example.com/".to_string()];
        let source = |key: &str| lock.source(key, &mirrors);
        assert_eq!(source("node_modules/ms"), Source::Registry);
        assert_eq!(source("node_modules/omitted"), Source::Registry, "npm leaves out its registry's");
        assert_eq!(source("node_modules/mirrored"), Source::Registry);
        assert_eq!(lock.source("node_modules/mirrored", &[]), Source::Other("https://npm.example.com/mirrored/-/mirrored-1.0.0.tgz".into()));
        assert!(matches!(source("node_modules/lookalike"), Source::Other(_)));
        assert!(matches!(source("node_modules/git"), Source::Other(_)));
        assert!(matches!(source("node_modules/tarball"), Source::Other(_)));
        assert!(matches!(source("node_modules/http"), Source::Other(_)), "only over https");
        assert_eq!(source("packages/ws"), Source::Local);
        assert!(matches!(source("node_modules/git/node_modules/bundled"), Source::Other(_)), "from where its bundler came");
        assert_eq!(source("node_modules/ms/node_modules/bundled/node_modules/deeper"), Source::Registry);
        assert_eq!(lock.link("node_modules/ws"), Some("packages/ws"));
        assert_eq!(lock.link("node_modules/ms"), None);
        assert_eq!(lock.local_package_of("packages/ws/src/a.js"), Some("packages/ws"));
        assert_eq!(lock.local_package_of("packages/ws"), Some("packages/ws"));
        assert_eq!(lock.local_package_of("packages/wsx/a.js"), None);
        assert_eq!(name_of_key("node_modules/a/node_modules/@s/b"), "@s/b");
        assert_eq!(name_of_key("packages/ws"), "packages/ws");
    }

    #[test]
    fn only_lockfiles_of_npm_7_and_newer_are_read() {
        assert!(lock(r#"{"lockfileVersion": 2, "packages": {}}"#).is_ok());
        let old = lock(r#"{"lockfileVersion": 1, "dependencies": {}}"#).err().unwrap();
        assert!(format!("{old:#}").contains("npm 7 or newer"), "{old:#}");
        assert!(lock("{").is_err());
        let files = MapFs::default();
        let missing = Lock::read(&files, Path::new("/a/package-lock.json")).err().unwrap();
        assert!(format!("{missing:#}").contains("run npm install"), "{missing:#}");
    }
}
