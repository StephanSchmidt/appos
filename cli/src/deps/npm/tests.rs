//! TypeScript apps, their node_modules and package-lock.json, as maps of
//! files, and the packages listed for them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::deps::files::MapFs;

const APP: &str = "/w/app";
const RUNTIME_PURL: &str = "pkg:npm/%40scriptc/runtime-wasm32-wasi@0.4.13";

/// An app as `appos build` leaves it: bindings/entry.ts imports
/// src/lib.ts, which is given.
struct App {
    files: BTreeMap<PathBuf, String>,
    lock: BTreeMap<String, serde_json::Value>,
    env: Vec<(&'static str, &'static str)>,
}

impl App {
    fn new(lib: &str) -> App {
        let mut app = App { files: BTreeMap::new(), lock: BTreeMap::new(), env: vec![] };
        app.lock.insert(String::new(), serde_json::json!({"name": "app"}));
        app.file("tsconfig.json", r#"{"compilerOptions": {"strict": true, "verbatimModuleSyntax": true}}"#)
            .file("package.json", r#"{"name": "app", "type": "module"}"#)
            .file("bindings/entry.ts", r#"import { render } from "../src/lib.js"; export function run() { return render(); }"#)
            .file("bindings/types.ts", r#"import "never-imported";"#)
            .file("src/lib.ts", lib)
            .package("node_modules/scriptc", r#"{"name": "scriptc", "version": "0.4.13"}"#, &[])
            .package("node_modules/@scriptc/runtime-wasm32-wasi", r#"{"name": "@scriptc/runtime-wasm32-wasi", "version": "0.4.13"}"#, &[])
    }

    fn file(mut self, path: &str, text: &str) -> App {
        self.files.insert(Path::new(APP).join(path), text.to_string());
        self
    }

    /// A package from the npm registry at a lockfile key, with its
    /// package.json and files.
    fn package(self, key: &str, manifest: &str, files: &[(&str, &str)]) -> App {
        let version = serde_json::from_str::<serde_json::Value>(manifest).unwrap()["version"].clone();
        let name = name_of_key(key).to_string();
        let resolved = format!("https://registry.npmjs.org/{name}/-/x-{}.tgz", version.as_str().unwrap_or_default());
        self.package_from(key, manifest, files, serde_json::json!({"version": version, "resolved": resolved}))
    }

    fn package_from(mut self, key: &str, manifest: &str, files: &[(&str, &str)], entry: serde_json::Value) -> App {
        self.lock.insert(key.to_string(), entry);
        let mut app = self.file(&format!("{key}/package.json"), manifest);
        for (path, text) in files {
            app = app.file(&format!("{key}/{path}"), text);
        }
        app
    }

    fn lock_entry(mut self, key: &str, entry: serde_json::Value) -> App {
        self.lock.insert(key.to_string(), entry);
        self
    }

    fn env(mut self, key: &'static str, value: &'static str) -> App {
        self.env.push((key, value));
        self
    }

    fn list(&self) -> anyhow::Result<Vec<String>> {
        let mut files = self.files.clone();
        let lock = serde_json::json!({"name": "app", "lockfileVersion": 3, "packages": self.lock});
        files.entry(Path::new(APP).join("package-lock.json")).or_insert_with(|| lock.to_string());
        let env = |key: &str| self.env.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string());
        packages(&MapFs(files), Path::new(APP), &env)
    }

    fn listed(&self) -> Vec<String> {
        let mut listed = self.list().unwrap();
        listed.retain(|p| p != RUNTIME_PURL);
        listed
    }

    fn error(&self) -> String {
        format!("{:#}", self.list().unwrap_err())
    }
}

#[test]
fn the_runtime_is_in_every_app_and_type_imports_load_nothing() {
    let app = App::new(
        r#"
        import type { Simplify } from "type-fest";
        export type { Other } from "type-fest/other";
        import fs from "node:fs";
        import path from "path";
        export function render(): string { return path.join("a", fs.name); }
        "#,
    )
    .package("node_modules/type-fest", r#"{"name": "type-fest", "version": "4.0.0", "types": "index.d.ts"}"#, &[("index.d.ts", "")]);
    assert_eq!(app.list().unwrap(), [RUNTIME_PURL]);
}

#[test]
fn the_packages_the_app_imports_are_listed_with_theirs_and_only_those() {
    let app = App::new(
        r#"
        import ms from "ms";
        import { nanoid } from "nanoid";
        import { type Options, sha256 } from "@noble/hashes/sha2.js";
        import { helper } from "./helper.js";
        export const render = () => ms(1) + nanoid() + sha256 + helper;
        "#,
    )
    .file("src/helper.ts", r#"export { lazy } from "./nested/index.js"; export const helper = 1;"#)
    .file("src/nested/index.ts", r#"export const lazy = () => import("lazy-loaded");"#)
    .package("node_modules/ms", r#"{"name": "ms", "version": "2.1.3"}"#, &[("index.js", "module.exports = function ms() {};")])
    .package(
        "node_modules/nanoid",
        r#"{"name": "nanoid", "version": "5.0.9", "type": "module", "exports": {".": {"types": "./index.d.ts", "default": "./index.js"}}}"#,
        &[
            ("index.js", r#"import { webcrypto } from "node:crypto"; import { urlAlphabet } from "./url-alphabet/index.js"; export const nanoid = 1;"#),
            ("url-alphabet/index.js", "export const urlAlphabet = '';"),
        ],
    )
    .package(
        "node_modules/@noble/hashes",
        r#"{"name": "@noble/hashes", "version": "2.0.1", "type": "module", "exports": {"./*": "./*"}}"#,
        &[("sha2.js", r#"import { u32 } from "./utils.js"; export const sha256 = 1;"#), ("utils.js", "export const u32 = 1;")],
    )
    .package("node_modules/lazy-loaded", r#"{"name": "lazy-loaded", "version": "1.0.0", "main": "lib/main"}"#, &[("lib/main.js", "")])
    .package("node_modules/unused", r#"{"name": "unused", "version": "1.0.0"}"#, &[("index.js", "")])
    .package("node_modules/never-imported", r#"{"name": "never-imported", "version": "1.0.0"}"#, &[("index.js", "")]);
    assert_eq!(app.list().unwrap(), ["pkg:npm/%40noble/hashes@2.0.1", RUNTIME_PURL, "pkg:npm/lazy-loaded@1.0.0", "pkg:npm/ms@2.1.3", "pkg:npm/nanoid@5.0.9",]);
}

#[test]
fn a_package_is_the_one_node_finds_nearest_and_its_dependencies_theirs() {
    let app = App::new(r#"import a from "a"; import b from "b"; export const render = () => a + b;"#)
        .package("node_modules/a", r#"{"name": "a", "version": "1.0.0"}"#, &[("index.js", r#"module.exports = require("b");"#)])
        .package("node_modules/a/node_modules/b", r#"{"name": "b", "version": "2.0.0"}"#, &[("index.js", r#"require("c")"#)])
        .package("node_modules/b", r#"{"name": "b", "version": "1.0.0"}"#, &[("index.js", "")])
        .package("node_modules/c", r#"{"name": "c", "version": "3.0.0"}"#, &[("index.js", "")]);
    assert_eq!(app.listed(), ["pkg:npm/a@1.0.0", "pkg:npm/b@1.0.0", "pkg:npm/b@2.0.0", "pkg:npm/c@3.0.0"]);
}

#[test]
fn exports_are_matched_with_scriptcs_conditions_and_nodes() {
    let app = App::new(
        r#"
        import dual from "dual";
        import own from "own";
        import "patterns/features/x.js";
        export const render = () => dual + own;
        "#,
    )
    .package(
        "node_modules/dual",
        r#"{"name": "dual", "version": "1.0.0", "exports": {".": {"types": "./t.d.ts", "browser": "./browser.js", "node": "./node.js", "import": "./esm.mjs", "require": "./cjs.js"}}}"#,
        &[
            ("browser.js", r#"require("browser-only")"#),
            ("node.js", r#"require("node-only")"#),
            ("esm.mjs", r#"import "esm-only";"#),
            ("cjs.js", r#"require("require-only")"#),
        ],
    )
    .package(
        "node_modules/own",
        r#"{"name": "own", "version": "1.0.0", "exports": {"import": "./index.js", "scriptc": "./for-scriptc.js"}}"#,
        &[("index.js", r#"require("not-for-scriptc")"#), ("for-scriptc.js", r#"require("for-scriptc")"#)],
    )
    .package(
        "node_modules/patterns",
        r#"{"name": "patterns", "version": "1.0.0", "exports": {"./*": "./all/*", "./features/*.js": "./src/*.js", "./features/internal/*": null}}"#,
        &[("src/x.js", r#"require("best-pattern")"#), ("all/features/x.js", r#"require("shorter-pattern")"#)],
    );
    let mut app = app;
    for name in ["browser-only", "node-only", "esm-only", "require-only", "not-for-scriptc", "for-scriptc", "best-pattern", "shorter-pattern"] {
        app = app.package(&format!("node_modules/{name}"), &format!(r#"{{"name": "{name}", "version": "1.0.0"}}"#), &[("index.js", "")]);
    }
    assert_eq!(
        app.listed(),
        [
            "pkg:npm/best-pattern@1.0.0",
            "pkg:npm/dual@1.0.0",
            "pkg:npm/esm-only@1.0.0",
            "pkg:npm/for-scriptc@1.0.0",
            "pkg:npm/node-only@1.0.0",
            "pkg:npm/own@1.0.0",
            "pkg:npm/patterns@1.0.0",
        ],
        "the import is matched once without node and once with: both files load what they import"
    );
}

#[test]
fn a_require_follows_the_require_condition_and_a_packages_own_imports_map() {
    let app = App::new(r#"import cjs from "cjs-user"; export const render = () => cjs;"#)
        .package(
            "node_modules/cjs-user",
            r##"{"name": "cjs-user", "version": "1.0.0", "imports": {"#dep": {"node": "dep-pkg", "default": "./polyfill.js"}, "#internal/*": "./lib/*.js"}}"##,
            &[
                ("index.js", r##"const d = require("#dep"); const i = require("#internal/x"); const s = require("split"); const m = require("#missing");"##),
                ("polyfill.js", ""),
                ("lib/x.js", r#"require("from-internal")"#),
            ],
        )
        .package(
            "node_modules/split",
            r#"{"name": "split", "version": "1.0.0", "exports": {"import": "./esm.js", "require": "./cjs.js"}}"#,
            &[("esm.js", r#"import "from-import";"#), ("cjs.js", r#"require("from-require")"#)],
        );
    let mut app = app;
    for name in ["dep-pkg", "from-internal", "from-import", "from-require"] {
        app = app.package(&format!("node_modules/{name}"), &format!(r#"{{"name": "{name}", "version": "1.0.0"}}"#), &[("index.js", "")]);
    }
    assert_eq!(
        app.listed(),
        ["pkg:npm/cjs-user@1.0.0", "pkg:npm/dep-pkg@1.0.0", "pkg:npm/from-internal@1.0.0", "pkg:npm/from-require@1.0.0", "pkg:npm/split@1.0.0"]
    );
}

#[test]
fn what_scriptc_serves_itself_is_no_package_and_a_shim_is_scriptcs_code() {
    let polyfills = |app: App| {
        app.package("node_modules/isarray", r#"{"name": "isarray", "version": "2.0.5"}"#, &[("index.js", "")])
            .package("node_modules/inherits", r#"{"name": "inherits", "version": "2.0.4", "main": "inherits.js"}"#, &[("inherits.js", "")])
            .package("node_modules/process-nextick-args", r#"{"name": "process-nextick-args", "version": "2.0.1"}"#, &[("index.js", "")])
            .package("node_modules/safe-buffer", r#"{"name": "safe-buffer", "version": "5.2.1"}"#, &[("index.js", "")])
            .package("node_modules/util-deprecate", r#"{"name": "util-deprecate", "version": "1.0.2", "main": "node.js"}"#, &[("node.js", "")])
            .package(
                "node_modules/readable-stream",
                r#"{"name": "readable-stream", "version": "3.6.2", "main": "readable.js"}"#,
                &[("readable.js", r#"require("string_decoder/"); require("string_decoder")"#), ("lib/_stream_duplex.js", r#"require("string_decoder/")"#)],
            )
            .package("node_modules/string_decoder", r#"{"name": "string_decoder", "version": "1.3.0"}"#, &[("index.js", "")])
            .package("node_modules/graceful-fs", r#"{"name": "graceful-fs", "version": "4.2.11", "main": "graceful-fs.js"}"#, &[("graceful-fs.js", "")])
            .package("node_modules/encoding", r#"{"name": "encoding", "version": "0.1.13", "main": "lib/encoding.js"}"#, &[("lib/encoding.js", "")])
    };
    let served =
        polyfills(App::new(r#"import user from "user"; import gfs from "graceful-fs"; import enc from "encoding"; export const render = () => user;"#))
            .file("package.json", r#"{"name": "app", "type": "module", "scriptc": {"packages": {"encoding": "unloaded", "other": "node"}}}"#)
            .package(
                "node_modules/user",
                r#"{"name": "user", "version": "1.0.0"}"#,
                &[(
                    "index.js",
                    r#"var isArray = require('isarray'); var inherits = require("inherits/inherits.js"); var pna = require(`process-nextick-args`);
                var Duplex = require('readable-stream/lib/_stream_duplex.js'); var deprecate = require('util-deprecate'); var gfs = require('graceful-fs');"#,
                )],
            );
    assert_eq!(served.listed(), ["pkg:npm/user@1.0.0"]);

    let loaded = polyfills(App::new(r#"import isarray from "isarray"; import user from "user"; export const render = () => user;"#)).package(
        "node_modules/user",
        r#"{"name": "user", "version": "1.0.0"}"#,
        &[
            ("index.js", r#"require("./a"); require("./b"); require("./c"); require("./d");"#),
            ("a.js", r#"var process = require('process-nextick-args');"#),
            ("b.js", r#"function load(require) { return require('safe-buffer'); }"#),
            ("c.js", r#"var Readable = require('readable-stream'); module.require('util-deprecate'); (require)('inherits');"#),
            ("d.js", r#"var Array = 1; require('isarray');"#),
        ],
    );
    let in_source = polyfills(App::new(r#"import "./legacy.cjs"; import "./module.mjs"; import "./typed.ts"; export const render = () => 1;"#))
        .file("src/legacy.cjs", "require('isarray');")
        .file("src/module.mjs", "require('safe-buffer');")
        .file("src/typed.ts", "require('inherits');");
    assert_eq!(
        in_source.listed(),
        ["pkg:npm/inherits@2.0.4", "pkg:npm/safe-buffer@5.2.1"],
        "the app's own CommonJS is written as the builtin, its modules and TypeScript aren't"
    );
    assert_eq!(
        loaded.listed(),
        [
            "pkg:npm/inherits@2.0.4",
            "pkg:npm/isarray@2.0.5",
            "pkg:npm/process-nextick-args@2.0.1",
            "pkg:npm/readable-stream@3.6.2",
            "pkg:npm/safe-buffer@5.2.1",
            "pkg:npm/string_decoder@1.3.0",
            "pkg:npm/user@1.0.0",
            "pkg:npm/util-deprecate@1.0.2",
        ],
        "the app's import isn't written as the builtin, nor a require where the file declares its name, nor another spelling, nor readable-stream's main; \
         string_decoder is a builtin, string_decoder/ the package"
    );

    let shimmed = App::new(r#"import chalk from "chalk"; import fse from "fs-extra"; export const render = () => chalk;"#)
        .package(
            "node_modules/chalk",
            r#"{"name": "chalk", "version": "5.3.0", "type": "module", "exports": "./source/index.js"}"#,
            &[("source/index.js", r##"import "#ansi";"##)],
        )
        .package("node_modules/fs-extra", r#"{"name": "fs-extra", "version": "11.2.0", "main": "lib/index.js"}"#, &[("lib/index.js", r#"require("jsonfile")"#)])
        .package("node_modules/jsonfile", r#"{"name": "jsonfile", "version": "6.1.0"}"#, &[("index.js", "")]);
    assert_eq!(shimmed.listed(), ["pkg:npm/scriptc@0.4.13"]);
    let compiled = App::new(
        r#"import chalk from "chalk"; import utils from "fs-extra/lib/util/utimes.js"; import old from "isarray/build.js"; export const render = () => chalk;"#,
    )
    .package("node_modules/chalk", r#"{"name": "chalk", "version": "4.1.2", "main": "source"}"#, &[("source/index.js", r#"require("ansi-styles")"#)])
    .package("node_modules/ansi-styles", r#"{"name": "ansi-styles", "version": "4.3.0"}"#, &[("index.js", "")])
    .package("node_modules/fs-extra", r#"{"name": "fs-extra", "version": "11.2.0"}"#, &[("lib/util/utimes.js", "")])
    .package("node_modules/isarray", r#"{"name": "isarray", "version": "2.0.5"}"#, &[("build.js", "")]);
    assert_eq!(
        compiled.listed(),
        ["pkg:npm/ansi-styles@4.3.0", "pkg:npm/chalk@4.1.2", "pkg:npm/fs-extra@11.2.0", "pkg:npm/isarray@2.0.5"],
        "chalk 4 isn't shimmed, a subpath of fs-extra isn't, nor isarray's other files"
    );
}

#[test]
fn a_require_of_what_isnt_installed_is_no_package() {
    let app = App::new(r#"import opt from "optional-user"; export const render = () => opt;"#).package(
        "node_modules/optional-user",
        r#"{"name": "optional-user", "version": "1.0.0", "exports": {".": "./index.js", "./hidden": null}}"#,
        &[(
            "index.js",
            r#"try { require("not-installed"); require("optional-user/hidden"); require("optional-user/unexported"); require("./gone"); } catch {}"#,
        )],
    );
    assert_eq!(app.listed(), ["pkg:npm/optional-user@1.0.0"]);
}

#[test]
fn packages_from_elsewhere_are_left_out_and_mirrors_count_as_the_registry() {
    let lib = r#"import a from "from-git"; import b from "aliased"; import c from "mirrored"; import d from "local-lib"; import e from "lookalike"; export const render = () => a + b + c + d + e;"#;
    let app = App::new(lib)
        .package_from(
            "node_modules/from-git",
            r#"{"name": "from-git", "version": "1.0.0"}"#,
            &[("index.js", r#"require("bundled")"#)],
            serde_json::json!({"version": "1.0.0", "resolved": "git+ssh://git@github.com/x/from-git.git#0123"}),
        )
        .package_from(
            "node_modules/from-git/node_modules/bundled",
            r#"{"name": "bundled", "version": "1.0.0"}"#,
            &[("index.js", "")],
            serde_json::json!({"version": "1.0.0", "inBundle": true}),
        )
        .package_from(
            "node_modules/aliased",
            r#"{"name": "real-name", "version": "3.0.0"}"#,
            &[("index.js", "")],
            serde_json::json!({"name": "real-name", "version": "3.0.0", "resolved": "https://registry.npmjs.org/real-name/-/real-name-3.0.0.tgz"}),
        )
        .package_from(
            "node_modules/mirrored",
            r#"{"name": "mirrored", "version": "1.0.0"}"#,
            &[("index.js", "")],
            serde_json::json!({"version": "1.0.0", "resolved": "https://npm.example.com/mirrored/-/mirrored-1.0.0.tgz"}),
        )
        .package_from(
            "node_modules/lookalike",
            r#"{"name": "lookalike", "version": "1.0.0"}"#,
            &[("index.js", "")],
            serde_json::json!({"version": "1.0.0", "resolved": "https://npm.example.com.evil/lookalike/-/lookalike-1.0.0.tgz"}),
        )
        .lock_entry("node_modules/local-lib", serde_json::json!({"resolved": "packages/local-lib", "link": true}))
        .lock_entry("packages/local-lib", serde_json::json!({"name": "local-lib", "version": "0.0.0"}))
        .file("packages/local-lib/package.json", r#"{"name": "local-lib", "version": "0.0.0", "main": "src/index.ts"}"#)
        .file("packages/local-lib/src/index.ts", r#"import { x } from "./x.js"; import dep from "dep-of-local";"#)
        .file("packages/local-lib/src/x.ts", r#"export const x = 1;"#)
        .package("node_modules/dep-of-local", r#"{"name": "dep-of-local", "version": "1.0.0"}"#, &[("index.js", "")]);
    assert_eq!(app.listed(), ["pkg:npm/dep-of-local@1.0.0", "pkg:npm/real-name@3.0.0"]);
    let app = app.env("APPOS_NPM_MIRRORS", "https://other.example.com  https://npm.example.com");
    assert_eq!(app.listed(), ["pkg:npm/dep-of-local@1.0.0", "pkg:npm/mirrored@1.0.0", "pkg:npm/real-name@3.0.0"], "a mirror is a whole host or path");
}

#[test]
fn where_the_list_could_be_wrong_there_is_none() {
    let lib = r#"import ms from "ms"; export const render = () => ms(1);"#;
    let ms = |app: App, version: &str| app.package("node_modules/ms", &format!(r#"{{"name": "ms", "version": "{version}"}}"#), &[("index.js", "")]);
    assert_eq!(ms(App::new(lib), "2.1.3").listed(), ["pkg:npm/ms@2.1.3"]);

    let without = ms(App::new(lib), "2.1.3").file("tsconfig.json", r#"{"compilerOptions": {"strict": true}}"#);
    assert!(without.error().contains("verbatimModuleSyntax") && without.error().contains("appos sync"), "{}", without.error());
    let off = ms(App::new(lib), "2.1.3").file("tsconfig.json", r#"{"compilerOptions": {"verbatimModuleSyntax": false}}"#);
    assert!(off.error().contains("verbatimModuleSyntax"), "{}", off.error());

    let newer = ms(App::new(lib), "2.1.3").package("node_modules/scriptc", r#"{"name": "scriptc", "version": "0.5.0"}"#, &[]);
    assert!(newer.error().contains("scriptc 0.5.0"), "{}", newer.error());

    let mut stale = ms(App::new(lib), "2.1.3");
    stale.lock.insert("node_modules/ms".into(), serde_json::json!({"version": "2.1.2", "resolved": "https://registry.npmjs.org/ms/-/ms-2.1.2.tgz"}));
    assert!(stale.error().contains("not 2.1.2 as package-lock.json says"), "{}", stale.error());

    let mut unlocked = ms(App::new(lib), "2.1.3");
    unlocked.lock.remove("node_modules/ms");
    assert!(unlocked.error().contains("package-lock.json doesn't have node_modules/ms"), "{}", unlocked.error());

    let mut v1 = ms(App::new(lib), "2.1.3");
    v1.files.insert(Path::new(APP).join("package-lock.json"), r#"{"lockfileVersion": 1, "dependencies": {}}"#.into());
    assert!(v1.error().contains("npm 7"), "{}", v1.error());

    let unparsed = ms(App::new("export const render = () => {"), "2.1.3");
    assert!(unparsed.error().contains("could not parse"), "{}", unparsed.error());
}

#[test]
fn the_tsconfig_nearest_a_typescript_file_tells_if_its_imports_are_as_written() {
    let lib = r#"import ms from "ms"; import "./legacy/old.js"; import "./scripts/tool.js"; export const render = () => ms(1);"#;
    let app = || {
        App::new(lib)
            .package("node_modules/ms", r#"{"name": "ms", "version": "2.1.3"}"#, &[("index.js", "")])
            .package("node_modules/typed", r#"{"name": "typed", "version": "1.0.0", "main": "index.ts"}"#, &[("index.ts", ""), ("tsconfig.json", "{}")])
            .file("src/legacy/old.ts", r#"import "typed";"#)
            .file("src/scripts/tool.js", "")
            .file("src/scripts/tsconfig.json", "{}")
    };
    assert_eq!(app().listed(), ["pkg:npm/ms@2.1.3", "pkg:npm/typed@1.0.0"], "a JavaScript file and a package's file are as written");

    let nested = app().file("src/legacy/tsconfig.json", r#"{"compilerOptions": {"strict": true}}"#);
    assert!(nested.error().starts_with("/w/app/src/legacy/tsconfig.json doesn't set"), "{}", nested.error());
    assert!(nested.error().contains("src/legacy/old.ts") && nested.error().ends_with("set it there"), "{}", nested.error());
    let broken = app().file("src/legacy/tsconfig.json", "{ broken");
    assert!(broken.error().contains("could not read /w/app/src/legacy/tsconfig.json"), "{}", broken.error());

    let mut none = app();
    none.files.remove(&Path::new(APP).join("tsconfig.json"));
    assert_eq!(none.listed(), ["pkg:npm/ms@2.1.3", "pkg:npm/typed@1.0.0"], "with none, as Node strips types");
    none.files.insert(PathBuf::from("/w/tsconfig.json"), "{}".into());
    assert!(none.error().starts_with("/w/tsconfig.json doesn't set") && none.error().contains("bindings/entry.ts"), "the one above the app: {}", none.error());
}

#[test]
fn a_package_above_the_app_has_no_version_to_tell() {
    let mut app = App::new(r#"import up from "up-there"; export const render = () => up;"#);
    app.files.insert(PathBuf::from("/w/node_modules/up-there/package.json"), r#"{"name": "up-there", "version": "1.0.0"}"#.into());
    app.files.insert(PathBuf::from("/w/node_modules/up-there/index.js"), String::new());
    assert!(app.error().contains("outside the app"), "{}", app.error());
}

#[test]
fn the_templates_scriptc_is_one_this_reads_imports_as() {
    let template: serde_json::Value = serde_json::from_str(include_str!("../../../template-ts/package.json")).unwrap();
    for name in ["scriptc", "@scriptc/runtime-wasm32-wasi"] {
        let version = template["devDependencies"][name].as_str().unwrap();
        assert!(SCRIPTC.contains(&version), "{name} {version}: follow its imports, then add it to SCRIPTC");
    }
}

/// Against scriptc itself, on apps on this machine, built: none fewer than
/// scriptc compiles, and those more printed.
/// APPOS_NPM_DIRS=dir:dir cargo test -p appos --bin appos -- --ignored as_scriptc_compiles --nocapture
#[test]
#[ignore]
fn as_scriptc_compiles_them() {
    use crate::deps::files::Disk;
    for dir in std::env::var("APPOS_NPM_DIRS").unwrap_or_default().split(':').filter(|d| !d.is_empty()) {
        let out = std::process::Command::new("node_modules/.bin/scriptc")
            .args(["coverage", ENTRY])
            .env("SCRIPTC_TARGET", "wasm32-wasi")
            .env("NO_COLOR", "1")
            .current_dir(dir)
            .output()
            .unwrap();
        let report = String::from_utf8_lossy(&out.stdout);
        let compiled: std::collections::BTreeSet<String> = report
            .lines()
            .skip_while(|l| !l.contains("npm package compilation:"))
            .skip(1)
            .map_while(|l| l.split_whitespace().collect::<Vec<_>>().split_first().map(|(name, rest)| (name.to_string(), rest.to_vec())))
            .filter(|(_, status)| status.first() == Some(&"static"))
            .map(|(name, _)| name)
            .collect();
        let env = |key: &str| std::env::var(key).ok();
        let ours: std::collections::BTreeSet<String> = packages(&Disk, &std::path::absolute(dir).unwrap(), &env)
            .unwrap()
            .into_iter()
            .filter(|p| p != RUNTIME_PURL)
            .map(|p| appos_shell::deps::Purl::parse(&p).unwrap().package().into_owned())
            .collect();
        assert!(!compiled.is_empty() || report.contains("npm package compilation:") || ours.is_empty(), "{dir}: no report: {report}");
        assert_eq!(compiled.difference(&ours).collect::<Vec<_>>(), Vec::<&String>::new(), "{dir}: fewer than scriptc compiles");
        let more: Vec<&String> = ours.difference(&compiled).collect();
        println!("{dir}: {} packages, scriptc compiles {}; also listed: {more:?}", ours.len(), compiled.len());
    }
}
