//! `appos new` and `appos sync`: the template an app or a kit starts from,
//! the SDK an app carries (src/appos.rs, src/appos.ts or
//! src/main/java/app/Appos.java) and the contract both are built against
//! (wit/).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;
// Platform strings are untrusted terminal data.
use appos_shell::{terminal_eprintln as eprintln, terminal_println as println};

use crate::platform::{self, Config};

// TEMPLATE, TEMPLATE_TS, TEMPLATE_JAVA and TEMPLATE_KIT: every file of
// cli/template, cli/template-ts, cli/template-java and cli/template-kit but
// build output, data, wit/ and rustfmt.toml, by path (see build.rs).
include!(concat!(env!("OUT_DIR"), "/template.rs"));

/// What `appos new` creates: an app in a language, or a kit (in Rust).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Project {
    App(Lang),
    Kit,
}

/// The language an app is written in: the template it starts from and the
/// SDK it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Rust,
    /// Compiled with scriptc; see `appos guide typescript`.
    TypeScript,
    /// Compiled with TeaVM; see `appos guide java`.
    Java,
}

impl Lang {
    /// The template of the app at `dir`: TypeScript if it has a
    /// package.json, Java if it has a pom.xml, none for a Zig app (a
    /// build.zig): it carries its own SDK, so `appos sync` updates only its
    /// wit/. None for a kit (kit.toml) too: it binds the contract itself, in
    /// src/lib.rs. Else Rust.
    pub fn of(dir: &Path) -> Option<Lang> {
        if dir.join(appos_shell::KIT_TOML).is_file() {
            None
        } else if crate::ts::is_ts(dir) {
            Some(Lang::TypeScript)
        } else if crate::java::is_java(dir) {
            Some(Lang::Java)
        } else if crate::zig::is_zig(dir) {
            None
        } else {
            Some(Lang::Rust)
        }
    }

    fn template(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::Rust => TEMPLATE,
            Lang::TypeScript => TEMPLATE_TS,
            Lang::Java => TEMPLATE_JAVA,
        }
    }

    /// Where an app carries the SDK; `appos sync` writes this CLI's copy.
    fn sdk_path(self) -> &'static str {
        match self {
            Lang::Rust => "src/appos.rs",
            Lang::TypeScript => "src/appos.ts",
            Lang::Java => "src/main/java/app/Appos.java",
        }
    }

    /// This CLI's SDK, the template's.
    fn sdk(self) -> &'static str {
        let path = self.sdk_path();
        self.template().iter().find(|(p, _)| *p == path).map(|(_, text)| *text).expect("every template has its SDK")
    }
}

/// The grants an app can ask for, as this appos's shell knows them: its
/// built-in kits'.
pub fn builtin_capabilities() -> Vec<String> {
    appos_shell::kit::Kits::builtin().names()
}

/// The contract built into this CLI, used when no platform is current.
pub fn builtin_wit() -> BTreeMap<String, String> {
    appos_shell::kit::Kits::builtin().contract()
}

/// The contract of a platform's binary (`<shell> --contract`, from
/// appos_shell::Platform::main): appos.wit and its kits' WIT.
pub fn shell_contract(shell: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let out = Command::new(shell).arg("--contract").output().with_context(|| format!("could not run {shell}"))?;
    anyhow::ensure!(out.status.success(), "{shell} --contract failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    serde_json::from_slice(&out.stdout).with_context(|| format!("{shell} --contract printed no contract; is it built with appos_shell::Platform::main?"))
}

/// The fakes of a platform binary's kits (`<shell> --fakes DIR`), by path
/// below the app's fakes/.
fn shell_fakes(shell: &str) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let dir = tempfile::tempdir()?;
    let out = Command::new(shell).arg("--fakes").arg(dir.path().join("fakes")).output().with_context(|| format!("could not run {shell}"))?;
    anyhow::ensure!(out.status.success(), "{shell} --fakes failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    let fakes = read_tree(&dir.path().join("fakes"));
    for path in fakes.keys() {
        platform::check_fake_path(path)?;
    }
    Ok(fakes)
}

/// The contract an app is built against.
struct Contract {
    /// WIT files by path below wit/.
    wit: BTreeMap<String, String>,
    /// The fakes of the platform's kits, by path below fakes/: Wasm kits
    /// `appos dev` runs in place of them.
    fakes: BTreeMap<String, Vec<u8>>,
    /// The platform it is from; None: built into this CLI.
    from: Option<String>,
    /// Where that platform keeps app repositories.
    git: Option<String>,
}

/// The current platform's contract, or the one built into this CLI.
fn contract() -> anyhow::Result<Contract> {
    if let Some(p) = Config::load()?.current() {
        match platform::fetch_spec(&p.url) {
            Ok(spec) => {
                let fakes = platform::fetch_fakes(&p.url).unwrap_or_else(|e| {
                    eprintln!("warning: {e:#}; no fakes of the platform's kits for appos dev");
                    BTreeMap::new()
                });
                return Ok(Contract { wit: spec.wit, fakes, from: Some(format!("platform {} ({})", p.name, spec.framework)), git: spec.git });
            }
            Err(e) => eprintln!("warning: {e:#}; using the built-in contract"),
        }
    }
    Ok(Contract { wit: builtin_wit(), fakes: BTreeMap::new(), from: None, git: None })
}

pub fn new(name: &str, group: &str, lang: Lang) -> anyhow::Result<()> {
    appos_shell::validate_app_id(name).map_err(anyhow::Error::msg)?;
    let dir = PathBuf::from(name);
    if dir.exists() {
        anyhow::bail!("{} already exists", dir.display());
    }
    let Contract { wit, fakes, from, git } = contract()?;
    let against = from.unwrap_or_else(|| "the built-in contract, fine for local development".into());
    let remote = git.map(|g| format!("{}/{name}.git", g.trim_end_matches('/')));
    let owner = git_owner();
    write_project(&dir, name, &wit, &owner, group, lang)?;
    sync_fakes(&dir, &fakes)?;

    let git = git_instructions(name, remote.as_deref());
    let pad = " ".repeat(name.len() + 5);
    let owner = match owner.as_str() {
        "" => "app.toml names no owner yet (git has no user.name or user.email): set\n\
               owner = \"Your Name <you@example.com>\" in it before `appos deploy`.\n\n"
            .to_string(),
        o => format!("The owner in app.toml is {o}, from git; the admin screen shows it.\n\n"),
    };
    let group = match group {
        "" => "app.toml names no group yet: set group = \"Marketing\" (your team) in it\n\
               before `appos deploy`; the admin screen groups apps by it.\n\n"
            .to_string(),
        g => format!("The group in app.toml is {g}; the admin screen groups apps by it.\n\n"),
    };

    let (kind, code) = match lang {
        Lang::Rust => (
            "",
            "The code is in src/lib.rs and src/hello/, the page in\n\
             templates/hello/index.html, the design in templates/style.css.\n\
             make check runs formatting, lints and tests.",
        ),
        Lang::TypeScript => (
            " in TypeScript",
            "The code is in src/lib.ts and src/hello/, the page in\n\
             src/hello/view.ts, the design in src/style.ts. It needs Node.js 24 or\n\
             later and zig; the first build installs scriptc, the compiler, into\n\
             node_modules/. make check builds (which type-checks) and runs the tests.",
        ),
        Lang::Java => (
            " in Java",
            "The code is in src/main/java/app/App.java and src/main/java/app/hello/,\n\
             the page in View.java there, the design in src/main/java/app/Style.java.\n\
             It needs a JDK 21 or later and Maven; the first build downloads TeaVM,\n\
             the compiler. make check builds and runs the tests (mvn test).",
        ),
    };
    println!(
        "Created {name}/ (against {against}).\n\
         It is a working hello world{kind}: change anything in it, or all of it.\n\n  \
         cd {name} && appos dev    run it at http://127.0.0.1:8080/{name}; changes reload\n  \
         appos guide{pad}    routes, the page, htmx, storage, styles, rules\n\n\
         {code}\n\n{owner}{group}{git}"
    );
    Ok(())
}

fn write_project(dir: &Path, name: &str, wit: &BTreeMap<String, String>, owner: &str, group: &str, lang: Lang) -> anyhow::Result<()> {
    let files = project_files(name, owner, group, lang)?;
    write_files(dir, files.into_iter().map(|(path, text)| (path.to_string(), text)).collect(), wit)
}

/// Writes a new project: its files, then the contract into wit/.
fn write_files(dir: &Path, files: Vec<(String, String)>, wit: &BTreeMap<String, String>) -> anyhow::Result<()> {
    // Claim the directory after network access too. create_dir_all in
    // write() alone would silently reuse a project created in the meantime.
    std::fs::create_dir(dir).with_context(|| format!("could not create {}", dir.display()))?;
    for (path, content) in files {
        write(&dir.join(path), &content)?;
    }
    for (file, text) in contract_for(dir, wit) {
        write(&dir.join("wit").join(file), &text)?;
    }
    Ok(())
}

/// `appos new --kit`: a kit in ./<name>, from cli/template-kit. <name> is
/// its id, its WIT package kit:<name> (unique, as the id is on a platform)
/// and the start of its grant, <name>-read.
pub fn new_kit(name: &str, group: &str) -> anyhow::Result<()> {
    check_kit_name(name)?;
    let dir = PathBuf::from(name);
    if dir.exists() {
        anyhow::bail!("{} already exists", dir.display());
    }
    let Contract { wit, fakes, from, .. } = contract()?;
    let against = from.unwrap_or_else(|| "the built-in contract, fine for local development".into());
    let owner = git_owner();
    write_kit(&dir, name, &wit, &owner, group)?;
    sync_fakes(&dir, &fakes)?;

    let pad = " ".repeat(name.len());
    let owner = match owner.as_str() {
        "" => "kit.toml names no owner yet (git has no user.name or user.email): set\n\
               owner = \"Your Name <you@example.com>\" in it; the admin screen shows it.\n\n"
            .to_string(),
        o => format!("The owner in kit.toml is {o}, from git; the admin screen shows it.\n\n"),
    };
    let group = match group {
        "" => "kit.toml names no group yet: set group = \"Platform\" (your team) in it.\n\n".to_string(),
        g => format!("The group in kit.toml is {g}.\n\n"),
    };
    println!(
        "Created {name}/, a kit (against {against}).\n\
         It is a working hello world: a function apps call, greet, behind the\n\
         grant {name}-read, and a page to try it. Change anything in it, or all of it.\n\n  \
         cd {name} && appos dev    run its page at http://127.0.0.1:8080/{name}; changes reload\n  \
         appos guide kits{pad}    what a kit offers, calls and shows\n\n\
         What it offers apps is in api/{name}.wit and kit.toml's [grants], the code\n\
         in src/lib.rs. wit/ is the platform's contract; appos sync updates it.\n\
         make check runs formatting, lints and tests; appos deploy puts it on\n\
         the current platform.\n\n{owner}{group}{}",
        git_instructions(name, None).replace("New app", "New kit").replace("the app", "the kit")
    );
    Ok(())
}

/// A kit's name is its id, its WIT package's name (kit:<name>) and the
/// start of its grant (<name>-read): it must make all three.
fn check_kit_name(name: &str) -> anyhow::Result<()> {
    appos_shell::validate_app_id(name).map_err(anyhow::Error::msg)?;
    appos_shell::validate_grant_name(&format!("{name}-read")).map_err(|e| anyhow::anyhow!("the kit's grant: {e}"))?;
    wit_parser::UnresolvedPackageGroup::parse("kit.wit", &format!("package kit:{name};")).map_err(|_| {
        anyhow::anyhow!("kit:{name} is no WIT package name, which a kit's name makes; a WIT word like use or list can't be one: pick another name")
    })?;
    Ok(())
}

fn write_kit(dir: &Path, name: &str, wit: &BTreeMap<String, String>, owner: &str, group: &str) -> anyhow::Result<()> {
    write_files(dir, kit_files(name, owner, group)?, wit)
}

/// The kit template's files for kit `name`: its id, crate, title, WIT
/// package, grant, owner and group put in, api/hello.wit named after it,
/// everything else as it is.
fn kit_files(name: &str, owner: &str, group: &str) -> anyhow::Result<Vec<(String, String)>> {
    let string = |s: &str| toml::Value::String(s.into()).to_string();
    let crate_name = name.replace('-', "_");
    TEMPLATE_KIT
        .iter()
        .map(|&(path, text)| {
            let file = |text: String| (path.to_string(), text);
            Ok(match path {
                "Cargo.toml" => file(replace(path, text, &[("name = \"hello\"\n", format!("name = \"{name}\"\n"))])?),
                "Cargo.lock" => file(lock_named(text, name)?),
                "kit.toml" => file(replace(
                    path,
                    text,
                    &[
                        ("id = \"hello\"\n", format!("id = {}\n", string(name))),
                        ("name = \"Hello\"\n", format!("name = {}\n", string(&title_case(name)))),
                        ("wasm = \"target/wasm32-wasip2/release/hello.wasm\"\n", format!("wasm = \"target/wasm32-wasip2/release/{crate_name}.wasm\"\n")),
                        ("fake = \"target/wasm32-wasip2/release/hello.wasm\"\n", format!("fake = \"target/wasm32-wasip2/release/{crate_name}.wasm\"\n")),
                        ("owner = \"\"\n", format!("owner = {}\n", string(owner))),
                        ("group = \"\"\n", format!("group = {}\n", string(group))),
                        ("[\"api/hello.wit\"]", format!("[\"api/{name}.wit\"]")),
                        ("[grants.hello-read]\n", format!("[grants.{name}-read]\n")),
                        ("\"kit:hello/greetings#greet\"", format!("\"kit:{name}/greetings#greet\"")),
                    ],
                )?),
                "api/hello.wit" => (format!("api/{name}.wit"), replace(path, text, &[("package kit:hello@", format!("package kit:{name}@"))])?),
                "src/lib.rs" => file(replace(
                    path,
                    text,
                    &[
                        ("export kit:hello/greetings@", format!("export kit:{name}/greetings@")),
                        ("[\"api/hello.wit\", ", format!("[\"api/{name}.wit\", ")),
                        ("exports::kit::hello::greetings::", format!("exports::kit::{crate_name}::greetings::")),
                    ],
                )?),
                _ => file(text.to_string()),
            })
        })
        .collect()
}

/// What the kit at `dir` is: its id and its own WIT packages ("kit:hello"),
/// from the files its kit.toml names. None for an app.
fn own(dir: &Path) -> Option<(String, Vec<String>)> {
    let toml: toml::Table = std::fs::read_to_string(dir.join(appos_shell::KIT_TOML)).ok()?.parse().ok()?;
    let kit = toml.get("kit")?.as_table()?;
    let id = kit.get("id")?.as_str()?.to_string();
    let files = kit.get("wit").and_then(|w| w.as_array()).into_iter().flatten().filter_map(|f| f.as_str());
    let packages = files
        .filter_map(|file| {
            let text = std::fs::read_to_string(dir.join(file)).ok()?;
            let name = wit_parser::UnresolvedPackageGroup::parse(file, &text).ok()?.main.name;
            Some(format!("{}:{}", name.namespace, name.name))
        })
        .collect();
    Some((id, packages))
}

/// The contract as the project at `dir` builds against it. A kit's own
/// packages are in its api/, maybe changed since it was deployed: they
/// leave the platform's contract, their files in deps/ and their imports
/// in appos.wit.
fn contract_for(dir: &Path, wit: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let own = own(dir).map(|(_, packages)| packages).unwrap_or_default();
    let in_deps = |path: &str| own.iter().any(|p| path.starts_with(&format!("deps/{}/", p.replace(':', "-"))));
    let imported = |line: &str| own.iter().any(|p| line.trim_start().starts_with(&format!("import {p}/")));
    wit.iter()
        .filter(|(path, _)| !in_deps(path))
        .map(|(path, text)| match path.as_str() {
            "appos.wit" => (path.clone(), text.split_inclusive('\n').filter(|line| !imported(line)).collect()),
            _ => (path.clone(), text.clone()),
        })
        .collect()
}

/// The fakes the project at `dir` runs with: a kit runs as itself, not as
/// its fake.
fn fakes_for(dir: &Path, fakes: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    let Some((id, own)) = own(dir) else { return fakes.clone() };
    let names: Vec<String> = std::iter::once(id).chain(own.iter().map(|p| p.replace(':', "-"))).collect();
    let mine = |path: &str| names.iter().any(|n| path.split('/').next() == Some(n.as_str()));
    fakes.iter().filter(|(path, _)| !mine(path)).map(|(path, bytes)| (path.clone(), bytes.clone())).collect()
}

/// The template's files for app `name`: its id, crate or package, title,
/// owner and group put in, everything else as it is.
fn project_files(name: &str, owner: &str, group: &str, lang: Lang) -> anyhow::Result<Vec<(&'static str, String)>> {
    let string = |s: &str| toml::Value::String(s.into()).to_string();
    let crate_name = name.replace('-', "_");
    let wasm = match lang {
        Lang::Rust => ("release/hello.wasm", format!("release/{crate_name}.wasm")),
        Lang::TypeScript | Lang::Java => ("target/hello.wasm", format!("target/{name}.wasm")),
    };
    lang.template()
        .iter()
        .map(|&(path, text)| {
            let text = match path {
                "Cargo.toml" => replace(path, text, &[("name = \"hello\"\n", format!("name = \"{name}\"\n"))])?,
                // The app's own entry; dependencies keep their versions.
                "Cargo.lock" => lock_named(text, name)?,
                // An app id is a valid npm package name.
                "package.json" => replace(path, text, &[("\"name\": \"hello\",\n", format!("\"name\": \"{name}\",\n"))])?,
                // An app id is a valid Maven artifactId.
                "pom.xml" => replace(path, text, &[("<artifactId>hello</artifactId>\n", format!("<artifactId>{name}</artifactId>\n"))])?,
                "app.toml" => replace(
                    path,
                    text,
                    &[
                        ("id = \"hello\"\n", format!("id = {}\n", string(name))),
                        ("name = \"Hello\"\n", format!("name = {}\n", string(&title_case(name)))),
                        wasm.clone(),
                        ("owner = \"\"\n", format!("owner = {}\n", string(owner))),
                        ("group = \"\"\n", format!("group = {}\n", string(group))),
                    ],
                )?,
                _ => text.to_string(),
            };
            Ok::<_, anyhow::Error>((path, text))
        })
        .collect()
}

/// The template's Cargo.lock with its own package named `name`, where cargo
/// keeps it (packages by name), so the first build doesn't rewrite it.
fn lock_named(text: &str, name: &str) -> anyhow::Result<String> {
    let text = replace("Cargo.lock", text, &[("name = \"hello\"\n", format!("name = \"{name}\"\n"))])?;
    let mut parts = text.split("\n[[package]]\n");
    let head = parts.next().unwrap_or_default().to_string();
    let mut packages: Vec<&str> = parts.collect();
    // Stable: two versions of a package keep their order.
    packages.sort_by_key(|p| p.lines().next().unwrap_or_default());
    Ok(packages.into_iter().fold(head, |lock, p| lock + "\n[[package]]\n" + p))
}

/// Puts each `to` in place of its `from`, which must be in the template's
/// `file` exactly once: a template edit that loses a marker fails here and
/// in the tests, instead of creating an app still named hello.
fn replace(file: &str, text: &str, pairs: &[(&str, String)]) -> anyhow::Result<String> {
    for (from, _) in pairs {
        anyhow::ensure!(text.matches(from).count() == 1, "the template's {file} must hold {from:?} exactly once");
    }
    Ok(pairs.iter().fold(text.to_string(), |text, (from, to)| text.replacen(from, to, 1)))
}

/// `appos sync`: src/appos.rs from this appos, wit/ from the current
/// platform (or this appos). Nothing else changes.
/// `shell`: take the contract from a platform's binary (shell_contract).
pub fn sync(shell: Option<&str>) -> anyhow::Result<()> {
    let dir = std::env::current_dir()?;
    crate::app_info(&dir)?;
    let Contract { wit, fakes, from, .. } = match shell {
        Some(shell) => Contract { wit: shell_contract(shell)?, fakes: shell_fakes(shell)?, from: Some(shell.to_string()), git: None },
        None => contract()?,
    };
    let from = from.map_or("the contract built into appos".to_string(), |p| format!("the contract of {p}"));
    let mut changed = sync_files(&dir, &wit)?;
    changed.extend(sync_fakes(&dir, &fakes)?);
    let sdk = Lang::of(&dir).map(|l| format!("{} is the SDK of this appos, ", l.sdk_path())).unwrap_or_default();
    if changed.is_empty() {
        println!("Up to date: {sdk}wit/ is {from}.");
    } else {
        println!(
            "Updated {}: {sdk}wit/ is {from}.\n\
             git diff shows what changed; make check builds and tests it with them.",
            changed.join(", ")
        );
    }
    Ok(())
}

/// Writes the SDK and the contract into the app at `dir`; removes contract
/// files the contract doesn't have; sets a TypeScript app's
/// verbatimModuleSyntax. Returns what changed.
fn sync_files(dir: &Path, wit: &BTreeMap<String, String>) -> anyhow::Result<Vec<String>> {
    let wit = &contract_for(dir, wit);
    let wit_dir = dir.join("wit");
    if wit_dir.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        anyhow::bail!("wit/ is a link; appos sync writes the contract into a wit/ folder");
    }
    let mut changed = Vec::new();
    if let Some(lang) = Lang::of(dir)
        && std::fs::read_to_string(dir.join(lang.sdk_path())).ok().as_deref() != Some(lang.sdk())
    {
        write(&dir.join(lang.sdk_path()), lang.sdk())?;
        changed.push(lang.sdk_path().to_string());
    }
    let had = read_wit(dir);
    for name in had.keys().filter(|name| !wit.contains_key(*name)) {
        std::fs::remove_file(wit_dir.join(name)).with_context(|| format!("could not remove wit/{name}"))?;
        changed.push(format!("wit/{name} (removed)"));
    }
    for (name, text) in wit {
        if had.get(name) != Some(text) {
            write(&wit_dir.join(name), text)?;
            changed.push(format!("wit/{name}"));
        }
    }
    if Lang::of(dir) == Some(Lang::TypeScript)
        && let Ok(text) = std::fs::read_to_string(dir.join("tsconfig.json"))
        && let Some(set) = crate::deps::tsconfig::with_verbatim_module_syntax(&text)?
    {
        write(&dir.join("tsconfig.json"), &set)?;
        changed.push("tsconfig.json (verbatimModuleSyntax)".to_string());
    }
    Ok(changed)
}

/// Puts `fakes` into the app's fakes/, replacing what it held; answers
/// which kits' fakes changed.
fn sync_fakes(dir: &Path, fakes: &BTreeMap<String, Vec<u8>>) -> anyhow::Result<Vec<String>> {
    let fakes = &fakes_for(dir, fakes);
    let fakes_dir = dir.join("fakes");
    let had = read_tree(&fakes_dir);
    if &had == fakes {
        return Ok(vec![]);
    }
    let kit = |path: &String| path.split('/').next().unwrap_or_default().to_string();
    let kits: std::collections::BTreeSet<String> = had.keys().chain(fakes.keys()).map(kit).collect();
    let mut changed = vec![];
    for name in kits {
        let of = |files: &BTreeMap<String, Vec<u8>>| files.iter().filter(|(p, _)| kit(p) == name).map(|(p, b)| (p.clone(), b.clone())).collect::<Vec<_>>();
        match (of(&had), of(fakes)) {
            (was, now) if was == now => {}
            (_, now) if now.is_empty() => changed.push(format!("fakes/{name} (removed)")),
            _ => changed.push(format!("fakes/{name}")),
        }
    }
    if fakes_dir.exists() {
        std::fs::remove_dir_all(&fakes_dir).with_context(|| format!("could not remove {}", fakes_dir.display()))?;
    }
    for (path, bytes) in fakes {
        let file = fakes_dir.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file, bytes).with_context(|| format!("could not write {}", file.display()))?;
    }
    Ok(changed)
}

/// Every file below `root`, by path below it.
fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, depth: u32, files: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() && depth < 8 {
                walk(root, &path, depth + 1, files);
            } else if let (Ok(rel), Ok(bytes)) = (path.strip_prefix(root), std::fs::read(&path)) {
                files.insert(rel.to_string_lossy().replace('\\', "/"), bytes);
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, 0, &mut files);
    files
}

/// The .wit files below the app's wit/, by path below it.
fn read_wit(dir: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, depth: u32, files: &mut BTreeMap<String, String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() && depth < 8 {
                walk(root, &path, depth + 1, files);
            } else if path.extension().is_some_and(|e| e == "wit")
                && let (Ok(rel), Ok(text)) = (path.strip_prefix(root), std::fs::read_to_string(&path))
            {
                files.insert(rel.to_string_lossy().replace('\\', "/"), text);
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(&dir.join("wit"), &dir.join("wit"), 0, &mut files);
    files
}

/// What in the app or kit at `dir` isn't what it builds and runs against:
/// the SDK of this appos, `wit`, the contract of `source` (for a kit,
/// without its own packages, and only if it builds against it: a sign-in
/// kit's wit/ is its own), and a TypeScript app's verbatimModuleSyntax,
/// without which it gets no deps.json.
fn stale(dir: &Path, wit: &BTreeMap<String, String>, source: &str) -> Vec<String> {
    let mut stale = Vec::new();
    if let Some(lang) = Lang::of(dir)
        && std::fs::read_to_string(dir.join(lang.sdk_path())).ok().as_deref() != Some(lang.sdk())
    {
        stale.push(format!("{} is not the one of this appos", lang.sdk_path()));
    }
    let against = own(dir).is_none() || dir.join("wit").join("appos.wit").is_file();
    if against && read_wit(dir) != contract_for(dir, wit) {
        stale.push(format!("wit/ is not the contract of {source}"));
    }
    if Lang::of(dir) == Some(Lang::TypeScript)
        && let Ok(text) = std::fs::read_to_string(dir.join("tsconfig.json"))
        && crate::deps::tsconfig::sets_verbatim_module_syntax(&text).is_ok_and(|set| !set)
    {
        stale.push("tsconfig.json doesn't set verbatimModuleSyntax".to_string());
    }
    stale
}

/// Warns when the SDK or contract of the app or kit at `dir` differ from
/// those it builds and runs against; `appos sync` brings both up to date.
pub fn warn_if_stale(dir: &Path, wit: &BTreeMap<String, String>, source: &str) {
    let stale = stale(dir, wit, source);
    if !stale.is_empty() {
        eprintln!("warning: {}. `appos sync` brings them up to date (and replaces changes made to the SDK and wit/).", stale.join(", and "));
    }
}

/// The owner `appos new` writes into app.toml: "Name <email>" from git's
/// user.name and user.email, as far as they are set.
fn git_owner() -> String {
    let get = |key: &str| {
        let out = Command::new("git").args(["config", "--get", key]).output().ok().filter(|o| o.status.success())?;
        let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!value.is_empty()).then_some(value)
    };
    match (get("user.name"), get("user.email")) {
        (Some(name), Some(mail)) => format!("{name} <{mail}>"),
        (one, other) => one.or(other).unwrap_or_default(),
    }
}

/// Tells the user (or their agent) to keep the app in git. appos doesn't
/// init or commit itself.
fn git_instructions(name: &str, remote: Option<&str>) -> String {
    let mut text = format!(
        "Keep the app in git, so every working state is saved and can be restored:\n  \
         cd {name} && git init -b main && git add -A && git commit -m \"New app\"\n"
    );
    if let Some(url) = remote {
        // A platform supplies this value. Copying the suggested command
        // must not evaluate substitutions or split it into shell words.
        let url = format!("'{}'", url.replace('\'', "'\"'\"'"));
        text.push_str(&format!(
            "  git remote add -- origin {url} && git push -u origin main\n  \
             (this platform keeps app repositories there; the repository may need to be created first)\n"
        ));
    }
    text.push_str("Then commit every time the app works after a change: git add -A && git commit -m \"what changed\"");
    text
}

fn write(file: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, content).with_context(|| format!("could not write {}", file.display()))
}

fn title_case(id: &str) -> String {
    id.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every file below `dir` by path, as bytes; without the directories
    /// build.rs leaves out at the top.
    fn files_below(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
            for path in std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
                let rel = path.strip_prefix(root).unwrap().to_str().unwrap().to_string();
                if dir == root && ["target", "appos-data", "wit", "rustfmt.toml", "node_modules", "bindings"].contains(&rel.as_str()) {
                    continue;
                }
                if path.is_dir() {
                    walk(root, &path, files);
                } else {
                    files.insert(rel, std::fs::read(&path).unwrap());
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(dir, dir, &mut files);
        files
    }

    const LANGS: [Lang; 3] = [Lang::Rust, Lang::TypeScript, Lang::Java];

    fn template_dir(lang: Lang) -> PathBuf {
        let dir = match lang {
            Lang::Rust => "template",
            Lang::TypeScript => "template-ts",
            Lang::Java => "template-java",
        };
        Path::new(env!("CARGO_MANIFEST_DIR")).join(dir)
    }

    /// With the template's own name and no owner or group, `appos new`
    /// writes the template byte for byte: every file, and only those.
    #[test]
    fn a_new_hello_is_the_template() {
        for lang in LANGS {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("hello");
            write_project(&target, "hello", &builtin_wit(), "", "", lang).unwrap();
            assert_eq!(files_below(&target), files_below(&template_dir(lang)), "{lang:?}");
            assert_eq!(read_wit(&target), builtin_wit());
            assert_eq!(read_wit(&template_dir(lang)), builtin_wit(), "the {lang:?} template's wit/ is not the built-in contract");
            assert_eq!(Lang::of(&target), Some(lang));
        }
    }

    /// Another app differs from the template only in the lines with its
    /// name, title, owner and group.
    #[test]
    fn a_new_app_changes_only_its_names_owner_and_group() {
        for (lang, wasm) in
            [(Lang::Rust, "target/wasm32-wasip2/release/lead_form.wasm"), (Lang::TypeScript, "target/lead-form.wasm"), (Lang::Java, "target/lead-form.wasm")]
        {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("lead-form");
            write_project(&target, "lead-form", &builtin_wit(), "Jane Doe <jane@example.com>", "Marketing", lang).unwrap();
            let (new, template) = (files_below(&target), files_below(&template_dir(lang)));
            assert_eq!(new.keys().collect::<Vec<_>>(), template.keys().collect::<Vec<_>>());
            for (path, text) in &new {
                let changed = changed_lines(text, &template[path]);
                let expected = match path.as_str() {
                    "Cargo.toml" | "Cargo.lock" | "package.json" | "pom.xml" => 1,
                    "app.toml" => 5,
                    _ => 0,
                };
                assert_eq!(changed, expected, "{lang:?} {path}");
            }
            let manifest: toml::Table = std::fs::read_to_string(target.join("app.toml")).unwrap().parse().unwrap();
            assert_eq!(manifest["app"]["id"].as_str(), Some("lead-form"));
            assert_eq!(manifest["app"]["name"].as_str(), Some("Lead Form"));
            assert_eq!(manifest["app"]["wasm"].as_str(), Some(wasm));
            assert_eq!(manifest["app"]["owner"].as_str(), Some("Jane Doe <jane@example.com>"));
        }
    }

    /// The lines of `new` that `template` doesn't have, wherever they are.
    fn changed_lines(new: &[u8], template: &[u8]) -> usize {
        let mut left: Vec<&str> = std::str::from_utf8(template).unwrap().lines().collect();
        std::str::from_utf8(new)
            .unwrap()
            .lines()
            .filter(|line| match left.iter().position(|l| l == line) {
                Some(i) => {
                    left.remove(i);
                    false
                }
                None => true,
            })
            .count()
    }

    /// Cargo keeps a lock's packages by name; renamed, the app's or kit's
    /// own moves there, or the first build rewrites the lock.
    #[test]
    fn a_new_lock_keeps_cargos_order() {
        for template in [TEMPLATE, TEMPLATE_KIT] {
            let lock = template.iter().find(|(p, _)| *p == "Cargo.lock").unwrap().1;
            for name in ["aa", "hello", "lead-form", "zz"] {
                let named = lock_named(lock, name).unwrap();
                let names: Vec<&str> = named.lines().filter(|l| l.starts_with("name = ")).collect();
                assert!(names.is_sorted(), "{name}");
                assert!(names.contains(&format!("name = \"{name}\"").as_str()));
                assert_eq!(named.len(), lock.len() + name.len() - "hello".len());
            }
        }
    }

    fn kit_template() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("template-kit")
    }

    /// With the template's own name and no owner or group, `appos new
    /// --kit` writes the kit template byte for byte, and the shell reads
    /// its kit.toml.
    #[test]
    fn a_new_hello_kit_is_the_template() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("hello");
        write_kit(&target, "hello", &builtin_wit(), "", "").unwrap();
        assert_eq!(files_below(&target), files_below(&kit_template()));
        assert_eq!(read_wit(&target), builtin_wit());
        assert_eq!(read_wit(&kit_template()), builtin_wit(), "the kit template's wit/ is not the built-in contract");
        assert_eq!(Lang::of(&target), None, "a kit carries no SDK");
        assert_eq!(own(&target), Some(("hello".to_string(), vec!["kit:hello".to_string()])));
        let (m, _, wasm) = appos_shell::Manifest::load(&target).unwrap();
        assert!(m.is_kit());
        assert_eq!(wasm, target.join("target/wasm32-wasip2/release/hello.wasm"));
    }

    /// Another kit differs from the template only in its names, owner and
    /// group, and its WIT file is named after it.
    #[test]
    fn a_new_kit_changes_only_its_names_owner_and_group() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("lead-score");
        write_kit(&target, "lead-score", &builtin_wit(), "Jane Doe <jane@example.com>", "Platform").unwrap();
        let (new, template) = (files_below(&target), files_below(&kit_template()));
        let renamed = |p: &String| if p == "api/hello.wit" { "api/lead-score.wit".to_string() } else { p.clone() };
        assert_eq!(
            new.keys().cloned().collect::<Vec<_>>(),
            template.keys().map(renamed).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>()
        );
        for (path, text) in &template {
            let changed = changed_lines(&new[&renamed(path)], text);
            let expected = match path.as_str() {
                "Cargo.toml" | "Cargo.lock" | "api/hello.wit" => 1,
                "src/lib.rs" => 3,
                "kit.toml" => 9,
                _ => 0,
            };
            assert_eq!(changed, expected, "{path}");
        }
        let kit: toml::Table = std::fs::read_to_string(target.join("kit.toml")).unwrap().parse().unwrap();
        assert_eq!(kit["kit"]["id"].as_str(), Some("lead-score"));
        assert_eq!(kit["kit"]["name"].as_str(), Some("Lead Score"));
        assert_eq!(kit["kit"]["owner"].as_str(), Some("Jane Doe <jane@example.com>"));
        assert_eq!(kit["kit"]["group"].as_str(), Some("Platform"));
        assert_eq!(kit["kit"]["fake"].as_str(), Some("target/wasm32-wasip2/release/lead_score.wasm"));
        assert_eq!(kit["grants"]["lead-score-read"]["functions"][0].as_str(), Some("kit:lead-score/greetings#greet"));
        assert_eq!(own(&target), Some(("lead-score".to_string(), vec!["kit:lead-score".to_string()])));
        appos_shell::Manifest::load(&target).unwrap();
    }

    /// A kit's name is its id, its WIT package and the start of its grant.
    #[test]
    fn a_kits_name_must_make_an_id_a_package_and_a_grant() {
        for name in ["scoring", "lead-score", "crm2", "events-2026", "a"] {
            assert!(check_kit_name(name).is_ok(), "{name}");
        }
        for name in ["2fa", "Scoring", "lead_score", "admin", "use", "list", "", &"a".repeat(56)] {
            assert!(check_kit_name(name).is_err(), "{name:?}");
            assert!(new_kit(name, "").is_err(), "{name:?}");
        }
    }

    /// Once a kit is deployed, the platform's contract has its package and
    /// its fake too. The kit has its own in api/, maybe changed: sync
    /// leaves them out, and the kit isn't stale for it.
    #[test]
    fn a_kit_builds_against_the_platform_without_its_own_package() {
        let root = tempfile::tempdir().unwrap();
        let kit = root.path().join("scoring");
        write_kit(&kit, "scoring", &builtin_wit(), "", "").unwrap();
        let marker = "    // The platform's kits: appos new and appos sync add their interfaces.\n";
        let mut platform = builtin_wit();
        let appos =
            platform["appos.wit"].replacen(marker, &format!("{marker}    import acme:crm/customers@0.1.0;\n    import kit:scoring/greetings@0.1.0;\n"), 1);
        platform.insert("appos.wit".into(), appos);
        platform.insert("deps/acme-crm/crm.wit".into(), "package acme:crm@0.1.0;\ninterface customers {}\n".into());
        platform.insert("deps/kit-scoring/scoring.wit".into(), "package kit:scoring@0.1.0;\ninterface greetings {}\n".into());

        assert_eq!(stale(&kit, &platform, "acme"), ["wit/ is not the contract of acme"]);
        sync_files(&kit, &platform).unwrap();
        let wit = read_wit(&kit);
        assert!(wit.contains_key("deps/acme-crm/crm.wit"));
        assert!(!wit.contains_key("deps/kit-scoring/scoring.wit"));
        assert!(wit["appos.wit"].contains("    import acme:crm/customers@0.1.0;\n"));
        assert!(!wit["appos.wit"].contains("kit:scoring"));
        assert_eq!(stale(&kit, &platform, "acme"), Vec::<String>::new());
        assert_eq!(sync_files(&kit, &platform).unwrap(), Vec::<String>::new());

        let fakes: BTreeMap<String, Vec<u8>> =
            ["acme-crm/kit.toml", "acme-crm/kit.wasm", "scoring/kit.toml", "scoring/kit.wasm"].iter().map(|p| (p.to_string(), b"x".to_vec())).collect();
        assert_eq!(sync_fakes(&kit, &fakes).unwrap(), ["fakes/acme-crm"]);
        assert_eq!(read_tree(&kit.join("fakes")).keys().collect::<Vec<_>>(), ["acme-crm/kit.toml", "acme-crm/kit.wasm"]);
        // An app keeps all of it.
        let app = root.path().join("events");
        write_project(&app, "events", &platform, "", "", Lang::Rust).unwrap();
        assert_eq!(read_wit(&app), platform);
    }

    #[test]
    fn owners_and_groups_are_toml_strings() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("events");
        write_project(&target, "events", &BTreeMap::new(), "Jo \"JJ\" Doe", "Sales \"EMEA\"\ngroup = \"\"", Lang::Rust).unwrap();
        let manifest: toml::Table = std::fs::read_to_string(target.join("app.toml")).unwrap().parse().unwrap();
        assert_eq!(manifest["app"]["owner"].as_str(), Some("Jo \"JJ\" Doe"));
        assert_eq!(manifest["app"]["group"].as_str(), Some("Sales \"EMEA\"\ngroup = \"\""));
    }

    #[test]
    fn a_template_marker_that_is_not_there_once_fails_loudly() {
        assert!(replace("app.toml", "id = \"x\"\n", &[("id = \"hello\"\n", "id = \"y\"\n".into())]).is_err());
        assert!(replace("app.toml", "a\na\n", &[("a\n", "b\n".into())]).is_err());
        assert_eq!(replace("app.toml", "a\nc\n", &[("a\n", "b\n".into())]).unwrap(), "b\nc\n");
    }

    #[test]
    fn project_creation_never_overwrites_a_directory_created_while_fetching_the_platform() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("events");
        // The early existence check passed; another process now owns it.
        assert!(!target.exists());
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("Cargo.toml"), b"user's project").unwrap();
        let result = write_project(&target, "events", &BTreeMap::new(), "Owner", "", Lang::Rust);
        assert!(result.is_err(), "scaffolding overwrote a pre-existing project");
        assert_eq!(std::fs::read(target.join("Cargo.toml")).unwrap(), b"user's project");
    }

    #[test]
    fn new_projects_cannot_claim_shell_routes() {
        for name in ["assets", "admin", "Events", ""] {
            assert!(new(name, "", Lang::Rust).is_err(), "the CLI created an app the shell refuses: {name:?}");
        }
    }

    #[test]
    fn sync_puts_the_platforms_fakes_into_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let files = |pairs: &[(&str, &str)]| pairs.iter().map(|(p, b)| (p.to_string(), b.as_bytes().to_vec())).collect::<BTreeMap<_, _>>();
        let crm = files(&[("acme-crm/kit.toml", "[kit]"), ("acme-crm/kit.wasm", "wasm"), ("acme-crm/wit/crm.wit", "package acme:crm;")]);
        assert_eq!(sync_fakes(dir.path(), &BTreeMap::new()).unwrap(), Vec::<String>::new());
        assert!(!dir.path().join("fakes").exists(), "no fakes, no folder");
        assert_eq!(sync_fakes(dir.path(), &crm).unwrap(), ["fakes/acme-crm"]);
        assert_eq!(read_tree(&dir.path().join("fakes")), crm);
        assert_eq!(sync_fakes(dir.path(), &crm).unwrap(), Vec::<String>::new());
        let erp = files(&[("acme-erp/kit.toml", "[kit]"), ("acme-erp/kit.wasm", "wasm")]);
        assert_eq!(sync_fakes(dir.path(), &erp).unwrap(), ["fakes/acme-crm (removed)", "fakes/acme-erp"]);
        assert_eq!(read_tree(&dir.path().join("fakes")), erp);
    }

    #[test]
    fn sync_brings_the_sdk_and_the_contract_up_to_date_and_nothing_else() {
        for (lang, code) in [(Lang::Rust, "src/lib.rs"), (Lang::TypeScript, "src/lib.ts"), (Lang::Java, "src/main/java/app/App.java")] {
            let root = tempfile::tempdir().unwrap();
            let app = root.path().join("events");
            write_project(&app, "events", &builtin_wit(), "", "", lang).unwrap();
            assert!(stale(&app, &builtin_wit(), "x").is_empty());
            assert!(sync_files(&app, &builtin_wit()).unwrap().is_empty());

            let sdk = lang.sdk_path();
            std::fs::write(app.join(sdk), "// changed").unwrap();
            std::fs::write(app.join("wit/old.wit"), "package old:old;").unwrap();
            std::fs::write(app.join(code), "// mine").unwrap();
            let platform: BTreeMap<String, String> = [("appos.wit", "package appos:framework;"), ("deps/types.wit", "package appos:types;")]
                .into_iter()
                .map(|(n, t)| (n.to_string(), t.to_string()))
                .collect();
            assert_eq!(stale(&app, &platform, "x").len(), 2);
            let changed = sync_files(&app, &platform).unwrap();
            assert_eq!(changed, [sdk, "wit/old.wit (removed)", "wit/appos.wit", "wit/deps/types.wit"]);
            assert!(stale(&app, &platform, "x").is_empty());
            assert_eq!(std::fs::read_to_string(app.join(sdk)).unwrap(), lang.sdk());
            assert_eq!(std::fs::read_to_string(app.join(code)).unwrap(), "// mine");
            // Another language's SDK never lands in the app.
            for other in LANGS.into_iter().filter(|&other| other != lang) {
                assert!(!app.join(other.sdk_path()).exists(), "{lang:?} got {other:?}'s SDK");
            }
        }
    }

    #[test]
    fn sync_sets_verbatim_module_syntax_in_an_older_typescript_app() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("events");
        write_project(&app, "events", &builtin_wit(), "", "", Lang::TypeScript).unwrap();
        let tsconfig = app.join("tsconfig.json");
        let template = std::fs::read_to_string(&tsconfig).unwrap();
        let older = template.replace("    \"verbatimModuleSyntax\": true,\n", "");
        assert_ne!(older, template);
        std::fs::write(&tsconfig, &older).unwrap();
        assert_eq!(stale(&app, &builtin_wit(), "x"), ["tsconfig.json doesn't set verbatimModuleSyntax"]);
        assert_eq!(sync_files(&app, &builtin_wit()).unwrap(), ["tsconfig.json (verbatimModuleSyntax)"]);
        let synced = std::fs::read_to_string(&tsconfig).unwrap();
        assert!(crate::deps::tsconfig::sets_verbatim_module_syntax(&synced).unwrap(), "{synced}");
        assert_eq!(synced.lines().count(), template.lines().count(), "one line more, and nothing else");
        assert!(stale(&app, &builtin_wit(), "x").is_empty());
        assert!(sync_files(&app, &builtin_wit()).unwrap().is_empty());

        std::fs::write(&tsconfig, "{ broken").unwrap();
        assert!(sync_files(&app, &builtin_wit()).is_err(), "a tsconfig.json that isn't JSON is the app's to fix");
        std::fs::remove_file(&tsconfig).unwrap();
        assert!(sync_files(&app, &builtin_wit()).unwrap().is_empty(), "nor is a missing one sync's to write");
    }

    /// The examples are apps like any other: they carry this SDK and this
    /// contract, byte for byte (cp cli/template/src/appos.rs and wit/ after
    /// changing them).
    #[test]
    fn every_example_carries_this_sdk_and_contract() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
        let mut seen = 0;
        for dir in std::fs::read_dir(examples).unwrap().map(|e| e.unwrap().path()).filter(|d| d.join("app.toml").exists()) {
            assert_eq!(stale(&dir, &builtin_wit(), "this appos"), Vec::<String>::new(), "{}", dir.display());
            seen += 1;
        }
        assert!(seen >= 4, "found {seen} examples");
    }

    #[test]
    fn git_instructions_quote_the_platforms_remote_as_one_literal_argument() {
        for remote in [
            "https://git.example/test.git",
            "https://git.example/$(printf injected).git",
            "https://git.example/`printf injected`.git",
            "https://git.example/a'b c.git",
        ] {
            let instructions = git_instructions("test", Some(remote));
            let line = instructions.lines().find(|line| line.contains("git remote add ")).unwrap();
            // Capture arguments with a fake git; no repository or network
            // is touched, even when testing command-substitution payloads.
            let script = format!("git() {{ for arg; do [ \"$arg\" = -- ] || printf '%s\\n' \"$arg\"; done; }}\n{line}");
            let output = Command::new("sh").args(["-c", &script]).output().unwrap();
            assert!(output.status.success(), "remote {remote:?}");
            assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("remote\nadd\norigin\n{remote}\npush\n-u\norigin\nmain\n"));
        }
    }
}
