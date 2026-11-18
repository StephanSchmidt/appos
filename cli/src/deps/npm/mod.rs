//! The npm packages compiled into a TypeScript app's .wasm. scriptc
//! compiles `bindings/entry.ts`, the modules it imports, theirs and so on,
//! and links its runtime. This follows the same imports through the files,
//! without running node, npm or scriptc:
//!   - which imports load, from the syntax alone. An import that only
//!     brings types says so (`import type`), and every other import loads
//!     its module. That holds for JavaScript files, for packages' files and
//!     for a TypeScript file under a tsconfig.json with
//!     verbatimModuleSyntax, or under none: the nearest one above the file
//!     decides. Without it tsc drops an import whose names are only used as
//!     types, which the syntax can't tell;
//!   - where each module is, as Node resolves it;
//!   - which version npm installed there and where it came from, in
//!     package-lock.json.
//!
//! `appos build` has scriptc build a library, which runs no package in
//! Node.js: every package that loads is compiled into the .wasm, or the
//! build fails.
//!
//! Where the list could be wrong it's an error instead: a tsconfig.json
//! without verbatimModuleSyntax over a TypeScript file, a scriptc this
//! doesn't know, a lockfile from before npm 7, a package that isn't what
//! package-lock.json says.
//!
//! Packages from the npm registry are listed, and from the mirrors of it
//! in APPOS_NPM_MIRRORS (URL prefixes, separated by spaces). Packages from
//! elsewhere (git, a tarball, another registry) are told and left out:
//! their names may be another package's in the registry. The project's own
//! packages (workspaces, `file:` directories) are its code, like `src/`.
//!
//!   syntax.rs   the modules a file loads
//!   resolve.rs  where they are, and the ones scriptc serves itself
//!   lock.rs     package-lock.json
//!   json.rs     package.json and tsconfig.json, as written
//!   tsconfig.rs verbatimModuleSyntax, read and set
//!   tests.rs    apps and their node_modules, as maps of files

mod json;
mod lock;
mod resolve;
mod syntax;
#[cfg(test)]
mod tests;
pub mod tsconfig;

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use appos_shell::terminal_eprintln as eprintln;

use super::Env;
use super::files::Files;
use json::Json;
use lock::{Lock, Source, name_of_key};
use resolve::{Resolver, Target};

/// The scriptc versions whose imports this follows.
const SCRIPTC: [&str; 1] = ["0.4.13"];
/// The module scriptc compiles, which `appos build` writes.
const ENTRY: &str = "bindings/entry.ts";
/// scriptc's runtime, linked into every app, and scriptc, whose own code
/// stands in for some packages.
const RUNTIME: &str = "node_modules/@scriptc/runtime-wasm32-wasi";
const SCRIPTC_PACKAGE: &str = "node_modules/scriptc";
/// The extensions of the files that load modules.
const SCRIPTS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
/// The TypeScript ones, whose imports tsc may drop.
const TYPESCRIPT: [&str; 4] = ["ts", "tsx", "mts", "cts"];

/// The npm registry's packages compiled into the app in `app`, as package
/// URLs.
pub fn packages(files: &dyn Files, app: &Path, env: &Env<'_>) -> anyhow::Result<Vec<String>> {
    let lock = Lock::read(files, &app.join("package-lock.json"))?;
    let resolver = Resolver::new(files, app, &lock, unloaded(files, app)?);
    let scriptc = resolver.manifest(&app.join(SCRIPTC_PACKAGE)).and_then(|m| m.version.clone());
    let scriptc = scriptc.context("scriptc isn't installed: run npm install")?;
    if !SCRIPTC.contains(&scriptc.as_str()) {
        bail!("scriptc {scriptc} may compile other packages than scriptc {} does, the version appos reads imports as", SCRIPTC.join(", "));
    }
    let mut used = BTreeSet::from([RUNTIME.to_string()]);
    let mut seen = HashSet::new();
    let mut configs = HashSet::new();
    let mut todo = vec![app.join(ENTRY)];
    while let Some(file) = todo.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        used.extend(resolver.owner(&file)?);
        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or_default();
        if !SCRIPTS.contains(&ext) {
            continue;
        }
        let source = resolver.is_source(&file);
        if source && TYPESCRIPT.contains(&ext) {
            verbatim_module_syntax(files, app, &file, &mut configs)?;
        }
        // scriptc writes a polyfill's builtin in installed packages' files
        // and the project's own CommonJS ones.
        let rewrites = !source || matches!(ext, "js" | "cjs");
        let text = files.read(&file).with_context(|| format!("could not read {}", file.display()))?;
        let module = syntax::read(&file, &text, resolver.is_module(&file))?;
        for edge in &module.edges {
            let rewrite = (rewrites && edge.plain).then_some(&module.declared);
            match resolver.resolve(&file, &edge.spec, edge.require, rewrite) {
                Target::Files(found) => todo.extend(found),
                Target::Shim => {
                    used.insert(SCRIPTC_PACKAGE.to_string());
                }
                Target::Builtin | Target::Missing => {}
            }
        }
    }
    let mirrors = mirrors(env);
    let mut listed = vec![];
    let mut elsewhere = vec![];
    for key in used {
        let source = lock.source(&key, &mirrors);
        if source == Source::Local {
            continue;
        }
        let entry = lock.packages.get(&key).with_context(|| format!("package-lock.json doesn't have {key}: run npm install"))?;
        let locked = entry.version.as_deref().with_context(|| format!("package-lock.json has no version of {key}"))?;
        let installed = resolver.manifest(&app.join(&key)).and_then(|m| m.version.clone());
        if installed.as_deref() != Some(locked) {
            let installed = installed.unwrap_or_else(|| "missing".into());
            bail!("{key} is {installed}, not {locked} as package-lock.json says: run npm install");
        }
        let name = entry.name.as_deref().unwrap_or(name_of_key(&key));
        match source {
            Source::Registry => listed.push(purl(name, locked)),
            Source::Other(from) => elsewhere.push(format!("{name}@{locked} from {from}")),
            Source::Local => {}
        }
    }
    if !elsewhere.is_empty() {
        eprintln!(
            "warning: deps.json leaves out what doesn't come from the npm registry, so `appos audit` doesn't check it ({}); a mirror of the registry goes in APPOS_NPM_MIRRORS",
            elsewhere.join(", ")
        );
    }
    listed.sort();
    listed.dedup();
    Ok(listed)
}

/// npm's package URL: `pkg:npm/%40scope/name@1.0.0`.
fn purl(name: &str, version: &str) -> String {
    format!("pkg:npm/{}@{version}", name.replacen('@', "%40", 1))
}

/// The registry mirrors, each ending in a slash, so a prefix is a whole
/// host or path.
fn mirrors(env: &Env<'_>) -> Vec<String> {
    let mirrors = env("APPOS_NPM_MIRRORS").unwrap_or_default();
    mirrors.split_whitespace().map(|m| if m.ends_with('/') { m.to_string() } else { format!("{m}/") }).collect()
}

/// An error unless tsc keeps the imports of the TypeScript `file` as
/// written: the nearest tsconfig.json above it sets verbatimModuleSyntax,
/// or there is none. `checked` holds the ones that set it.
fn verbatim_module_syntax(files: &dyn Files, app: &Path, file: &Path, checked: &mut HashSet<PathBuf>) -> anyhow::Result<()> {
    let nearest = file.ancestors().skip(1).map(|dir| dir.join("tsconfig.json")).find(|config| files.is_file(config));
    let Some(config) = nearest else {
        return Ok(());
    };
    if checked.contains(&config) {
        return Ok(());
    }
    let text = files.read(&config).with_context(|| format!("could not read {}", config.display()))?;
    let set = tsconfig::sets_verbatim_module_syntax(&text).with_context(|| format!("could not read {}", config.display()))?;
    if !set {
        let fix = if config == app.join("tsconfig.json") { "`appos sync` sets it" } else { "set it there" };
        bail!(
            "{} doesn't set \"verbatimModuleSyntax\": true, so the imports of {} can't tell which load a package: {fix}",
            config.display(),
            file.strip_prefix(app).unwrap_or(file).display()
        );
    }
    checked.insert(config);
    Ok(())
}

/// The packages the app's package.json marks unloaded: never loaded.
fn unloaded(files: &dyn Files, app: &Path) -> anyhow::Result<HashSet<String>> {
    let path = app.join("package.json");
    let text = files.read(&path).with_context(|| format!("could not read {}", path.display()))?;
    let manifest = Json::parse(&text).with_context(|| format!("{} is not JSON", path.display()))?;
    let packages = manifest.get("scriptc").and_then(|s| s.get("packages"));
    let entries = packages.map(Json::entries).unwrap_or_default();
    Ok(entries.iter().filter(|(_, v)| v.as_str() == Some("unloaded")).map(|(name, _)| name.clone()).collect())
}
