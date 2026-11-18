//! Embeds the hello worlds `appos new` writes, file by file: cli/template
//! (Rust) as TEMPLATE, cli/template-ts (TypeScript) as TEMPLATE_TS,
//! cli/template-java (Java) as TEMPLATE_JAVA and cli/template-kit (a kit,
//! `appos new --kit`) as TEMPLATE_KIT. A file added to a template is
//! in the next `appos new`, with no list to keep. Left out at the top: build
//! output and local data (target/, appos-data/, node_modules/ and
//! bindings/), wit/ (the contract comes from
//! the platform or the CLI's own copy) and rustfmt.toml (it only keeps this
//! repository's formatting settings out of the template).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const SKIP: &[&str] = &["target", "appos-data", "wit", "rustfmt.toml", "node_modules", "bindings"];

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let mut out = String::new();
    for (name, dir, what) in [
        ("TEMPLATE", "template", "Rust app"),
        ("TEMPLATE_TS", "template-ts", "TypeScript app"),
        ("TEMPLATE_JAVA", "template-java", "Java app"),
        ("TEMPLATE_KIT", "template-kit", "kit"),
    ] {
        let root = manifest_dir.join(dir);
        let mut files = Vec::new();
        walk(&root, &root, &mut files);
        files.sort();
        writeln!(out, "/// The {what} hello world every new {what} starts from (cli/{dir}), by path.\npub const {name}: &[(&str, &str)] = &[")
            .expect("writing to a String");
        for (rel, abs) in &files {
            writeln!(out, "    ({rel:?}, include_str!({abs:?})),").expect("writing to a String");
        }
        out.push_str("];\n");
    }
    let dest = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("template.rs");
    std::fs::write(&dest, out).unwrap_or_else(|e| panic!("could not write {}: {e}", dest.display()));
}

/// Collects (path below the template, absolute path) of every file. Cargo
/// watches each file, and each directory below the top (a directory is
/// watched with everything in it, and the top holds target/).
fn walk(root: &Path, dir: &Path, files: &mut Vec<(String, String)>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("could not read {}: {e}", dir.display()));
        let path = entry.path();
        let rel = path.strip_prefix(root).expect("a file below the template").to_str().expect("template paths are UTF-8").replace('\\', "/");
        if dir == root && SKIP.contains(&rel.as_str()) {
            continue;
        }
        let kind = entry.file_type().unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
        assert!(!kind.is_symlink(), "{}/{rel} is a symlink; a template holds files (only wit/ may link)", root.display());
        println!("cargo::rerun-if-changed={}", path.display());
        if kind.is_dir() {
            walk(root, &path, files);
        } else {
            files.push((rel, path.to_str().expect("template paths are UTF-8").to_string()));
        }
    }
}
