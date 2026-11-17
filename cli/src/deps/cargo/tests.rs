//! Apps as `cargo build` would leave them, as maps of files: the app in
//! /w/app, the cargo home in /cargo. One test per rule of cargo's.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use super::target::TARGET;
use super::{CRATES_IO, packages};
use crate::deps::files::{Disk, MapFs};

const IO: &str = CRATES_IO[0];
const REGISTRY: &str = "/cargo/registry/src/index.crates.io-1949cf8c6b5b557f";

/// An app in /w/app and what `cargo build` left: Cargo.lock, crates in
/// the cargo home /cargo.
#[derive(Clone)]
struct World {
    files: MapFs,
    lock: String,
    lock_at: &'static str,
    env: HashMap<&'static str, String>,
}

impl World {
    /// The app, edition 2024, with `manifest` after its [package] and
    /// `deps` its dependencies in Cargo.lock.
    fn new(manifest: &str, deps: &[&str]) -> World {
        World {
            files: MapFs::default(),
            lock: "version = 4\n\n".into(),
            lock_at: "/w/app/Cargo.lock",
            env: HashMap::from([("CARGO_HOME", "/cargo".to_string())]),
        }
        .file("/w/app/Cargo.toml", &format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{manifest}"))
        .locked("app", "0.1.0", None, deps)
    }

    fn file(mut self, path: &str, text: &str) -> World {
        self.files.0.insert(path.into(), text.into());
        self
    }

    /// A [[package]] in Cargo.lock.
    fn locked(mut self, name: &str, version: &str, source: Option<&str>, deps: &[&str]) -> World {
        let source = source.map(|s| format!("source = \"{s}\"\n")).unwrap_or_default();
        let deps = deps.iter().map(|d| format!("\"{d}\"")).collect::<Vec<_>>().join(", ");
        self.lock += &format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n{source}dependencies = [{deps}]\n\n");
        self
    }

    /// A crate from crates.io in Cargo.lock and downloaded, with
    /// `manifest` after its [package].
    fn krate(self, name: &str, version: &str, deps: &[&str], manifest: &str) -> World {
        let path = format!("{REGISTRY}/{name}-{version}/Cargo.toml");
        self.locked(name, version, Some(IO), deps).file(&path, &format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n\n{manifest}"))
    }

    fn edition(self, edition: &str) -> World {
        let app = self.files.0[Path::new("/w/app/Cargo.toml")].replace("edition = \"2024\"", &format!("edition = \"{edition}\""));
        self.file("/w/app/Cargo.toml", &app)
    }

    fn lock_at(mut self, path: &'static str) -> World {
        self.lock_at = path;
        self
    }

    fn env(mut self, key: &'static str, value: &str) -> World {
        self.env.insert(key, value.into());
        self
    }

    fn packages(&self) -> anyhow::Result<Vec<String>> {
        let files = self.clone().file(self.lock_at, &self.lock).files;
        let env = |key: &str| self.env.get(key).cloned();
        packages(&files, Path::new("/w/app"), &env)
    }

    /// The packages, without the pkg:cargo/.
    fn names(&self) -> Vec<String> {
        self.packages().unwrap().iter().map(|p| p.trim_start_matches("pkg:cargo/").to_string()).collect()
    }

    fn error(&self) -> String {
        format!("{:#}", self.packages().unwrap_err())
    }
}

#[test]
fn only_normal_dependencies_for_the_target_count_and_proc_macros_stop_the_walk() {
    let app = r#"
[dependencies]
serde = { version = "1", features = ["derive"] }
helper = { path = "../helper" }

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-only = "1"

[target.'cfg(windows)'.dependencies]
windows-sys = "0.59"

[build-dependencies]
cc = "1"

[dev-dependencies]
proptest = "1"
"#;
    let serde =
        "[features]\ndefault = [\"std\"]\nstd = []\nderive = [\"dep:serde_derive\"]\n\n[dependencies]\nserde_derive = { version = \"1\", optional = true }\n";
    let world = World::new(app, &["cc", "helper", "proptest", "serde", "wasm-only", "windows-sys"])
        .locked("helper", "0.1.0", None, &["itoa"])
        .file("/w/helper/Cargo.toml", "[package]\nname = \"helper\"\nversion = \"0.1.0\"\n\n[dependencies]\nitoa = \"1\"\n")
        .krate("serde", "1.0.228", &["serde_derive"], serde)
        .krate("serde_derive", "1.0.228", &["syn"], "[lib]\nproc-macro = true\n\n[dependencies]\nsyn = \"2\"\n")
        .krate("syn", "2.0.0", &[], "")
        .krate("itoa", "1.0.15", &[], "")
        .krate("wasm-only", "1.0.0", &[], "")
        // Not downloaded: cargo doesn't need them for this build.
        .locked("windows-sys", "0.59.0", Some(IO), &[])
        .locked("cc", "1.2.0", Some(IO), &[])
        .locked("proptest", "1.0.0", Some(IO), &[]);
    assert_eq!(world.names(), ["itoa@1.0.15", "serde@1.0.228", "wasm-only@1.0.0"]);
}

#[test]
fn optional_dependencies_count_when_a_feature_turns_them_on() {
    let lib = r#"
[features]
default = ["std"]
std = ["implicit", "dep:explicit", "renamed/extra", "weak?/x"]
unused = ["dep:never"]

[dependencies]
implicit = { version = "1", optional = true }
explicit = { version = "1", optional = true }
renamed = { package = "real", version = "2", optional = true }
weak = { version = "1", optional = true }
never = { version = "1", optional = true }
ghost = { version = "1", optional = true }
"#;
    let world = |app: &str| {
        World::new(app, &["lib"])
            // ghost: optional, never on, so not in Cargo.lock.
            .krate("lib", "1.0.0", &["explicit", "implicit", "never", "real 2.1.0", "weak"], lib)
            .krate("implicit", "1.0.0", &[], "")
            .krate("explicit", "1.0.0", &[], "")
            .krate(
                "real",
                "2.1.0",
                &["extra-dep"],
                "[features]\nextra = [\"dep:extra-dep\"]\n\n[dependencies]\nextra-dep = { version = \"1\", optional = true }\n",
            )
            .krate("real", "1.0.0", &[], "")
            .krate("extra-dep", "1.0.0", &[], "")
            .krate("weak", "1.0.0", &[], "")
            .krate("never", "1.0.0", &[], "")
    };
    assert_eq!(world("[dependencies]\nlib = \"1\"\n").names(), ["explicit@1.0.0", "extra-dep@1.0.0", "implicit@1.0.0", "lib@1.0.0", "real@2.1.0"]);
    assert_eq!(world("[dependencies]\nlib = { version = \"1\", default-features = false }\n").names(), ["lib@1.0.0"], "without default features, none of them");
}

#[test]
fn a_weak_feature_waits_for_its_dependency() {
    let lib = |default: &str| {
        format!("[features]\ndefault = {default}\na = [\"opt?/fast\"]\nb = [\"dep:opt\"]\n\n[dependencies]\nopt = {{ version = \"1\", optional = true }}\n")
    };
    let opt = "[features]\nfast = [\"dep:simd\"]\n\n[dependencies]\nsimd = { version = \"1\", optional = true }\n";
    let world = |default: &str| {
        World::new("[dependencies]\nlib = \"1\"\n", &["lib"]).krate("lib", "1.0.0", &["opt"], &lib(default)).krate("opt", "1.0.0", &["simd"], opt).krate(
            "simd",
            "1.0.0",
            &[],
            "",
        )
    };
    assert_eq!(world(r#"["a", "b"]"#).names(), ["lib@1.0.0", "opt@1.0.0", "simd@1.0.0"], "b turns opt on after a asked for fast");
    assert_eq!(world(r#"["a"]"#).names(), ["lib@1.0.0"], "nothing turns opt on");
}

#[test]
fn dep_slash_feature_turns_on_the_dependency_and_the_feature_of_its_name() {
    let lib = r#"
[features]
default = ["opt/fast"]
opt = ["dep:opt", "dep:extra"]

[dependencies]
opt = { version = "1", optional = true }
extra = { version = "1", optional = true }
"#;
    let world = World::new("[dependencies]\nlib = \"1\"\n", &["lib"])
        .krate("lib", "1.0.0", &["extra", "opt"], lib)
        .krate("opt", "1.0.0", &["simd"], "[features]\nfast = [\"dep:simd\"]\n\n[dependencies]\nsimd = { version = \"1\", optional = true }\n")
        .krate("simd", "1.0.0", &[], "")
        .krate("extra", "1.0.0", &[], "");
    assert_eq!(world.names(), ["extra@1.0.0", "lib@1.0.0", "opt@1.0.0", "simd@1.0.0"]);
}

#[test]
fn a_crate_in_two_versions_is_two_crates() {
    let app = "[dependencies]\nold = { package = \"foo\", version = \"0.1\", features = [\"x\"] }\nfoo = \"0.2\"\n";
    let world = World::new(app, &["foo 0.1.0", "foo 0.2.0"])
        .krate("foo", "0.1.0", &["bar"], "[features]\nx = [\"dep:bar\"]\n\n[dependencies]\nbar = { version = \"1\", optional = true }\n")
        .krate("foo", "0.2.0", &["baz"], "[features]\nx = [\"dep:baz\"]\n\n[dependencies]\nbaz = { version = \"1\", optional = true }\n")
        .krate("bar", "1.0.0", &[], "")
        .krate("baz", "1.0.0", &[], "");
    assert_eq!(world.names(), ["bar@1.0.0", "foo@0.1.0", "foo@0.2.0"]);
}

#[test]
fn workspace_dependencies_are_inherited_as_the_members_edition_says() {
    let root = "[workspace]\nmembers = [\"app\"]\nresolver = \"2\"\n\n[workspace.dependencies]\nitoa = \"1\"\nhelper = { path = \"helper\" }\n";
    let app = "[dependencies]\nitoa = { workspace = true, default-features = false, features = [\"more\"] }\nhelper = { workspace = true }\n";
    let itoa = "[features]\ndefault = [\"std\"]\nstd = [\"dep:std-dep\"]\nmore = [\"dep:more-dep\"]\n\n[dependencies]\nstd-dep = { version = \"1\", optional = true }\nmore-dep = { version = \"1\", optional = true }\n";
    let world = World::new(app, &["helper", "itoa"])
        .file("/w/Cargo.toml", root)
        .lock_at("/w/Cargo.lock")
        .locked("helper", "0.1.0", None, &["ryu"])
        .file("/w/helper/Cargo.toml", "[package]\nname = \"helper\"\nversion = \"0.1.0\"\n\n[dependencies]\nryu = \"1\"\n")
        .krate("itoa", "1.0.15", &["more-dep", "std-dep"], itoa)
        .krate("more-dep", "1.0.0", &[], "")
        .krate("std-dep", "1.0.0", &[], "")
        .krate("ryu", "1.0.0", &[], "");
    assert_eq!(world.names(), ["itoa@1.0.15", "more-dep@1.0.0", "ryu@1.0.0"], "2024: the member's default-features = false");
    assert_eq!(world.edition("2021").names(), ["itoa@1.0.15", "more-dep@1.0.0", "ryu@1.0.0", "std-dep@1.0.0"], "before: the workspace's");
}

#[test]
fn the_workspace_is_found_as_cargo_finds_it() {
    let world = World::new("[dependencies]\nitoa = \"1\"\n", &["itoa"]).krate("itoa", "1.0.15", &[], "");
    let excluded = world.clone().file("/w/Cargo.toml", "[workspace]\nexclude = [\"app\"]\n");
    assert_eq!(excluded.names(), ["itoa@1.0.15"], "a workspace up the tree that excludes the app");
    let member = world.clone().file("/w/Cargo.toml", "[workspace]\nresolver = \"2\"\n");
    assert!(member.error().contains("could not read /w/Cargo.lock"), "{}", member.error());
    let pointed = world
        .file(
            "/w/app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\nworkspace = \"../ws\"\n\n[dependencies]\nitoa = \"1\"\n",
        )
        .file("/w/ws/Cargo.toml", "[workspace]\nmembers = [\"../app\"]\nresolver = \"2\"\n")
        .lock_at("/w/ws/Cargo.lock");
    assert_eq!(pointed.names(), ["itoa@1.0.15"], "package.workspace points to it");
}

#[test]
fn a_patched_crate_is_read_from_its_path_and_is_no_crates_io_crate() {
    let world = World::new("[dependencies]\nitoa = \"1\"\n\n[patch.crates-io]\nitoa = { path = \"../itoa\" }\n", &["itoa"])
        .locked("itoa", "1.0.15", None, &["ryu"])
        .file("/w/itoa/Cargo.toml", "[package]\nname = \"itoa\"\nversion = \"1.0.15\"\n\n[dependencies]\nryu = \"1\"\n")
        .krate("ryu", "1.0.0", &[], "");
    assert_eq!(world.names(), ["ryu@1.0.0"]);
}

#[test]
fn a_git_crate_is_the_first_in_its_checkout_as_cargo_walks_it() {
    let source = "git+https://github.com/o/tools?branch=main#0123456789abcdef0123456789abcdef01234567";
    let checkout = "/cargo/git/checkouts/tools-1a2b3c4d5e6f7a8b/0123456";
    let tool = |deps: &str| format!("[package]\nname = \"tool\"\nversion = \"0.3.0\"\nedition = \"2024\"\n\n[dependencies]\n{deps}");
    let world = World::new("[dependencies]\ntool = { git = \"https://github.com/o/tools\", branch = \"main\" }\n", &["tool"])
        .locked("tool", "0.3.0", Some(source), &["itoa"])
        .krate("itoa", "1.0.15", &[], "")
        .file(&format!("{checkout}/Cargo.toml"), "[workspace]\nmembers = [\"u/*\"]\nresolver = \"2\"\n\n[workspace.dependencies]\nitoa = \"1\"\n")
        // Sorted before the crate, where cargo doesn't look: hidden
        // directories, git submodules, the target directory.
        .file(&format!("{checkout}/.hidden/tool/Cargo.toml"), &tool("bogus = \"1\"\n"))
        .file(&format!("{checkout}/a-sub/.git"), "gitdir: ../.git/modules/a-sub")
        .file(&format!("{checkout}/a-sub/tool/Cargo.toml"), &tool("bogus = \"1\"\n"))
        .file(&format!("{checkout}/target/package/tool/Cargo.toml"), &tool("bogus = \"1\"\n"))
        .file(&format!("{checkout}/u/tool/Cargo.toml"), &tool("itoa = { workspace = true }\n"))
        // Sorted after: the first found wins. Another revision.
        .file(&format!("{checkout}/zzz/tool/Cargo.toml"), &tool("bogus = \"1\"\n"))
        .file("/cargo/git/checkouts/tools-1a2b3c4d5e6f7a8b/0000000/u/tool/Cargo.toml", &tool("bogus = \"1\"\n"));
    assert_eq!(world.names(), ["itoa@1.0.15"]);
}

#[test]
fn a_crate_from_another_registry_is_read_from_its_own_index_and_not_listed() {
    let world = World::new("[dependencies]\nutil = { version = \"1\", registry = \"mine\" }\n", &["util"])
        .locked("util", "1.0.0", Some("sparse+https://my.host/index/"), &["itoa"])
        .file(
            "/cargo/registry/src/my.host-0a1b2c3d4e5f6a7b/util-1.0.0/Cargo.toml",
            "[package]\nname = \"util\"\nversion = \"1.0.0\"\n\n[dependencies]\nitoa = \"1\"\n",
        )
        .file(&format!("{REGISTRY}/util-1.0.0/Cargo.toml"), "[package]\nname = \"util\"\nversion = \"1.0.0\"\n\n[dependencies]\nbogus = \"1\"\n")
        .krate("itoa", "1.0.15", &[], "");
    assert_eq!(world.names(), ["itoa@1.0.15"]);
}

#[test]
fn rustflags_that_change_the_targets_cfg_are_refused() {
    let world = World::new("[dependencies]\nitoa = \"1\"\n", &["itoa"]).krate("itoa", "1.0.15", &[], "");
    assert_eq!(world.clone().env("RUSTFLAGS", "-D warnings -C link-arg=-s").names(), ["itoa@1.0.15"]);
    for (refused, says) in [
        (world.clone().env("RUSTFLAGS", "--cfg getrandom_backend=\"custom\""), "`--cfg getrandom_backend=\"custom\"` in RUSTFLAGS"),
        (world.clone().env("CARGO_ENCODED_RUSTFLAGS", "-C\x1ftarget-feature=+simd128"), "`-C target-feature=+simd128` in CARGO_ENCODED_RUSTFLAGS"),
        (world.clone().env("CARGO_TARGET_WASM32_WASIP2_RUSTFLAGS", "-Zshare-generics"), "`-Zshare-generics`"),
        (
            world.clone().file("/w/app/.cargo/config.toml", "[target.wasm32-wasip2]\nrustflags = [\"-Ctarget_cpu=mvp\"]\n"),
            "`-C target_cpu=mvp` in /w/app/.cargo/config.toml",
        ),
        (world.clone().file("/w/.cargo/config", "[build]\nrustflags = \"-O\"\n"), "`-O` in /w/.cargo/config"),
        (world.clone().file("/cargo/config.toml", "[build]\nrustflags = [\"--codegen=panic=unwind\"]\n"), "`-C panic=unwind` in /cargo/config.toml"),
    ] {
        assert!(refused.error().contains(says), "{} lacks {says}", refused.error());
    }
}

#[test]
fn what_cargo_would_build_differently_than_read_is_an_error() {
    let world = World::new("[dependencies]\nitoa = \"1\"\n", &["itoa"]);
    let no_lock = world.clone().krate("itoa", "1.0.15", &[], "").lock_at("/w/elsewhere.lock");
    assert!(no_lock.error().contains("could not read /w/app/Cargo.lock; build the app first"), "{}", no_lock.error());
    let not_downloaded = world.clone().locked("itoa", "1.0.15", Some(IO), &[]);
    assert!(not_downloaded.error().contains("itoa 1.0.15 is not in /cargo/registry/src"), "{}", not_downloaded.error());
    let not_locked = World::new("[dependencies]\nitoa = \"1\"\n", &[]);
    assert!(not_locked.error().contains("app 0.1.0 depends on itoa, which /w/app/Cargo.lock doesn't have"), "{}", not_locked.error());
    let replaced = world.krate("itoa", "1.0.15", &[], "").locked("ryu", "1.0.0", Some(IO), &[]);
    let replaced = World { lock: replaced.lock.replace("name = \"ryu\"", "replace = \"ryu 1.0.0 (git+https://x/ryu#abc)\"\nname = \"ryu\""), ..replaced };
    assert!(replaced.error().contains("with [replace]"), "{}", replaced.error());
    let resolver_1 = World::new("[dependencies]\nitoa = \"1\"\n", &["itoa"]).krate("itoa", "1.0.15", &[], "").edition("2018");
    assert!(resolver_1.error().contains("resolver 1"), "{}", resolver_1.error());
}

/// Against cargo itself, on apps on this machine:
/// APPOS_CRATES_DIRS=dir:dir cargo test -p appos --bin appos -- --ignored as_cargo_tree
#[test]
#[ignore]
fn as_cargo_tree_has_them() {
    for dir in std::env::var("APPOS_CRATES_DIRS").unwrap_or_default().split(':').filter(|d| !d.is_empty()) {
        let out = std::process::Command::new("cargo")
            .args(["tree", "--locked", "-e", "normal,no-proc-macro", "--target", TARGET, "--prefix", "none", "-f", "{p}"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "{dir}: {}", String::from_utf8_lossy(&out.stderr));
        let tree: BTreeSet<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim_end_matches(" (*)"))
            .filter(|l| !l.contains('('))
            .filter_map(|l| l.split_once(" v").map(|(n, v)| format!("pkg:cargo/{n}@{v}")))
            .collect();
        let env = |key: &str| std::env::var(key).ok();
        let ours: BTreeSet<String> = packages(&Disk, &std::path::absolute(dir).unwrap(), &env).unwrap().into_iter().collect();
        assert_eq!(ours.difference(&tree).collect::<Vec<_>>(), Vec::<&String>::new(), "{dir}: more than cargo tree");
        assert_eq!(tree.difference(&ours).collect::<Vec<_>>(), Vec::<&String>::new(), "{dir}: fewer than cargo tree");
        println!("{dir}: {} crates, as cargo tree", ours.len());
    }
}
