//! deps.json, which `appos pack` puts in app.zip: the packages compiled
//! into the app, as package URLs, for `appos audit`. appos_shell::deps has
//! the file and the advisories.
//!
//!   files.rs   what a lister reads: the disk, or a map of files in tests
//!   cargo/     Rust: Cargo.lock and the crates' Cargo.toml, as cargo
//!              resolves them for wasm32-wasip2
//!   npm/       TypeScript: the imports from bindings/entry.ts on, through
//!              node_modules, and package-lock.json
//!
//! A language plugs in with a lister: a module whose `packages` is a
//! `Lister`, picked in `lister` below. It reads the lock file and manifests
//! through `Files` and runs no build tool. Its package URL type goes into
//! appos_shell::deps::ECOSYSTEMS, so the audit matches it.

mod cargo;
mod files;
mod npm;

pub use npm::tsconfig;

use std::path::Path;

use appos_shell::terminal_eprintln as eprintln;

use files::{Disk, Files};

/// What an environment variable is set to.
pub type Env<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// The packages compiled into the app in a directory (absolute), as
/// package URLs, from the files and the environment.
type Lister = fn(&dyn Files, &Path, &Env<'_>) -> anyhow::Result<Vec<String>>;

/// The lister for the app in `dir`, by its language as `appos build` tells
/// it (dev.rs); None for a language without one yet.
fn lister(dir: &Path) -> Option<Lister> {
    if crate::ts::is_ts(dir) {
        return Some(npm::packages);
    }
    if crate::zig::is_zig(dir) || crate::java::is_java(dir) {
        return None;
    }
    Some(cargo::packages)
}

/// deps.json for the app in `dir`. None for a language without a lister,
/// and when its packages can't be told: that warns, it doesn't stop a
/// deploy.
pub fn json(dir: &Path) -> Option<String> {
    let lister = lister(dir)?;
    let env = |key: &str| std::env::var(key).ok();
    let packages = std::path::absolute(dir).map_err(anyhow::Error::from).and_then(|dir| lister(&Disk, &dir, &env));
    match packages {
        Ok(packages) => Some(appos_shell::deps::to_json(packages)),
        Err(e) => {
            eprintln!("warning: app.zip goes without deps.json, so `appos audit` can't check its packages: {e:#}");
            None
        }
    }
}
