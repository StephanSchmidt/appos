//! Apps in Java. TeaVM (https://teavm.org) compiles Java bytecode ahead of
//! time to WebAssembly with GC: Java objects and strings live on the
//! engine's garbage-collected heap, and only what crosses to the shell goes
//! through linear memory. `appos build`:
//!
//!   1. bindings.rs writes `bindings/appos/wit/Wit.java` from `wit/`: a Java
//!      method for every import (`Wit.Page.setHtml`, `Wit.Kv.get`), the
//!      interface the app's class `app.App` implements for every export,
//!      and the canonical ABI, strings in UTF-16 like Java's.
//!   2. Maven compiles the app (`mvn -P wasm process-classes`); the pom.xml
//!      runs TeaVM in its profile `wasm`, from `appos.wit.Wit`, target
//!      WEBASSEMBLY_GC, into target/teavm/.
//!   3. rewrite.rs replaces what TeaVM expects from its JavaScript runtime
//!      (JS strings, the clock, Math.random, the console) with WebAssembly
//!      and WASI, and wit-component wraps the module in a component.
//!
//! The shell can't tell the result from a Rust app: same contract, same
//! import check, same sandbox.

mod bindings;
mod rewrite;

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, anyhow, bail};
use wit_component::{ComponentEncoder, StringEncoding};
use wit_parser::{Resolve, WorldId};

/// The generated Java, a source root next to `src/`.
const BINDINGS: &str = "bindings";
/// TeaVM's module, as the pom's profile `wasm` writes it.
const MODULE: &str = "target/teavm/classes.wasm";

/// An app written in Java has a pom.xml.
pub fn is_java(dir: &Path) -> bool {
    dir.join("pom.xml").is_file()
}

/// Builds the Java app in `dir` into the component `wasm`.
pub fn build(dir: &Path, wasm: &Path) -> anyhow::Result<()> {
    let (resolve, world) = crate::ts::load_wit(&dir.join("wit"))?;
    let code = bindings::generate(&resolve, world)?;
    let path = dir.join(BINDINGS).join(bindings::FILE);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(code.as_str()) {
        std::fs::create_dir_all(path.parent().expect("a directory"))?;
        std::fs::write(&path, code).with_context(|| format!("{}", path.display()))?;
    }
    let module = maven(dir)?;
    let component = componentize(&module, resolve, world).context("could not turn TeaVM's module into a component")?;
    if let Some(parent) = wasm.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(wasm, component).with_context(|| format!("{}", wasm.display()))
}

/// Runs Maven up to TeaVM (no tests, no jar); returns TeaVM's module.
fn maven(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let module = dir.join(MODULE);
    // A failed TeaVM run leaves the last module in place.
    let _ = std::fs::remove_file(&module);
    let status = Command::new("mvn")
        .args(["--quiet", "--batch-mode", "--activate-profiles", "wasm", "process-classes"])
        .current_dir(dir)
        .status()
        .map_err(|_| anyhow!("mvn not found; install Maven and a JDK 21 or later (e.g. apt install maven openjdk-21-jdk)"))?;
    if !status.success() {
        bail!("build failed (see above)");
    }
    std::fs::read(&module).with_context(|| format!("Maven built no {MODULE}: the pom's profile `wasm` runs TeaVM into target/teavm"))
}

/// The rewritten module as a component of the app's world, strings in
/// UTF-16.
fn componentize(module: &[u8], mut resolve: Resolve, app: WorldId) -> anyhow::Result<Vec<u8>> {
    let world = crate::ts::adapt::with_wasi(&mut resolve, app)?;
    let mut module = rewrite::rewrite(module, &crate::ts::imports(&resolve, world), &crate::ts::exports(&resolve, world))?;
    wit_component::embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF16, false)?;
    ComponentEncoder::default().module(&module)?.validate(true).encode()
}
