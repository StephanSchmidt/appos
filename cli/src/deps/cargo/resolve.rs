//! Cargo's feature resolver (src/resolver/features.rs, resolver 2) for the
//! target, on the manifests' declarations and Cargo.lock's edges, and the
//! crates built from it.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context as _, bail};
use toml::Value;

use super::home::CargoHome;
use super::lock::{Lock, Locked};
use super::manifest::{Decl, Kind, Manifest};
use super::target::TARGET;
use super::workspace::Workspace;
use super::{CRATES_IO, read_toml};
use crate::deps::files::Files;

/// A feature value as `[features]` and a dependency's `features` write it.
enum Fv<'a> {
    Feature(&'a str),
    Dep(&'a str),
    DepFeature { dep: &'a str, feature: &'a str, weak: bool },
}

fn fv(s: &str) -> Fv<'_> {
    match s.split_once('/') {
        Some((dep, feature)) => match dep.strip_suffix('?') {
            Some(dep) => Fv::DepFeature { dep, feature, weak: true },
            None => Fv::DepFeature { dep, feature, weak: false },
        },
        None => s.strip_prefix("dep:").map_or(Fv::Feature(s), Fv::Dep),
    }
}

/// A declared dependency, with the crate in Cargo.lock it resolved to; None
/// for an optional one no build turns on, which Cargo.lock leaves out.
struct Edge {
    to: Option<usize>,
    decl: Decl,
}

pub struct Graph<'f> {
    files: &'f dyn Files,
    home: CargoHome<'f>,
    lock_path: PathBuf,
    lock: Lock,
    root: usize,
    workspace: Workspace,
    /// Where path crates are, from the declarations that lead to them.
    paths: HashMap<usize, PathBuf>,
    manifests: HashMap<usize, Rc<Manifest>>,
    edges: HashMap<usize, Rc<Vec<Edge>>>,
    // The resolving, as cargo's FeatureResolver keeps it.
    features: HashMap<usize, HashSet<String>>,
    /// Optional dependencies a feature turned on, by name.
    enabled: HashMap<usize, HashSet<String>>,
    /// `dep?/feature` waiting for the dependency.
    deferred: HashMap<(usize, String), BTreeSet<String>>,
    processed: HashSet<usize>,
}

impl<'f> Graph<'f> {
    pub fn open(files: &'f dyn Files, app: &Path, cargo_home: &Path) -> anyhow::Result<Graph<'f>> {
        let manifest_path = app.join("Cargo.toml");
        let manifest = read_toml(files, &manifest_path)?;
        let workspace = Workspace::find(files, app).unwrap_or_else(|| Workspace::new(app, &manifest));
        if workspace.resolver_1 {
            bail!(
                "{} has cargo's resolver 1 (edition 2015 or 2018, or a [workspace] without `resolver`), which unifies features with crates cargo doesn't download for {TARGET}; set resolver = \"2\"",
                workspace.dir.join("Cargo.toml").display()
            );
        }
        let lock_path = workspace.dir.join("Cargo.lock");
        let lock = Lock::read(files, &lock_path)?;
        let name = manifest
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .with_context(|| format!("{} names no package", manifest_path.display()))?;
        let root = lock.path_package(name).with_context(|| format!("{name} is not in {}; build the app first", lock_path.display()))?;
        Ok(Graph {
            files,
            home: CargoHome { files, dir: cargo_home.to_path_buf() },
            lock_path,
            lock,
            root,
            workspace,
            paths: HashMap::from([(root, app.to_path_buf())]),
            manifests: HashMap::new(),
            edges: HashMap::new(),
            features: HashMap::new(),
            enabled: HashMap::new(),
            deferred: HashMap::new(),
            processed: HashSet::new(),
        })
    }

    /// Resolves the features from the app's defaults, then walks what is
    /// built for the target, as cargo's unit graph has it; the crates.io
    /// crates of it, as package URLs.
    pub fn packages(mut self) -> anyhow::Result<Vec<String>> {
        let root = self.root;
        let fvs: Vec<String> = self.manifest(root)?.features.contains_key("default").then(|| "default".to_string()).into_iter().collect();
        self.activate_pkg(root, &fvs)?;
        let mut built = HashSet::new();
        let mut todo = vec![root];
        while let Some(i) = todo.pop() {
            if self.manifest(i)?.proc_macro || !built.insert(i) {
                continue;
            }
            let enabled = self.enabled.get(&i).cloned().unwrap_or_default();
            for e in self.edges(i)?.iter() {
                if self.follows(&e.decl) && (!e.decl.optional || enabled.contains(&e.decl.name)) {
                    todo.push(self.to(i, e)?);
                }
            }
        }
        let crates_io = |p: &&Locked| p.source.as_deref().is_some_and(|s| CRATES_IO.contains(&s));
        let mut packages: Vec<String> =
            built.iter().map(|&i| &self.lock.packages[i]).filter(crates_io).map(|p| format!("pkg:cargo/{}@{}", p.name, p.version)).collect();
        packages.sort();
        Ok(packages)
    }

    // The target's normal dependencies for its platform. Proc-macros and
    // build-dependencies resolve apart, for the host, and are left out.

    fn follows(&self, d: &Decl) -> bool {
        d.kind == Kind::Normal && d.active
    }

    fn activate_pkg(&mut self, i: usize, fvs: &[String]) -> anyhow::Result<()> {
        if self.manifest(i)?.proc_macro {
            return Ok(());
        }
        self.features.entry(i).or_default();
        for f in fvs {
            self.activate_fv(i, f)?;
        }
        if !self.processed.insert(i) {
            return Ok(());
        }
        for e in self.edges(i)?.iter() {
            if !e.decl.optional && self.follows(&e.decl) {
                let to = self.to(i, e)?;
                let fvs = self.fvs_from(to, &e.decl)?;
                self.activate_pkg(to, &fvs)?;
            }
        }
        Ok(())
    }

    fn activate_fv(&mut self, i: usize, value: &str) -> anyhow::Result<()> {
        if self.manifest(i)?.proc_macro {
            return Ok(());
        }
        match fv(value) {
            Fv::Feature(f) => self.activate_rec(i, f),
            Fv::Dep(dep) => self.activate_dependency(i, dep),
            Fv::DepFeature { dep, feature, weak } => self.activate_dep_feature(i, dep, feature, weak),
        }
    }

    fn activate_rec(&mut self, i: usize, feature: &str) -> anyhow::Result<()> {
        if !self.features.entry(i).or_default().insert(feature.to_string()) {
            return Ok(());
        }
        let manifest = self.manifest(i)?;
        for f in manifest.features.get(feature).into_iter().flatten() {
            self.activate_fv(i, f)?;
        }
        Ok(())
    }

    fn activate_dependency(&mut self, i: usize, dep: &str) -> anyhow::Result<()> {
        self.enabled.entry(i).or_default().insert(dep.to_string());
        let deferred = self.deferred.remove(&(i, dep.to_string())).unwrap_or_default();
        for e in self.edges(i)?.iter() {
            if e.decl.name != dep || !self.follows(&e.decl) {
                continue;
            }
            let to = self.to(i, e)?;
            for feature in &deferred {
                self.activate_fv(to, feature)?;
            }
            let fvs = self.fvs_from(to, &e.decl)?;
            self.activate_pkg(to, &fvs)?;
        }
        Ok(())
    }

    fn activate_dep_feature(&mut self, i: usize, dep: &str, feature: &str, weak: bool) -> anyhow::Result<()> {
        for e in self.edges(i)?.iter() {
            if e.decl.name != dep || !self.follows(&e.decl) {
                continue;
            }
            if e.decl.optional {
                if weak && !self.enabled.get(&i).is_some_and(|on| on.contains(dep)) {
                    self.deferred.entry((i, dep.to_string())).or_default().insert(feature.to_string());
                    continue;
                }
                self.activate_dependency(i, dep)?;
                // As before weak dependencies: the feature of its name too,
                // unless `dep:` hid it.
                if !weak && self.manifest(i)?.features.contains_key(dep) {
                    self.activate_rec(i, dep)?;
                }
            }
            let to = self.to(i, e)?;
            self.activate_fv(to, feature)?;
        }
        Ok(())
    }

    /// The features a dependency asks of its crate.
    fn fvs_from(&mut self, to: usize, decl: &Decl) -> anyhow::Result<Vec<String>> {
        let mut fvs = decl.features.clone();
        if decl.default_features && self.manifest(to)?.features.contains_key("default") {
            fvs.push("default".into());
        }
        Ok(fvs)
    }

    // The graph: the manifests' declarations on Cargo.lock's edges.

    fn to(&self, from: usize, edge: &Edge) -> anyhow::Result<usize> {
        edge.to.with_context(|| {
            let p = &self.lock.packages[from];
            format!("{} {} depends on {}, which {} doesn't have; build the app first", p.name, p.version, edge.decl.package, self.lock_path.display())
        })
    }

    fn edges(&mut self, i: usize) -> anyhow::Result<Rc<Vec<Edge>>> {
        if let Some(edges) = self.edges.get(&i) {
            return Ok(edges.clone());
        }
        let manifest = self.manifest(i)?;
        let edges: Vec<Edge> = manifest.deps.iter().map(|d| Edge { to: self.resolved(i, d), decl: d.clone() }).collect();
        for e in &edges {
            if let (Some(to), Some(path)) = (e.to, &e.decl.path) {
                self.paths.entry(to).or_insert_with(|| path.clone());
            }
        }
        let edges = Rc::new(edges);
        self.edges.insert(i, edges.clone());
        Ok(edges)
    }

    /// The crate in Cargo.lock a dependency of `i` resolved to: of its
    /// name, a version its requirement takes, from the source it names
    /// unless a [patch] moved it; the newest of several.
    fn resolved(&self, i: usize, d: &Decl) -> Option<usize> {
        let packages = &self.lock.packages;
        let version = |c: usize| semver::Version::parse(&packages[c].version).ok();
        let takes = |c: usize| d.req.as_ref().is_none_or(|req| version(c).is_some_and(|v| req.matches(&v)));
        let candidates: Vec<usize> = self.lock.deps[i].iter().copied().filter(|&c| packages[c].name == d.package && takes(c)).collect();
        let from = packages[i].source.as_deref();
        let named = |c: usize| {
            let source = packages[c].source.as_deref();
            if d.git {
                source.is_some_and(|s| s.starts_with("git+"))
            } else if d.path.is_some() {
                // A path crate, or a crate of the same git repository.
                source == from
            } else if d.registry {
                source.is_some_and(|s| !s.starts_with("git+") && !CRATES_IO.contains(&s))
            } else {
                source.is_some_and(|s| CRATES_IO.contains(&s))
            }
        };
        let newest = |same: bool| candidates.iter().copied().filter(|&c| !same || named(c)).max_by_key(|&c| version(c));
        newest(true).or_else(|| newest(false))
    }

    fn manifest(&mut self, i: usize) -> anyhow::Result<Rc<Manifest>> {
        if let Some(manifest) = self.manifests.get(&i) {
            return Ok(manifest.clone());
        }
        let p = &self.lock.packages[i];
        let (dir, workspace) = match p.source.as_deref() {
            None => {
                let dir = self.paths.get(&i).or_else(|| self.workspace.patches.get(&p.name));
                let dir = dir.cloned().with_context(|| format!("could not find the path crate {} {}", p.name, p.version))?;
                let workspace = Workspace::find(self.files, &dir);
                (dir, workspace)
            }
            Some(source) if source.starts_with("git+") => {
                let dir = match self.paths.get(&i) {
                    Some(dir) => dir.clone(),
                    None => self.home.git_crate(p, source)?,
                };
                let workspace = Workspace::find(self.files, &dir);
                (dir, workspace)
            }
            // Registries publish manifests with the workspace filled in.
            Some(source) => (self.home.registry_crate(p, source)?, None),
        };
        let manifest = Rc::new(Manifest::read(self.files, &dir, workspace.as_ref())?);
        self.paths.entry(i).or_insert(dir);
        self.manifests.insert(i, manifest.clone());
        Ok(manifest)
    }
}
