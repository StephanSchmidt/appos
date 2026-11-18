//! The modules a file loads, read from its syntax: no type checker, which
//! `verbatimModuleSyntax` makes exact. The import forms that load:
//!   - `import … from "x"`, `import "x"`, `export … from "x"`, unless
//!     `import type` or `export type`: those go when the types are
//!     stripped; `import { type A } from "x"` still loads x;
//!   - `import x = require("x")`, unless `import type x = require("x")`;
//!   - `import("x")` with a string;
//!   - a call of `require`, `__require` (bundlers), `module.require` or
//!     `module["require"]` with one string, anywhere in the file.
//!
//! Imports in `declare module`, `declare namespace` and `declare global`
//! blocks are types.
//!
//! A plain `require("x")` (the identifier, not a member) is told apart:
//! scriptc writes some of those as a Node builtin instead, depending on the
//! names the file declares, which are read along.

use std::collections::HashSet;
use std::path::Path;

use anyhow::bail;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingIdentifier, CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration, ImportExpression,
    TSExternalModuleDeclaration, TSGlobalDeclaration, TSImportEqualsDeclaration, TSModuleReference, TSNamespaceDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// A module a file loads, as the file names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub spec: String,
    /// Loaded with require: the `require` export condition, not `import`.
    pub require: bool,
    /// `require("x")`, called by that name.
    pub plain: bool,
}

/// What a file loads, and every name it declares.
#[derive(Default)]
pub struct Module {
    pub edges: Vec<Edge>,
    pub declared: HashSet<String>,
}

/// The modules the file at `path` loads. `module` is whether a .js, .jsx,
/// .ts or .tsx file is an ES module (its package.json's type); the other
/// extensions say it. A file that doesn't parse as that is tried as the
/// other: Node, too, runs ES module syntax in a file without the type.
pub fn read(path: &Path, text: &str, module: bool) -> anyhow::Result<Module> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let module = match ext {
        "mjs" | "mts" => true,
        "cjs" | "cts" => false,
        _ => module,
    };
    let base = match ext {
        "ts" | "mts" | "cts" => SourceType::ts(),
        "tsx" => SourceType::tsx(),
        "jsx" => SourceType::mjs().with_jsx(true),
        _ => SourceType::mjs(),
    };
    let kind = |module: bool| if module { base.with_module(true) } else { base.with_commonjs(true) };
    for source_type in [kind(module), kind(!module)] {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, text, source_type).parse();
        if parsed.fatal_error {
            continue;
        }
        let mut found = Module::default();
        found.visit_program(&parsed.program);
        return Ok(found);
    }
    bail!("could not parse {}", path.display())
}

impl Module {
    fn add(&mut self, spec: Option<&str>, require: bool, plain: bool) {
        if let Some(spec) = spec {
            self.edges.push(Edge { spec: spec.to_string(), require, plain });
        }
    }
}

/// A string, or a template without substitutions.
fn string<'a>(e: &Expression<'a>) -> Option<&'a str> {
    match e {
        Expression::StringLiteral(s) => s.value.as_str(),
        Expression::TemplateLiteral(t) => t.single_quasi()?.as_str(),
        _ => None,
    }
}

/// Whether a callee is require: `require`, `__require`, `module.require`,
/// `module["require"]`.
fn is_require(callee: &Expression<'_>) -> bool {
    let is_module = |e: &Expression<'_>| matches!(e, Expression::Identifier(id) if id.name == "module");
    match callee.without_parentheses() {
        Expression::Identifier(id) => id.name == "require" || id.name == "__require",
        Expression::StaticMemberExpression(m) => is_module(&m.object) && m.property.name == "require",
        Expression::ComputedMemberExpression(m) => is_module(&m.object) && string(&m.expression) == Some("require"),
        _ => false,
    }
}

impl<'a> Visit<'a> for Module {
    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        self.declared.insert(it.name.to_string());
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        if !it.import_kind.is_type() {
            self.add(it.source.value.as_str(), false, false);
        }
        walk::walk_import_declaration(self, it);
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        if !it.export_kind.is_type() {
            self.add(it.source.value.as_str(), false, false);
        }
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        if !it.export_kind.is_type() {
            self.add(it.source.value.as_str(), false, false);
        }
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(r) = &it.module_reference
            && !it.import_kind.is_type()
        {
            self.add(r.expression.value.as_str(), true, false);
        }
        walk::walk_ts_import_equals_declaration(self, it);
    }

    fn visit_ts_external_module_declaration(&mut self, _: &TSExternalModuleDeclaration<'a>) {}

    fn visit_ts_global_declaration(&mut self, _: &TSGlobalDeclaration<'a>) {}

    fn visit_ts_namespace_declaration(&mut self, it: &TSNamespaceDeclaration<'a>) {
        if !it.declare {
            walk::walk_ts_namespace_declaration(self, it);
        }
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        self.add(string(&it.source), false, false);
        walk::walk_import_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if is_require(&it.callee)
            && let [argument] = it.arguments.as_slice()
        {
            let plain = matches!(&it.callee, Expression::Identifier(id) if id.name == "require");
            self.add(argument.as_expression().map(Expression::without_parentheses).and_then(string), true, plain);
        }
        walk::walk_call_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(file: &str, text: &str, module: bool) -> Vec<(String, bool)> {
        read(Path::new(file), text, module).unwrap().edges.into_iter().map(|e| (e.spec, e.require)).collect()
    }

    fn named(found: Vec<(String, bool)>) -> Vec<String> {
        found.into_iter().map(|(spec, _)| spec).collect()
    }

    #[test]
    fn imports_that_load_are_found_and_type_imports_are_not() {
        let text = r#"
            import a from "a";
            import { type T, b } from "b";
            import { type U } from "only-a-type-specifier";
            import type { V } from "type-only";
            import type W from "type-default";
            import "side-effect";
            import * as ns from "ns";
            export { x } from "re-export";
            export type { Y } from "type-re-export";
            export * from "star";
            export * as star from "star-as";
            export type * from "type-star";
            import eq = require("import-equals");
            import type teq = require("type-import-equals");
            import json from "./data.json" with { type: "json" };
            const lazy = () => import("dynamic");
            const tpl = import(`template`);
            const computed = import("dyn-" + name);
            const meta = import.meta.resolve("resolve-is-no-load");
            declare module "ambient" { import z from "in-declare-module"; }
            declare global { interface Window { q: typeof import("type-query") } }
            declare namespace D { const d: typeof import("type-in-declare-namespace"); const e = require("in-declare-namespace"); }
            namespace N { export const n = require("in-namespace"); }
            type Q = import("type-import").Q;
        "#;
        assert_eq!(
            named(specs("/a/lib.ts", text, true)),
            [
                "a",
                "b",
                "only-a-type-specifier",
                "side-effect",
                "ns",
                "re-export",
                "star",
                "star-as",
                "import-equals",
                "./data.json",
                "dynamic",
                "template",
                "in-namespace",
            ]
        );
    }

    #[test]
    fn requires_are_found_anywhere_in_the_forms_bundlers_write() {
        let text = r#"
            "use strict";
            const a = require("a");
            function later() { return require('nested'); }
            if (x) { try { require(`templ`); } catch {} }
            var b = __require("bundled");
            module.require("module-require");
            module["require"]("module-computed");
            (require)("parenthesized");
            require(("parenthesized-argument"));
            require("two", "arguments");
            require(name);
            require.resolve("resolve-is-no-load");
            other.require("not-module");
            return;
        "#;
        let found = specs("/p/index.js", text, false);
        assert!(found.iter().all(|(_, require)| *require));
        assert_eq!(named(found), ["a", "nested", "templ", "bundled", "module-require", "module-computed", "parenthesized", "parenthesized-argument"]);
        let module = read(Path::new("/p/index.js"), text, false).unwrap();
        let plain: Vec<&str> = module.edges.iter().filter(|e| e.plain).map(|e| e.spec.as_str()).collect();
        assert_eq!(plain, ["a", "nested", "templ", "parenthesized-argument"], "called as require, by that name");
    }

    #[test]
    fn the_names_a_file_declares_are_read() {
        let text = r#"
            import process from "process";
            import { a as b } from "x";
            import eq = require("y");
            var isArray = require("isarray");
            function f(require, [d], { e: g }) { let h; class C {} }
            try {} catch (k) {}
            declare module "m" { const ambient: number; }
        "#;
        let declared = read(Path::new("/a/lib.ts"), text, true).unwrap().declared;
        for name in ["process", "b", "eq", "isArray", "f", "require", "d", "g", "h", "C", "k"] {
            assert!(declared.contains(name), "{name}");
        }
        assert!(!declared.contains("a") && !declared.contains("e") && !declared.contains("ambient"));
    }

    #[test]
    fn a_file_is_read_as_its_module_kind_and_tried_as_the_other() {
        // Sloppy mode: octal literals and `with` don't parse in a module.
        let sloppy = r#"var o = 010; with (o) { require("sloppy"); }"#;
        assert_eq!(named(specs("/p/a.js", sloppy, false)), ["sloppy"]);
        assert_eq!(named(specs("/p/a.js", sloppy, true)), ["sloppy"], "a module that's CommonJS");
        assert_eq!(named(specs("/p/a.cjs", sloppy, true)), ["sloppy"]);
        let esm = r#"import x from "esm"; await x;"#;
        assert_eq!(named(specs("/p/a.js", esm, false)), ["esm"], "CommonJS by its type, a module by its syntax");
        assert_eq!(specs("/p/a.mjs", esm, false), [("esm".to_string(), false)]);
        let jsx = r#"import React from "react"; export const A = () => <div/>;"#;
        assert_eq!(named(specs("/p/a.tsx", jsx, true)), ["react"]);
        assert_eq!(named(specs("/p/a.jsx", jsx, true)), ["react"]);
        let generic = r#"const f = <T,>(x: T) => x; import("g");"#;
        assert_eq!(named(specs("/p/a.mts", generic, false)), ["g"]);
        assert!(read(Path::new("/p/a.js"), "import {", true).is_err());
    }
}
