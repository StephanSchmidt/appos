//! `appos new` and `appos sync`: the template an app starts from, the SDK
//! it carries (src/appos.rs, src/appos.ts or src/main/java/app/Appos.java)
//! and the contract it is built against (wit/).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;
// Platform strings are untrusted terminal data.
use appos_shell::{terminal_eprintln as eprintln, terminal_println as println};

use crate::platform::{self, Config};

// TEMPLATE, TEMPLATE_TS and TEMPLATE_JAVA: every file of cli/template,
// cli/template-ts and cli/template-java but build output, data, wit/ and
// rustfmt.toml, by path (see build.rs).
include!(concat!(env!("OUT_DIR"), "/template.rs"));

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
    /// wit/. Else Rust.
    pub fn of(dir: &Path) -> Option<Lang> {
        if crate::ts::is_ts(dir) {
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

/// The contract built into this CLI, used when no platform is current.
const BUILTIN_WIT: &[(&str, &str)] = &[("appos.wit", include_str!("../../wit/appos.wit"))];

/// What the built-in shell grants. Integration point: the shell's own
/// CAPABILITIES list, once appos_shell exports it.
/// The grants an app can ask for, as this appos's shell knows them.
pub fn builtin_capabilities() -> Vec<&'static str> {
    appos_shell::api::Grant::ALL.iter().map(|g| g.as_str()).collect()
}

pub fn builtin_wit() -> BTreeMap<String, String> {
    BUILTIN_WIT.iter().map(|(name, text)| (name.to_string(), text.to_string())).collect()
}

/// The contract an app is built against.
struct Contract {
    /// WIT files by path below wit/.
    wit: BTreeMap<String, String>,
    /// The platform it is from; None: built into this CLI.
    from: Option<String>,
    /// Where that platform keeps app repositories.
    git: Option<String>,
}

/// The current platform's contract, or the one built into this CLI.
fn contract() -> anyhow::Result<Contract> {
    if let Some(p) = Config::load()?.current() {
        match platform::fetch_spec(&p.url) {
            Ok(spec) => return Ok(Contract { wit: spec.wit, from: Some(format!("platform {} ({})", p.name, spec.framework)), git: spec.git }),
            Err(e) => eprintln!("warning: {e:#}; using the built-in contract"),
        }
    }
    Ok(Contract { wit: builtin_wit(), from: None, git: None })
}

pub fn new(name: &str, group: &str, lang: Lang) -> anyhow::Result<()> {
    appos_shell::validate_app_id(name).map_err(anyhow::Error::msg)?;
    let dir = PathBuf::from(name);
    if dir.exists() {
        anyhow::bail!("{} already exists", dir.display());
    }
    let Contract { wit, from, git } = contract()?;
    let against = from.unwrap_or_else(|| "the built-in contract, fine for local development".into());
    let remote = git.map(|g| format!("{}/{name}.git", g.trim_end_matches('/')));
    let owner = git_owner();
    write_project(&dir, name, &wit, &owner, group, lang)?;

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
    // Claim the directory after network access too. create_dir_all in
    // write() alone would silently reuse a project created in the meantime.
    std::fs::create_dir(dir).with_context(|| format!("could not create {}", dir.display()))?;
    for (path, content) in files {
        write(&dir.join(path), &content)?;
    }
    for (file, text) in wit {
        write(&dir.join("wit").join(file), text)?;
    }
    Ok(())
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
                "Cargo.lock" => replace(path, text, &[("name = \"hello\"\n", format!("name = \"{name}\"\n"))])?,
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
pub fn sync() -> anyhow::Result<()> {
    let dir = std::env::current_dir()?;
    crate::app_info(&dir)?;
    let Contract { wit, from, .. } = contract()?;
    let from = from.map_or("the contract built into appos".to_string(), |p| format!("the contract of {p}"));
    let changed = sync_files(&dir, &wit)?;
    let sdk = Lang::of(&dir).map(|l| format!("{} is the SDK of this appos, ", l.sdk_path())).unwrap_or_default();
    if changed.is_empty() {
        println!("Up to date: {sdk}wit/ is {from}.");
    } else {
        println!(
            "Updated {}: {sdk}wit/ is {from}.\n\
             git diff shows what changed; make check builds and tests the app with them.",
            changed.join(", ")
        );
    }
    Ok(())
}

/// Writes the SDK and the contract into the app at `dir`; removes contract
/// files the contract doesn't have. Returns what changed.
fn sync_files(dir: &Path, wit: &BTreeMap<String, String>) -> anyhow::Result<Vec<String>> {
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
    Ok(changed)
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

/// What in the app at `dir` isn't what it builds and runs against: the SDK
/// of this appos, and `wit`, the contract of `source`.
fn stale(dir: &Path, wit: &BTreeMap<String, String>, source: &str) -> Vec<String> {
    let mut stale = Vec::new();
    if let Some(lang) = Lang::of(dir)
        && std::fs::read_to_string(dir.join(lang.sdk_path())).ok().as_deref() != Some(lang.sdk())
    {
        stale.push(format!("{} is not the one of this appos", lang.sdk_path()));
    }
    if &read_wit(dir) != wit {
        stale.push(format!("wit/ is not the contract of {source}"));
    }
    stale
}

/// Warns when the app's SDK or contract differ from those it builds and
/// runs against; `appos sync` brings both up to date.
pub fn warn_if_stale(dir: &Path, wit: &BTreeMap<String, String>, source: &str) {
    let stale = stale(dir, wit, source);
    if !stale.is_empty() {
        eprintln!("warning: {}. `appos sync` updates both (and replaces changes made to them).", stale.join(", and "));
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
                let changed = String::from_utf8_lossy(text).lines().zip(String::from_utf8_lossy(&template[path]).lines()).filter(|(a, b)| a != b).count();
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
