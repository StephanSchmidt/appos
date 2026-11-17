//! A crate's Cargo.toml: its dependencies, of every kind and platform,
//! with what `workspace = true` takes from the workspace, and its features.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use toml::Value;

use super::workspace::{Workspace, edition_of};
use super::{read_toml, target};
use crate::deps::files::{Files, normalize};

/// What resolving needs of a Cargo.toml.
pub struct Manifest {
    pub proc_macro: bool,
    /// With the implicit feature of each optional dependency that no `dep:`
    /// names, as cargo's build_feature_map adds them.
    pub features: HashMap<String, Vec<String>>,
    /// Every dependency, of every kind, for every platform.
    pub deps: Vec<Decl>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Normal,
    Build,
    Dev,
}

/// A dependency as a Cargo.toml declares it.
#[derive(Clone)]
pub struct Decl {
    /// The name in the manifest, which features use.
    pub name: String,
    /// The crate's name: `package = "..."` renames it.
    pub package: String,
    pub kind: Kind,
    /// Whether its `[target.<spec>]`, if any, holds for wasm32-wasip2.
    pub active: bool,
    pub optional: bool,
    pub default_features: bool,
    pub features: Vec<String>,
    pub req: Option<semver::VersionReq>,
    pub path: Option<PathBuf>,
    pub git: bool,
    /// From a registry other than crates.io.
    pub registry: bool,
}

impl Manifest {
    /// The Cargo.toml in `dir`, of a crate in `workspace`.
    pub fn read(files: &dyn Files, dir: &Path, workspace: Option<&Workspace>) -> anyhow::Result<Manifest> {
        const KINDS: [(&str, Kind); 5] = [
            ("dependencies", Kind::Normal),
            ("build-dependencies", Kind::Build),
            ("build_dependencies", Kind::Build),
            ("dev-dependencies", Kind::Dev),
            ("dev_dependencies", Kind::Dev),
        ];
        let path = dir.join("Cargo.toml");
        let table = read_toml(files, &path)?;
        let lib = table.get("lib");
        let proc_macro = ["proc-macro", "proc_macro"].iter().any(|k| lib.and_then(|l| l.get(k)).and_then(Value::as_bool) == Some(true));
        let edition = table.get("package").map_or(2015, |p| edition_of(p, workspace.and_then(|w| w.edition.as_deref())));
        let mut tables = vec![];
        for (key, kind) in KINDS {
            tables.extend(table.get(key).and_then(Value::as_table).map(|t| (t, kind, true)));
        }
        for (spec, platform) in table.get("target").and_then(Value::as_table).into_iter().flatten() {
            let active = target::matches(spec);
            for (key, kind) in KINDS {
                tables.extend(platform.get(key).and_then(Value::as_table).map(|t| (t, kind, active)));
            }
        }
        let mut deps = vec![];
        for (t, kind, active) in tables {
            for (name, value) in t {
                deps.push(decl(name, value, kind, active, dir, workspace, edition).with_context(|| format!("{}: {name}", path.display()))?);
            }
        }
        let mut features: HashMap<String, Vec<String>> = HashMap::new();
        for (name, values) in table.get("features").and_then(Value::as_table).into_iter().flatten() {
            features.insert(name.clone(), values.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect());
        }
        let named: HashSet<String> = features.values().flatten().filter_map(|f| f.strip_prefix("dep:")).map(str::to_string).collect();
        for d in deps.iter().filter(|d| d.optional) {
            if !features.contains_key(&d.name) && !named.contains(&d.name) {
                features.insert(d.name.clone(), vec![format!("dep:{}", d.name)]);
            }
        }
        Ok(Manifest { proc_macro, features, deps })
    }
}

/// A dependency, with what `workspace = true` takes from the workspace, as
/// cargo's inner_dependency_inherit_with merges it.
fn decl(name: &str, value: &Value, kind: Kind, active: bool, dir: &Path, workspace: Option<&Workspace>, edition: u32) -> anyhow::Result<Decl> {
    let own = value.as_table();
    let default_features = |t: Option<&toml::Table>| ["default-features", "default_features"].iter().find_map(|k| t?.get(*k)?.as_bool());
    let list = |v: Option<&Value>| v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>();
    let mut features = list(own.and_then(|t| t.get("features")));
    let (spec, spec_dir, defaults) = if own.and_then(|t| t.get("workspace")).and_then(Value::as_bool) == Some(true) {
        let ws = workspace.context("it takes `workspace = true` outside a workspace")?;
        let spec = ws.dependencies.get(name).context("[workspace.dependencies] doesn't have it")?;
        let (member, root) = (default_features(own), default_features(spec.as_table()));
        // Edition 2024 (RFC 3945): the member's say; before, only to turn
        // the defaults on.
        let defaults = if edition >= 2024 {
            member.or(root)
        } else if member == Some(true) && root == Some(false) {
            member
        } else {
            root
        };
        let mut all = list(spec.get("features"));
        all.append(&mut features);
        features = all;
        (spec, ws.dir.as_path(), defaults)
    } else {
        (value, dir, default_features(own))
    };
    let get = |key: &str| spec.as_table().and_then(|t| t.get(key));
    let req = match spec {
        Value::String(req) => Some(req.as_str()),
        _ => get("version").and_then(Value::as_str),
    };
    Ok(Decl {
        name: name.to_string(),
        package: get("package").and_then(Value::as_str).unwrap_or(name).to_string(),
        kind,
        active,
        optional: own.and_then(|t| t.get("optional")).and_then(Value::as_bool).unwrap_or(false),
        default_features: defaults.unwrap_or(true),
        features,
        req: req.and_then(|r| semver::VersionReq::parse(r).ok()),
        path: get("path").and_then(Value::as_str).map(|p| normalize(&spec_dir.join(p))),
        git: get("git").is_some(),
        registry: get("registry").is_some(),
    })
}
