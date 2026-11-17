//! The workspace a crate is in, as cargo's find_root finds it, and what it
//! gives its members: dependencies, the edition, patches, the resolver.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use toml::Value;

use super::read_toml;
use crate::deps::files::{Files, normalize};

/// A workspace's root manifest, as far as resolving needs it.
pub struct Workspace {
    pub dir: PathBuf,
    /// `[workspace.dependencies]`, which `workspace = true` takes from.
    pub dependencies: toml::Table,
    /// `[patch.*]` paths, by crate name.
    pub patches: HashMap<String, PathBuf>,
    members: Vec<String>,
    exclude: Vec<String>,
    /// `[workspace.package] edition`.
    pub edition: Option<String>,
    pub resolver_1: bool,
}

impl Workspace {
    pub fn new(dir: &Path, root: &toml::Table) -> Workspace {
        let ws = root.get("workspace");
        let strings =
            |key: &str| ws.and_then(|w| w.get(key)).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let mut patches = HashMap::new();
        for (_, crates) in root.get("patch").and_then(Value::as_table).into_iter().flatten() {
            for (key, patch) in crates.as_table().into_iter().flatten() {
                if let Some(path) = patch.get("path").and_then(Value::as_str) {
                    let name = patch.get("package").and_then(Value::as_str).unwrap_or(key);
                    patches.insert(name.to_string(), normalize(&dir.join(path)));
                }
            }
        }
        let edition = ws.and_then(|w| w.get("package")?.get("edition")?.as_str()).map(str::to_string);
        // As cargo's set_resolve_behavior: `resolver`, else the root
        // package's edition; a virtual workspace has 1.
        let resolver = ws.and_then(|w| w.get("resolver")).or_else(|| root.get("package")?.get("resolver")).and_then(Value::as_str);
        let resolver_1 = match (resolver, root.get("package")) {
            (Some(resolver), _) => resolver == "1",
            (None, None) => true,
            (None, Some(package)) => edition_of(package, edition.as_deref()) < 2021,
        };
        Workspace {
            dir: dir.to_path_buf(),
            dependencies: ws.and_then(|w| w.get("dependencies")).and_then(Value::as_table).cloned().unwrap_or_default(),
            patches,
            members: strings("members"),
            exclude: strings("exclude"),
            edition,
            resolver_1,
        }
    }

    /// The workspace of the crate in `dir`: the crate's own [workspace],
    /// its `package.workspace`, or the nearest one up that doesn't exclude
    /// it.
    pub fn find(files: &dyn Files, dir: &Path) -> Option<Workspace> {
        let manifest = dir.join("Cargo.toml");
        let pointed = |from: &Path, table: &toml::Table| {
            let root = normalize(&from.join(table.get("package")?.get("workspace")?.as_str()?));
            Some(Workspace::new(&root, &read_toml(files, &root.join("Cargo.toml")).ok()?))
        };
        let own = read_toml(files, &manifest).ok()?;
        if own.contains_key("workspace") {
            return Some(Workspace::new(dir, &own));
        }
        if let Some(ws) = pointed(dir, &own) {
            return Some(ws);
        }
        for up in dir.ancestors().skip(1) {
            let Ok(table) = read_toml(files, &up.join("Cargo.toml")) else { continue };
            if table.contains_key("workspace") {
                let ws = Workspace::new(up, &table);
                if !ws.excludes(&manifest) {
                    return Some(ws);
                }
            } else if let Some(ws) = pointed(up, &table) {
                return Some(ws);
            }
        }
        None
    }

    /// Whether `exclude` keeps out the crate whose Cargo.toml is `manifest`.
    fn excludes(&self, manifest: &Path) -> bool {
        let under = |paths: &[String]| paths.iter().any(|p| manifest.starts_with(normalize(&self.dir.join(p))));
        !under(&self.members) && under(&self.exclude)
    }
}

/// A package's edition, the workspace's for `edition.workspace = true`,
/// 2015 without one.
pub fn edition_of(package: &Value, workspace: Option<&str>) -> u32 {
    let edition = match package.get("edition") {
        Some(Value::String(edition)) => Some(edition.as_str()),
        Some(e) if e.get("workspace").and_then(Value::as_bool) == Some(true) => workspace,
        _ => None,
    };
    edition.and_then(|e| e.parse().ok()).unwrap_or(2015)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_resolver_is_the_workspace_roots() {
        for (root, resolver_1) in [
            ("[package]\nedition = \"2018\"\n", true),
            ("[package]\n", true),
            ("[package]\nedition = \"2021\"\n", false),
            ("[package]\nedition = \"2018\"\nresolver = \"2\"\n", false),
            ("[package]\nedition = \"2024\"\n\n[workspace]\nresolver = \"1\"\n", true),
            ("[workspace]\nmembers = [\"app\"]\n", true),
            ("[workspace]\nresolver = \"3\"\n", false),
            ("[package]\nedition.workspace = true\n\n[workspace]\n\n[workspace.package]\nedition = \"2021\"\n", false),
        ] {
            assert_eq!(Workspace::new(Path::new("/w"), &toml::from_str(root).unwrap()).resolver_1, resolver_1, "{root}");
        }
    }

    #[test]
    fn exclude_keeps_a_crate_out_unless_members_takes_it_in() {
        let ws = Workspace::new(Path::new("/w"), &toml::from_str("[workspace]\nmembers = [\"vendor/kept\"]\nexclude = [\"vendor\", \"../x\"]\n").unwrap());
        assert!(ws.excludes(Path::new("/w/vendor/lib/Cargo.toml")));
        assert!(!ws.excludes(Path::new("/w/vendor/kept/Cargo.toml")));
        assert!(!ws.excludes(Path::new("/w/app/Cargo.toml")));
        assert!(ws.excludes(Path::new("/x/Cargo.toml")), "a path out of the workspace, normalized");
    }
}
