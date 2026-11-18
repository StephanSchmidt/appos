//! Where a module a file names is, as Node resolves it from that file, and
//! the modules scriptc doesn't load from a file at all:
//!   - Node's builtins (`fs`, `node:fs`), and graceful-fs, which it serves
//!     as fs;
//!   - polyfills whose module is a builtin on Node 24, when a package's
//!     file requires them: isarray, safe-buffer, process-nextick-args,
//!     inherits, util-deprecate and readable-stream's stream classes, at
//!     the versions and files that are. scriptc writes the builtin in place
//!     of a plain `require("isarray")` in a package's file or the app's own
//!     .js and .cjs, unless the file declares `require` or the name the
//!     builtin starts with (`process`, `Array`);
//!   - packages it ships its own code for (shims), at their main entry;
//!   - packages the app's package.json marks `"unloaded"`.
//!
//! A package's exports are matched with the conditions of an import or a
//! require, and both with and without `node`: where that picks two files,
//! both are read. readable-stream's main is read too, which scriptc may
//! serve as Node's stream by how it's used. scriptc also leaves out some
//! packages a file requires (readable-stream 4's abort-controller and
//! process), which this lists. A list can name a package too many that
//! way, never one too few.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use anyhow::Context as _;

use super::json::Json;
use super::lock::Lock;
use crate::deps::files::{Files, normalize};

/// Node's builtins without `node:`; any with it is one.
const BUILTINS: &[&str] = &[
    "_http_agent",
    "_http_client",
    "_http_common",
    "_http_incoming",
    "_http_outgoing",
    "_http_server",
    "_stream_duplex",
    "_stream_passthrough",
    "_stream_readable",
    "_stream_transform",
    "_stream_wrap",
    "_stream_writable",
    "_tls_common",
    "_tls_wrap",
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

/// npm packages served as a builtin, whatever their version.
const SERVED_AS_BUILTIN: [&str; 1] = ["graceful-fs"];

/// Packages scriptc ships its own code for, by the major versions it does.
const SHIMS: [(&str, &[u64]); 5] = [("fs-extra", &[8, 9, 10, 11]), ("chalk", &[5]), ("hot-shots", &[8]), ("@hono/node-server", &[2]), ("undici", &[5, 7])];

/// The stream classes readable-stream has a file for.
const STREAM_CLASSES: [&str; 5] = ["readable", "writable", "duplex", "transform", "passthrough"];

/// What a module a file names is.
#[derive(Debug, PartialEq)]
pub enum Target {
    /// The files it may be.
    Files(Vec<PathBuf>),
    /// No file: a builtin, or a package that's one, or unloaded.
    Builtin,
    /// scriptc's own code for the package.
    Shim,
    /// Not there: a require in a `try` of a package that isn't installed.
    Missing,
}

/// What resolving reads of a package.json.
#[derive(Default)]
pub struct Manifest {
    pub name: Option<String>,
    pub version: Option<String>,
    /// `"type": "module"`
    pub module: bool,
    main: Option<String>,
    exports: Option<Json>,
    imports: Option<Json>,
}

impl Manifest {
    fn parse(text: &str) -> Option<Manifest> {
        let json = Json::parse(text).ok()?;
        let string = |key: &str| json.get(key).and_then(Json::as_str).map(str::to_string);
        Some(Manifest {
            name: string("name"),
            version: string("version"),
            module: json.get("type").and_then(Json::as_str) == Some("module"),
            main: string("main"),
            exports: json.get("exports").filter(|e| **e != Json::Null).cloned(),
            imports: json.get("imports").cloned(),
        })
    }

    fn major(&self) -> Option<u64> {
        let version = self.version.as_deref()?;
        let (major, _) = version.split_once('.')?;
        major.parse().ok()
    }
}

pub struct Resolver<'a> {
    files: &'a dyn Files,
    app: &'a Path,
    lock: &'a Lock,
    unloaded: HashSet<String>,
    manifests: RefCell<HashMap<PathBuf, Option<Rc<Manifest>>>>,
}

impl<'a> Resolver<'a> {
    pub fn new(files: &'a dyn Files, app: &'a Path, lock: &'a Lock, unloaded: HashSet<String>) -> Resolver<'a> {
        Resolver { files, app, lock, unloaded, manifests: RefCell::default() }
    }

    /// The package.json in `dir`.
    pub fn manifest(&self, dir: &Path) -> Option<Rc<Manifest>> {
        if let Some(m) = self.manifests.borrow().get(dir) {
            return m.clone();
        }
        let manifest = self.files.read(&dir.join("package.json")).ok().and_then(|text| Manifest::parse(&text)).map(Rc::new);
        self.manifests.borrow_mut().insert(dir.to_path_buf(), manifest.clone());
        manifest
    }

    /// The nearest package.json above a file, and its directory.
    fn scope(&self, file: &Path) -> Option<(PathBuf, Rc<Manifest>)> {
        file.ancestors().skip(1).find_map(|dir| Some((dir.to_path_buf(), self.manifest(dir)?)))
    }

    /// Whether a .js, .jsx, .ts or .tsx file is an ES module.
    pub fn is_module(&self, file: &Path) -> bool {
        self.scope(file).is_some_and(|(_, m)| m.module)
    }

    /// What `spec`, imported (or required) by `from`, is. `rewrite` is
    /// there for a plain `require` in a package's file: the names the file
    /// declares.
    pub fn resolve(&self, from: &Path, spec: &str, require: bool, rewrite: Option<&HashSet<String>>) -> Target {
        if is_builtin(spec) || SERVED_AS_BUILTIN.contains(&spec) {
            return Target::Builtin;
        }
        if is_relative(spec) {
            let dir = from.parent().unwrap_or(Path::new("/"));
            return files(self.file(&dir.join(spec), self.is_source(from)));
        }
        if spec.starts_with('#') {
            return self.import_map(from, spec, require);
        }
        let Some((name, sub)) = split(spec) else { return Target::Missing };
        if self.unloaded.contains(name) {
            return Target::Builtin;
        }
        if let Some((dir, scope)) = self.scope(from)
            && scope.name.as_deref() == Some(name)
            && scope.exports.is_some()
        {
            return self.in_package(&dir, &scope, &sub, require);
        }
        let Some(dir) = self.package_dir(from, name) else { return Target::Missing };
        let Some(manifest) = self.manifest(&dir) else {
            return files(if sub == "." { self.directory(&dir, false) } else { self.file(&dir.join(&sub), false) });
        };
        if manifest.name.as_deref() == Some(name) {
            if let Some(declared) = rewrite
                && let Some(root) = builtin_alias(name, manifest.major(), &spec[name.len()..])
                && !declared.contains("require")
                && (root == "require" || !declared.contains(root))
            {
                return Target::Builtin;
            }
            if spec == name && SHIMS.iter().any(|(shim, majors)| *shim == name && manifest.major().is_some_and(|m| majors.contains(&m))) {
                return Target::Shim;
            }
        }
        self.in_package(&dir, &manifest, &sub, require)
    }

    /// The lockfile key of the package a file is in: the innermost
    /// node_modules/<name> above it, or a local package's directory; None
    /// for the project's own files.
    pub fn owner(&self, file: &Path) -> anyhow::Result<Option<String>> {
        fn name(p: Option<&Path>) -> &str {
            p.and_then(Path::file_name).and_then(|n| n.to_str()).unwrap_or_default()
        }
        let package = file.ancestors().find(|a| {
            let parent = a.parent();
            (name(parent) == "node_modules" && !name(Some(a)).starts_with('@'))
                || (name(parent).starts_with('@') && name(parent.and_then(Path::parent)) == "node_modules")
        });
        if let Some(dir) = package {
            let key = dir.strip_prefix(self.app).ok().and_then(Path::to_str);
            let key = key.with_context(|| format!("{} is outside the app, where its package-lock.json can't tell its version", dir.display()))?;
            return Ok(Some(key.to_string()));
        }
        let path = file.strip_prefix(self.app).ok().and_then(Path::to_str);
        Ok(path.and_then(|p| self.lock.local_package_of(p)).map(str::to_string))
    }

    /// Whether a file is the project's source, which imports TypeScript
    /// by the name of its JavaScript (`./a.js` for a.ts), and not an
    /// installed package's.
    pub fn is_source(&self, file: &Path) -> bool {
        !file.components().any(|c| c == Component::Normal("node_modules".as_ref()))
    }

    /// The directory Node finds a package in from a file: node_modules
    /// above it, nearest first. A link npm made is where it points.
    fn package_dir(&self, from: &Path, name: &str) -> Option<PathBuf> {
        for dir in from.ancestors().skip(1) {
            if dir.file_name().is_some_and(|n| n == "node_modules") {
                continue;
            }
            let candidate = dir.join("node_modules").join(name);
            let key = candidate.strip_prefix(self.app).ok().and_then(Path::to_str);
            if let Some(target) = key.and_then(|key| self.lock.link(key)) {
                return Some(normalize(&self.app.join(target)));
            }
            if self.files.exists(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// A subpath of the package in `dir`: through its exports, or else its
    /// main and its files.
    fn in_package(&self, dir: &Path, manifest: &Manifest, sub: &str, require: bool) -> Target {
        let Some(exports) = &manifest.exports else {
            return files(if sub == "." { self.directory(dir, false) } else { self.file(&dir.join(sub), false) });
        };
        let mut found = vec![];
        for conditions in conditions(require) {
            if let Some(Some(target)) = exports_target(exports, sub, conditions)
                && let Some(rel) = target.strip_prefix("./")
            {
                let path = normalize(&dir.join(rel));
                if self.files.is_file(&path) && !found.contains(&path) {
                    found.push(path);
                }
            }
        }
        files(found)
    }

    /// `#name`: the imports of the package the file is in.
    fn import_map(&self, from: &Path, spec: &str, require: bool) -> Target {
        let Some((dir, scope)) = self.scope(from) else { return Target::Missing };
        let Some(imports) = &scope.imports else { return Target::Missing };
        let mut found = vec![];
        let mut other = Target::Missing;
        for conditions in conditions(require) {
            let Some(Some(target)) = map_target(imports, spec, conditions, true) else { continue };
            let target = match target.strip_prefix("./") {
                Some(rel) => files(Some(normalize(&dir.join(rel))).filter(|p| self.files.is_file(p))),
                None if target.starts_with('#') => Target::Missing,
                None => self.resolve(&dir.join("package.json"), &target, require, None),
            };
            match target {
                Target::Files(paths) => found.extend(paths),
                target => other = target,
            }
        }
        found.dedup();
        if found.is_empty() { other } else { Target::Files(found) }
    }

    /// A file by its path: TypeScript's for the project's source, then the
    /// path, then with an extension; then a directory.
    fn file(&self, path: &Path, source: bool) -> Option<PathBuf> {
        let path = normalize(path);
        self.as_file(&path, source).or_else(|| self.directory(&path, source))
    }

    fn as_file(&self, path: &Path, source: bool) -> Option<PathBuf> {
        let text = path.to_str()?;
        let mut candidates = vec![];
        if source {
            for (js, ts) in [(".js", &[".ts", ".tsx"][..]), (".jsx", &[".tsx"]), (".mjs", &[".mts"]), (".cjs", &[".cts"])] {
                if let Some(stem) = text.strip_suffix(js) {
                    candidates.extend(ts.iter().map(|ts| format!("{stem}{ts}")));
                }
            }
        }
        candidates.push(text.to_string());
        let extensions: &[&str] =
            if source { &[".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"] } else { &[".js", ".json", ".mjs", ".cjs"] };
        candidates.extend(extensions.iter().map(|e| format!("{text}{e}")));
        candidates.into_iter().map(PathBuf::from).find(|c| self.files.is_file(c) && !is_declaration(c))
    }

    /// A directory: the main of its package.json, or its index.
    fn directory(&self, dir: &Path, source: bool) -> Option<PathBuf> {
        let main = self.manifest(dir).and_then(|m| m.main.clone()).map(|main| normalize(&dir.join(main)));
        let from_main = main.and_then(|main| self.as_file(&main, source).or_else(|| self.as_file(&main.join("index"), source)));
        from_main.or_else(|| self.as_file(&dir.join("index"), source))
    }
}

/// Files found, or Missing.
fn files(found: impl IntoIterator<Item = PathBuf>) -> Target {
    let found: Vec<PathBuf> = found.into_iter().collect();
    if found.is_empty() { Target::Missing } else { Target::Files(found) }
}

fn is_builtin(spec: &str) -> bool {
    spec.starts_with("node:") || BUILTINS.contains(&spec)
}

fn is_relative(spec: &str) -> bool {
    spec == "." || spec == ".." || spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/')
}

fn is_declaration(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    [".d.ts", ".d.mts", ".d.cts"].iter().any(|d| name.ends_with(d))
}

/// A package's name and the subpath in it: `@s/p/a/b` is `@s/p` and
/// `./a/b`, `p` is `p` and `.`.
fn split(spec: &str) -> Option<(&str, String)> {
    let first = spec.find('/');
    let end = match first {
        Some(slash) if spec.starts_with('@') => spec[slash + 1..].find('/').map_or(spec.len(), |n| slash + 1 + n),
        Some(slash) => slash,
        None if spec.starts_with('@') => return None,
        None => spec.len(),
    };
    let name = &spec[..end];
    if name.is_empty() || name.ends_with('/') || name.starts_with('.') || name.contains(['\\', '%']) {
        return None;
    }
    let rest = &spec[end..];
    Some((name, if rest.is_empty() { ".".to_string() } else { format!(".{rest}") }))
}

/// The builtin a polyfill's module is, by the name its expression starts
/// with (`Array.isArray`, `require("buffer")`): `rest` is what follows the
/// package's name in the specifier, `""` for its main. readable-stream's
/// main, which is a builtin or not by how it's used, isn't one.
fn builtin_alias(name: &str, major: Option<u64>, rest: &str) -> Option<&'static str> {
    let major = major?;
    let file = rest.strip_prefix('/').unwrap_or(rest);
    let file = file.strip_prefix("./").unwrap_or(file);
    let file = file.strip_suffix(".js").unwrap_or(file);
    let main_or = |other: &str| file.is_empty() || file == other;
    let builtin = match name {
        "isarray" => major <= 2 && main_or("index"),
        "safe-buffer" => major == 5 && main_or("index"),
        "process-nextick-args" => major == 2 && main_or("index"),
        "inherits" => major == 2 && main_or("inherits"),
        "util-deprecate" => major == 1 && main_or("node"),
        "readable-stream" => {
            let main = if major == 4 { "lib/ours/index" } else { "readable" };
            (2..=4).contains(&major) && !main_or(main) && STREAM_CLASSES.iter().any(|c| file == format!("lib/_stream_{c}") || (major == 2 && file == *c))
        }
        _ => false,
    };
    builtin.then_some(match name {
        "isarray" => "Array",
        "process-nextick-args" => "process",
        _ => "require",
    })
}

/// The conditions exports are matched with: scriptc's own, the import's or
/// the require's, and `default`; once without `node` and once with.
fn conditions(require: bool) -> [&'static [&'static str]; 2] {
    if require {
        [&["scriptc", "require", "default"], &["scriptc", "require", "node", "default"]]
    } else {
        [&["scriptc", "import", "default"], &["scriptc", "import", "node", "default"]]
    }
}

/// The target of a subpath in a package's exports: Some(None) where
/// they exclude it, None where they don't have it.
fn exports_target(exports: &Json, sub: &str, conditions: &[&str]) -> Option<Option<String>> {
    if exports.entries().iter().any(|(k, _)| k.starts_with('.')) {
        map_target(exports, sub, conditions, false)
    } else if sub == "." {
        target(exports, None, conditions, false)
    } else {
        None
    }
}

/// The target of a key in an exports or imports map: the key's own, or
/// the one of the pattern with a `*` that matches it best, the longest
/// before the `*`.
fn map_target(map: &Json, key: &str, conditions: &[&str], imports: bool) -> Option<Option<String>> {
    if !key.contains('*')
        && let Some(value) = map.get(key)
    {
        return target(value, None, conditions, imports);
    }
    let mut best: Option<(&str, &Json, &str)> = None;
    for (pattern, value) in map.entries() {
        let Some((prefix, suffix)) = pattern.split_once('*') else { continue };
        if suffix.contains('*') || key.len() <= prefix.len() || !key.starts_with(prefix) {
            continue;
        }
        if !suffix.is_empty() && !(key.ends_with(suffix) && key.len() >= pattern.len()) {
            continue;
        }
        let rank = |p: &str| (p.find('*'), p.len());
        if best.is_none_or(|(b, _, _)| rank(pattern) > rank(b)) {
            best = Some((pattern, value, &key[prefix.len()..key.len() - suffix.len()]));
        }
    }
    let (_, value, matched) = best?;
    target(value, Some(matched), conditions, imports)
}

/// A target: a path in the package, or for imports a package too; in a
/// conditions object the first key matched, scriptc's first; in an array
/// the first that is one. None: no condition matched.
fn target(value: &Json, matched: Option<&str>, conditions: &[&str], imports: bool) -> Option<Option<String>> {
    match value {
        Json::Str(t) => {
            let bare = imports && !is_relative(t) && !t.contains(':');
            if !t.starts_with("./") && !bare {
                return Some(None);
            }
            Some(Some(matched.map_or_else(|| t.clone(), |m| t.replace('*', m))))
        }
        Json::Object(entries) => {
            let scriptc = entries.iter().filter(|(k, _)| k == "scriptc");
            let others = entries.iter().filter(|(k, _)| k != "scriptc");
            scriptc.chain(others).filter(|(k, _)| k == "default" || conditions.contains(&k.as_str())).find_map(|(_, v)| target(v, matched, conditions, imports))
        }
        Json::Array(items) => items.iter().find_map(|item| target(item, matched, conditions, imports).filter(Option::is_some)),
        _ => Some(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_specifiers_split_into_the_name_and_the_subpath() {
        assert_eq!(split("ms"), Some(("ms", ".".to_string())));
        assert_eq!(split("ms/a/b.js"), Some(("ms", "./a/b.js".to_string())));
        assert_eq!(split("@s/p"), Some(("@s/p", ".".to_string())));
        assert_eq!(split("@s/p/a"), Some(("@s/p", "./a".to_string())));
        for invalid in ["@s", "@s/", "", ".hidden", "a\\b", "%40s/p"] {
            assert_eq!(split(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn the_builtin_polyfills_are_those_versions_and_files() {
        let alias = |name: &str, major: u64, rest: &str| builtin_alias(name, Some(major), rest);
        assert_eq!(alias("isarray", 0, ""), Some("Array"));
        assert!(alias("isarray", 2, "/index.js").is_some() && alias("isarray", 1, "/index").is_some());
        assert!(alias("isarray", 3, "").is_none() && alias("isarray", 2, "/other.js").is_none());
        assert_eq!(alias("safe-buffer", 5, ""), Some("require"));
        assert!(alias("safe-buffer", 4, "").is_none());
        assert_eq!(alias("process-nextick-args", 2, "/index.js"), Some("process"));
        assert!(alias("process-nextick-args", 1, "").is_none());
        assert!(alias("inherits", 2, "/inherits.js").is_some() && alias("inherits", 2, "/inherits_browser.js").is_none());
        assert!(alias("util-deprecate", 1, "/node.js").is_some() && alias("util-deprecate", 1, "/browser.js").is_none());
        assert!(alias("readable-stream", 2, "/duplex.js").is_some() && alias("readable-stream", 2, "/lib/_stream_writable.js").is_some());
        assert!(alias("readable-stream", 3, "/lib/_stream_passthrough.js").is_some() && alias("readable-stream", 3, "/duplex.js").is_none());
        assert!(alias("readable-stream", 4, "/lib/_stream_readable.js").is_some() && alias("readable-stream", 5, "/lib/_stream_readable.js").is_none());
        for main in [(2, "/readable.js"), (3, ""), (4, "/lib/ours/index.js")] {
            assert_eq!(alias("readable-stream", main.0, main.1), None, "its main is a builtin by how it's used");
        }
        assert!(alias("ms", 2, "").is_none() && builtin_alias("isarray", None, "").is_none());
        assert!(is_builtin("node:anything") && is_builtin("fs/promises") && is_builtin("_stream_readable") && !is_builtin("test") && !is_builtin("punycode/"));
    }

    #[test]
    fn exports_and_imports_maps_pick_their_target() {
        let json = |text: &str| Json::parse(text).unwrap();
        let import = conditions(false)[0];
        let target = |exports: &str, sub: &str| exports_target(&json(exports), sub, import);
        assert_eq!(target(r#""./main.js""#, "."), Some(Some("./main.js".into())));
        assert_eq!(target(r#""./main.js""#, "./other"), None);
        assert_eq!(target(r#"{".": "./a.js", "./b": "./b.js"}"#, "./b"), Some(Some("./b.js".into())));
        assert_eq!(target(r#"{"./*": "./lib/*.js", "./x/*": "./x/*.mjs"}"#, "./x/y"), Some(Some("./x/y.mjs".into())), "the longest prefix");
        assert_eq!(target(r#"{"./*": "./b/*", "./*.js": "./a/*.js"}"#, "./q.js"), Some(Some("./a/q.js".into())), "then the longest key");
        assert_eq!(target(r#"{"./a/*": "./a/*", "./a/secret/*": null}"#, "./a/secret/x"), Some(None));
        assert_eq!(target(r#"{"./*": "./*"}"#, "./"), None, "a pattern matches something");
        assert_eq!(target(r#"{".": ["invalid:x", "./second.js"]}"#, "."), Some(Some("./second.js".into())));
        assert_eq!(target(r#"{".": {"browser": "./b.js"}}"#, "."), None);
        assert_eq!(target(r#"{".": {"import": null, "default": "./d.js"}}"#, "."), Some(None), "a matched condition decides");
        assert_eq!(target(r#"{".": {"default": "../outside.js"}}"#, "."), Some(None));
        assert_eq!(target(r#"{".": {"import": {"node": "./n.js", "default": "./d.js"}}}"#, "."), Some(Some("./d.js".into())));
        assert_eq!(
            exports_target(&json(r#"{".": {"import": {"node": "./n.js", "default": "./d.js"}}}"#), ".", conditions(false)[1]),
            Some(Some("./n.js".into()))
        );
        assert_eq!(map_target(&json(r##"{"#a": "dep", "#b/*": "./b/*.js"}"##), "#a", import, true), Some(Some("dep".into())));
        assert_eq!(map_target(&json(r##"{"#a": "dep"}"##), "#a", import, false), Some(None), "exports name no package");
        assert_eq!(map_target(&json(r##"{"#b/*": "./b/*.js"}"##), "#b/c", import, true), Some(Some("./b/c.js".into())));
    }
}
