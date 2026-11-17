//! Where cargo keeps the crates it downloaded: $CARGO_HOME, ~/.cargo.

use std::path::{Path, PathBuf};

use anyhow::bail;
use toml::Value;

use super::lock::Locked;
use super::{CRATES_IO, read_toml};
use crate::deps::files::Files;

pub struct CargoHome<'f> {
    pub files: &'f dyn Files,
    pub dir: PathBuf,
}

impl CargoHome<'_> {
    /// Where cargo unpacked a crate from a registry:
    /// registry/src/<host>-<hash>/<name>-<version>.
    pub fn registry_crate(&self, p: &Locked, source: &str) -> anyhow::Result<PathBuf> {
        let hosts = if CRATES_IO.contains(&source) {
            vec!["index.crates.io", "github.com"]
        } else {
            let url = source.split_once('+').map_or(source, |(_, url)| url);
            let url = url.split_once("://").map_or(url, |(_, rest)| rest);
            vec![url.split(['/', ':']).next().unwrap_or(url)]
        };
        let src = self.dir.join("registry/src");
        let from_host = |dir: &PathBuf| dir.file_name().is_some_and(|n| hosts.iter().any(|h| n.to_string_lossy().starts_with(&format!("{h}-"))));
        match self
            .files
            .dirs(&src)
            .iter()
            .filter(|dir| from_host(dir))
            .map(|index| index.join(format!("{}-{}", p.name, p.version)))
            .find(|dir| self.files.exists(&dir.join("Cargo.toml")))
        {
            Some(dir) => Ok(dir),
            None => bail!("{} {} is not in {}; build the app first", p.name, p.version, src.display()),
        }
    }

    /// Where cargo checked out a git crate: git/checkouts/<repo>-<hash>/
    /// <short rev>/, the crate the first found below, as cargo's
    /// read_packages walks.
    pub fn git_crate(&self, p: &Locked, source: &str) -> anyhow::Result<PathBuf> {
        let (url, rev) = source.trim_start_matches("git+").rsplit_once('#').unwrap_or((source, ""));
        let url = url.split('?').next().unwrap_or(url).trim_end_matches('/');
        let repo = url.rsplit('/').next().unwrap_or(url);
        let repo = format!("{}-", repo.strip_suffix(".git").unwrap_or(repo));
        let checkouts = self.dir.join("git/checkouts");
        for repo_dir in self.files.dirs(&checkouts).iter().filter(|d| d.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&repo))) {
            for checkout in self.files.dirs(repo_dir) {
                let short = checkout.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if short.len() >= 7
                    && rev.starts_with(&short)
                    && let Some(dir) = self.find_crate(&checkout, &checkout, p)
                {
                    return Ok(dir);
                }
            }
        }
        bail!("{} {} from {source} is not in {}; build the app first", p.name, p.version, checkouts.display())
    }

    fn find_crate(&self, top: &Path, dir: &Path, p: &Locked) -> Option<PathBuf> {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // Not into hidden directories and git submodules, nor a crate's target.
        if dir != top && (name.starts_with('.') || self.files.exists(&dir.join(".git"))) {
            return None;
        }
        if name == "target" && dir.parent().is_some_and(|up| self.files.exists(&up.join("Cargo.toml"))) {
            return None;
        }
        if let Ok(manifest) = read_toml(self.files, &dir.join("Cargo.toml"))
            && let Some(package) = manifest.get("package")
            && package.get("name").and_then(Value::as_str) == Some(&p.name)
            && package.get("version").and_then(Value::as_str).is_none_or(|v| v == p.version)
        {
            return Some(dir.to_path_buf());
        }
        self.files.dirs(dir).iter().find_map(|d| self.find_crate(top, d, p))
    }
}
