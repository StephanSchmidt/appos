//! Apps in TypeScript. scriptc (https://github.com/vercel-labs/scriptc)
//! compiles TypeScript ahead of time to a core WebAssembly module whose host
//! calls are plain imports, its "callbacks". `appos build` turns that module
//! into a component like any other app's:
//!
//!   1. bindings.rs writes `bindings/` from `wit/`: a typed TypeScript
//!      function for every import (`page.setHtml`, `kv.get`), the interface
//!      the app implements for every export, and the scriptc profile that
//!      declares the callbacks and exports.
//!   2. scriptc compiles `bindings/entry.ts`, which imports the app's
//!      `src/lib.ts`, to a core module.
//!   3. adapt.rs rewrites that module to the component model's canonical
//!      ABI: each callback becomes its WIT import, and small shims cover the
//!      bits of WASI preview 1 that scriptc's runtime uses. wit-component then
//!      wraps it in a component.
//!
//! The shell can't tell the result from a Rust app: same contract, same
//! import check, same sandbox.

pub(crate) mod adapt;
mod bindings;

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, anyhow, bail};
use appos_shell::{terminal_print as print, terminal_println as println};
use wit_parser::abi::WasmSignature;
use wit_parser::{Function, InterfaceId, LiftLowerAbi, ManglingAndAbi, Resolve, WasmExport, WasmExportKind, WasmImport, WorldId, WorldItem, WorldKey};

/// The generated TypeScript, next to `src/`.
const BINDINGS: &str = "bindings";
/// scriptc's core module, before adapting.
const CORE: &str = "target/scriptc/core.wasm";

/// Import and export names as wit-component expects them from a core module:
/// `appos:framework/page@0.1.0` / `set-html`.
const MANGLING: ManglingAndAbi = ManglingAndAbi::Legacy(LiftLowerAbi::Sync);

/// An app written in TypeScript has a package.json.
pub fn is_ts(dir: &Path) -> bool {
    dir.join("package.json").is_file()
}

/// Builds the TypeScript app in `dir` into the component `wasm`.
pub fn build(dir: &Path, wasm: &Path) -> anyhow::Result<()> {
    let (resolve, world) = load_wit(&dir.join("wit"))?;
    let files = bindings::generate(&resolve, world)?;
    write_bindings(&dir.join(BINDINGS), &files)?;
    let core = dir.join(CORE);
    scriptc(dir, &core)?;
    let bytes = std::fs::read(&core).with_context(|| format!("{}", core.display()))?;
    let component = adapt::componentize(&bytes, resolve, world).context("could not turn scriptc's module into a component")?;
    if let Some(parent) = wasm.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(wasm, component).with_context(|| format!("{}", wasm.display()))
}

pub(crate) fn load_wit(dir: &Path) -> anyhow::Result<(Resolve, WorldId)> {
    let mut resolve = Resolve::new();
    let (package, _) = resolve.push_dir(dir).with_context(|| format!("{}", dir.display()))?;
    let world = resolve.select_world(&[package], None)?;
    Ok((resolve, world))
}

/// Writes only what changed, so editors and the dev watcher see no churn,
/// and removes files a WIT change made obsolete.
fn write_bindings(dir: &Path, files: &[(String, String)]) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, content) in files {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
            std::fs::write(&path, content).with_context(|| format!("{}", path.display()))?;
        }
    }
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let generated = [".ts", ".json", ".mjs"].iter().any(|ext| name.ends_with(ext));
        if generated && !files.iter().any(|(n, _)| *n == name) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn scriptc(dir: &Path, out: &Path) -> anyhow::Result<()> {
    let bin = dir.join("node_modules/.bin/scriptc");
    if !bin.exists() {
        println!("installing the app's packages (npm install)");
        let status = Command::new("npm")
            .arg("install")
            .current_dir(dir)
            .status()
            .map_err(|_| anyhow!("npm not found; install Node.js 24 or later from https://nodejs.org"))?;
        if !status.success() {
            bail!("npm install failed (see above)");
        }
        if !bin.exists() {
            bail!("scriptc isn't installed; add it with: npm install --save-dev scriptc @scriptc/runtime-wasm32-wasi");
        }
    }
    // scriptc links with zig and only says "spawnSync zig ENOENT" without it.
    if Command::new("zig").arg("version").output().is_err() {
        bail!("zig not found; scriptc needs it to compile TypeScript: install it from https://ziglang.org/download/");
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let profile = Path::new(BINDINGS).join("profile.json");
    // scriptc names its output on stdout; show it only with a failure.
    let output = Command::new(&bin)
        .args(["build", "--lib", "--profile"])
        .arg(&profile)
        .arg("-o")
        .arg(out)
        .env("SCRIPTC_TARGET", "wasm32-wasi")
        .current_dir(dir)
        .stderr(Stdio::inherit())
        .output()
        .context("could not run scriptc")?;
    if !output.status.success() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
        bail!("build failed (see above)");
    }
    Ok(())
}

/// A WIT function, with its names on the TypeScript and the core side.
pub(crate) struct WitFn<'a> {
    /// The scriptc callback or export: a C identifier such as
    /// `appos_framework_page_set_html`.
    pub c_name: String,
    /// The interface, `None` for a function of the world itself.
    pub interface: Option<InterfaceId>,
    pub func: &'a Function,
    /// The core import's module and name, or the export's name
    /// (`appos:framework/handler@0.1.0#start`, module empty).
    pub module: String,
    pub name: String,
    /// The core signature under the canonical ABI.
    pub sig: WasmSignature,
}

pub(crate) fn imports(resolve: &Resolve, world: WorldId) -> Vec<WitFn<'_>> {
    functions(resolve, &resolve.worlds[world].imports, |key, func| {
        let (module, name) = resolve.wasm_import_name(MANGLING, WasmImport::Func { interface: key, func });
        (module, name, resolve.wasm_signature(MANGLING.import_variant(), func))
    })
}

pub(crate) fn exports(resolve: &Resolve, world: WorldId) -> Vec<WitFn<'_>> {
    functions(resolve, &resolve.worlds[world].exports, |key, func| {
        let name = resolve.wasm_export_name(MANGLING, WasmExport::Func { interface: key, func, kind: WasmExportKind::Normal });
        (String::new(), name, resolve.wasm_signature(MANGLING.export_variant(), func))
    })
}

fn functions<'a>(
    resolve: &'a Resolve,
    items: &'a wit_parser::IndexMap<WorldKey, WorldItem>,
    names: impl Fn(Option<&WorldKey>, &Function) -> (String, String, WasmSignature),
) -> Vec<WitFn<'a>> {
    let mut out = Vec::new();
    for (key, item) in items {
        let (interface, funcs): (_, Vec<&Function>) = match item {
            WorldItem::Interface { id, .. } => (Some(*id), resolve.interfaces[*id].functions.values().collect()),
            WorldItem::Function(f) => (None, vec![f]),
            WorldItem::Type { .. } => continue,
        };
        let key = interface.map(|_| key);
        for func in funcs {
            let (module, name, sig) = names(key, func);
            out.push(WitFn { c_name: c_name(resolve, interface, &func.name), interface, func, module, name, sig });
        }
    }
    out
}

/// `appos:framework/page` + `set-html` -> `appos_framework_page_set_html`.
fn c_name(resolve: &Resolve, interface: Option<InterfaceId>, func: &str) -> String {
    let prefix = match interface {
        Some(id) => {
            let i = &resolve.interfaces[id];
            let iface = i.name.as_deref().unwrap_or("interface");
            match i.package {
                Some(p) => format!("{}_{}_{iface}", resolve.packages[p].name.namespace, resolve.packages[p].name.name),
                None => iface.to_string(),
            }
        }
        None => "world".to_string(),
    };
    format!("{prefix}_{func}").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}
