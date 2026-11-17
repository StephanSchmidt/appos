//! The crates compiled into a Rust app's .wasm: read from Cargo.lock and
//! the crates' Cargo.toml as cargo resolves them for `cargo build --release
//! --target wasm32-wasip2`, without running cargo.
//!
//! Cargo.lock holds what any build of the workspace could need: every
//! platform, dev- and build-dependencies, every feature of the workspace's
//! crates. Less goes into the .wasm. Cargo picks it with the manifests, and
//! so does this, after cargo's source (src/resolver/features.rs and
//! encode.rs, src/workspace/, src/sources/path.rs, crates/cargo-platform):
//!   - normal dependencies only: dev-dependencies are for tests,
//!     build-dependencies and proc-macros run in the compiler;
//!   - `[target.<spec>.dependencies]` only where the spec holds for
//!     wasm32-wasip2;
//!   - optional dependencies only when a feature turns them on, starting
//!     from the app's default features (resolver 2 and 3).
//!
//! The manifests are where `cargo build` left them: ~/.cargo/registry/src
//! for registries, ~/.cargo/git/checkouts for git, the path for path crates.
//! Where cargo could build other crates than read here, it's an error, never
//! a wrong list: resolver 1 (edition 2015 and 2018), rustflags that change
//! the target's cfg, [replace], a crate cargo hasn't downloaded.
//!
//!   lock.rs       Cargo.lock, as cargo's encode.rs decodes it
//!   workspace.rs  the workspace a crate is in, and what it gives its members
//!   manifest.rs   a crate's Cargo.toml: its dependencies and features
//!   target.rs     whether a [target.<spec>] holds for wasm32-wasip2
//!   rustflags.rs  the rustflags that would change that
//!   home.rs       where cargo keeps the crates it downloaded
//!   resolve.rs    cargo's feature resolver, and what is built
//!   tests.rs      apps and what `cargo build` left, as maps of files

mod home;
mod lock;
mod manifest;
mod resolve;
mod rustflags;
mod target;
#[cfg(test)]
mod tests;
mod workspace;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};

use super::Env;
use super::files::Files;

/// What Cargo.lock calls crates.io, the git index's name and the sparse.
const CRATES_IO: [&str; 2] = ["registry+https://github.com/rust-lang/crates.io-index", "sparse+https://index.crates.io/"];

/// The crates.io crates compiled into the app in `app`, as package URLs.
pub fn packages(files: &dyn Files, app: &Path, env: &Env<'_>) -> anyhow::Result<Vec<String>> {
    let cargo_home = match env("CARGO_HOME") {
        Some(home) => PathBuf::from(home),
        None => PathBuf::from(env("HOME").unwrap_or_default()).join(".cargo"),
    };
    let cargo_home = std::path::absolute(cargo_home)?;
    if let Some(flag) = rustflags::changing_cfg(files, app, &cargo_home, env) {
        bail!("{flag} changes the cfg rustc builds {} with, which appos doesn't read", target::TARGET);
    }
    resolve::Graph::open(files, app, &cargo_home)?.packages()
}

fn read_toml(files: &dyn Files, path: &Path) -> anyhow::Result<toml::Table> {
    let text = files.read(path).with_context(|| format!("could not read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("{} is not TOML", path.display()))
}
