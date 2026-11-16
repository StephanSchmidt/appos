//! Apps in Zig. Zig compiles to a core WebAssembly module for
//! wasm32-freestanding, no WASI, and reaches the canonical ABI directly, so
//! `appos build` needs no adapter, only the bindings and the component
//! wrapper:
//!
//!   1. bindings.rs writes `bindings/wit.zig` from `wit/`: the app's `wit`
//!      module, a Zig function for every import and `wit.exports(App)` for
//!      the world's exports.
//!   2. `zig build` runs the app's build.zig, which builds `src/main.zig`
//!      with that module (and anything else it likes, such as templates).
//!   3. wit-component wraps the module in a component.
//!
//! The shell can't tell the result from a Rust app: same contract, same
//! import check, same sandbox.

mod bindings;

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, anyhow, bail};
use wit_component::{ComponentEncoder, StringEncoding};
use wit_parser::{Resolve, WorldId};

/// The generated Zig, next to `src/`.
const BINDINGS: &str = "bindings";
/// zig build's install prefix; the module lands in `bin/`.
const OUT: &str = "target/zig";

/// An app written in Zig has a build.zig.
pub fn is_zig(dir: &Path) -> bool {
    dir.join("build.zig").is_file()
}

/// Builds the Zig app in `dir` into the component `wasm`.
pub fn build(dir: &Path, wasm: &Path) -> anyhow::Result<()> {
    let (resolve, world) = crate::ts::load_wit(&dir.join("wit"))?;
    let code = bindings::generate(&resolve, world)?;
    let path = dir.join(BINDINGS).join(bindings::FILE);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(code.as_str()) {
        std::fs::create_dir_all(dir.join(BINDINGS))?;
        std::fs::write(&path, code).with_context(|| format!("{}", path.display()))?;
    }
    let module = zig_build(dir)?;
    let component = componentize(module, resolve, world).context("could not turn zig's module into a component")?;
    if let Some(parent) = wasm.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(wasm, component).with_context(|| format!("{}", wasm.display()))
}

/// Runs `zig build --release`; returns the one .wasm it installed.
fn zig_build(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let out = dir.join(OUT);
    let status = Command::new("zig")
        .args(["build", "--release", "--summary", "none", "--prefix"])
        .arg(&out)
        .current_dir(dir)
        .status()
        .map_err(|_| anyhow!("zig not found; install it from https://ziglang.org/download/"))?;
    if !status.success() {
        bail!("build failed (see above)");
    }
    let bin = out.join("bin");
    let wasms: Vec<_> = std::fs::read_dir(&bin)
        .with_context(|| format!("zig build installed nothing in {}", bin.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
        .collect();
    let [one] = wasms.as_slice() else { bail!("expected one .wasm in {}, found {}", bin.display(), wasms.len()) };
    std::fs::read(one).with_context(|| format!("{}", one.display()))
}

/// Wraps the core module in a component of the app's world. An import the
/// world doesn't have is an error naming it: Zig apps get the host only
/// through `wit`.
fn componentize(module: Vec<u8>, resolve: Resolve, world: WorldId) -> anyhow::Result<Vec<u8>> {
    let known: Vec<(String, String)> = crate::ts::imports(&resolve, world).into_iter().map(|w| (w.module, w.name)).collect();
    for payload in wasmparser::Parser::new(0).parse_all(&module) {
        if let wasmparser::Payload::ImportSection(section) = payload? {
            for import in section.into_imports() {
                let import = import?;
                if !known.iter().any(|(m, n)| m == import.module && n == import.name) {
                    bail!(
                        "the app imports {}#{}, which the WIT doesn't have; build for wasm32-freestanding and reach the host only through the `wit` module (no WASI, no std.fs, std.process or std.time)",
                        import.module,
                        import.name
                    );
                }
            }
        }
    }
    let mut module = module;
    wit_component::embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8, false)?;
    ComponentEncoder::default().module(&module)?.validate(true).encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_encoder::{EntityType, ImportSection, Module, TypeSection};

    fn appos() -> (Resolve, WorldId) {
        let mut resolve = Resolve::new();
        let pkg = resolve.push_str("appos.wit", include_str!("../../../wit/appos.wit")).unwrap();
        let world = resolve.select_world(&[pkg], None).unwrap();
        (resolve, world)
    }

    /// std.debug.print or std.fs on wasm32-wasi would import WASI.
    #[test]
    fn an_import_outside_the_wit_is_named() {
        let mut types = TypeSection::new();
        types.ty().function([wasm_encoder::ValType::I32; 4], [wasm_encoder::ValType::I32]);
        let mut imports = ImportSection::new();
        imports.import("wasi_snapshot_preview1", "fd_write", EntityType::Function(0));
        let mut module = Module::new();
        module.section(&types).section(&imports);
        let (resolve, world) = appos();
        let err = componentize(module.finish(), resolve, world).unwrap_err().to_string();
        assert!(err.starts_with("the app imports wasi_snapshot_preview1#fd_write, which the WIT doesn't have"), "{err}");
    }
}
