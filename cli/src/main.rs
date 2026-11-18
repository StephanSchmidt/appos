//! `appos`: build apps for an AppOS shell. The help text is the
//! documentation; an agent told "use the appos cli" should need nothing else.
//!
//!   main.rs       the commands and their arguments, app.toml
//!   help.rs       the help texts and the guide
//!   args.rs       how every command reads its arguments
//!   scaffold.rs   appos new and appos sync: the template, the SDK, the contract
//!   dev.rs        appos build and appos dev
//!   review.rs     appos review: appfuzz's files, from what the shell knows
//!   commands.rs   the commands that talk to a platform
//!   audit.rs      appos audit: the live apps' packages against OSV
//!   deps/         deps.json: the packages compiled into an app, per language
//!   platform.rs   the platform API and the platforms this CLI knows
//!   sources.rs    what appos dev watches

mod args;
mod audit;
mod commands;
mod deps;
mod dev;
mod help;
mod java;
mod platform;
mod review;
mod scaffold;
mod sources;
mod ts;
mod zig;

use std::path::{Path, PathBuf};

// Platform strings and stored logs are untrusted terminal data.
use appos_shell::{terminal_eprintln as eprintln, terminal_print as print};
use args::Spec;

const GUIDE: Spec = Spec { positional: 1, ..Spec::bare("appos guide [section]") };
const NEW: Spec = Spec {
    required: 1,
    positional: 1,
    switches: &["--ts", "--java"],
    options: &["--group"],
    ..Spec::bare("appos new <name> [--group <team>] [--ts | --java]")
};
const DEV: Spec = Spec { options: &["--addr"], ..Spec::bare("appos dev [--addr 127.0.0.1:8080]") };
const LOGIN: Spec = Spec { required: 1, positional: 1, ..Spec::bare("appos login <url>") };
const USE: Spec = Spec { required: 1, positional: 1, ..Spec::bare("appos use <name>") };
const HOSTS: Spec = Spec { switches: &["--wit"], ..Spec::bare("appos hosts [--wit]") };
const PACK: Spec = Spec { positional: 1, ..Spec::bare("appos pack [file]") };
const DEPLOY: Spec = Spec { positional: 1, ..Spec::bare("appos deploy [app.zip]") };
const LOG: Spec = Spec { switches: &["--failures"], options: &["--before"], ..Spec::bare("appos log [--failures] [--before <n>]") };
const ROLLBACK: Spec = Spec { positional: 1, options: &["--reason"], ..Spec::bare("appos rollback [version] [--reason <text>]") };
const OFF: Spec = Spec { options: &["--reason"], ..Spec::bare("appos off [--reason <text>]") };
const ON: Spec = Spec { options: &["--reason"], ..Spec::bare("appos on [--reason <text>]") };
const AUDIT: Spec = Spec { switches: &["--json"], ..Spec::bare("appos audit [--json]") };

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str);
    let rest = args.get(1..).unwrap_or(&[]);
    let wants_help = rest.iter().any(|a| a == "-h" || a == "--help");

    let result = match command {
        None | Some("help" | "-h" | "--help") => {
            print!("{}", help::HELP);
            Ok(())
        }
        Some("-V" | "--version") => {
            print!("appos {}\n", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(cmd) if wants_help => match help::command(cmd) {
            Some(text) => {
                print!("{text}");
                Ok(())
            }
            None => Err(unknown(cmd)),
        },
        Some(cmd) => run(cmd, rest).await,
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

/// Runs a command with the arguments after it.
async fn run(cmd: &str, rest: &[String]) -> anyhow::Result<()> {
    match cmd {
        "guide" => help::guide(GUIDE.parse(rest)?.first()),
        "new" => {
            let (name, group, lang) = new_args(rest)?;
            scaffold::new(name, group, lang)
        }
        "sync" => {
            Spec::bare("appos sync").parse(rest)?;
            scaffold::sync()
        }
        "build" => {
            Spec::bare("appos build").parse(rest)?;
            dev::build().map(drop)
        }
        "dev" => dev::dev(DEV.parse(rest)?.option("--addr")).await,
        "review" => {
            Spec::bare("appos review").parse(rest)?;
            review::review().await
        }
        "login" => commands::login(LOGIN.parse(rest)?.first().unwrap_or_default()),
        "logout" => {
            Spec::bare("appos logout").parse(rest)?;
            commands::logout()
        }
        "platforms" => {
            Spec::bare("appos platforms").parse(rest)?;
            commands::platforms()
        }
        "use" => commands::use_platform(USE.parse(rest)?.first().unwrap_or_default()),
        "hosts" => commands::hosts(HOSTS.parse(rest)?.switch("--wit")),
        "pack" => commands::pack(PACK.parse(rest)?.first()),
        "deploy" => commands::deploy(DEPLOY.parse(rest)?.first()),
        "versions" => {
            Spec::bare("appos versions").parse(rest)?;
            commands::versions()
        }
        "log" => {
            let (failures, before) = log_args(rest)?;
            commands::log(failures, before)
        }
        "rollback" => {
            let args = ROLLBACK.parse(rest)?;
            commands::rollback(args.first(), args.option("--reason"))
        }
        "off" => commands::turn(false, OFF.parse(rest)?.option("--reason")),
        "on" => commands::turn(true, ON.parse(rest)?.option("--reason")),
        "audit" => audit::audit(AUDIT.parse(rest)?.switch("--json")),
        other => Err(unknown(other)),
    }
}

fn unknown(cmd: &str) -> anyhow::Error {
    anyhow::anyhow!("unknown command {cmd:?}; run `appos` for help")
}

/// `appos new <name> [--group <team>] [--ts | --java]`: the name, the group
/// ("" if none) and the language.
fn new_args(rest: &[String]) -> anyhow::Result<(&str, &str, scaffold::Lang)> {
    let args = NEW.parse(rest)?;
    let group = match args.option("--group").map(str::trim) {
        Some("") => anyhow::bail!("--group needs a team, like Marketing; usage: {}", NEW.usage),
        group => group.unwrap_or_default(),
    };
    let lang = match (args.switch("--ts"), args.switch("--java")) {
        (true, true) => anyhow::bail!("--ts or --java, not both; usage: {}", NEW.usage),
        (true, false) => scaffold::Lang::TypeScript,
        (false, true) => scaffold::Lang::Java,
        (false, false) => scaffold::Lang::Rust,
    };
    Ok((args.first().unwrap_or_default(), group, lang))
}

/// `appos log [--failures] [--before <n>]`.
fn log_args(rest: &[String]) -> anyhow::Result<(bool, Option<i64>)> {
    let args = LOG.parse(rest)?;
    let before = match args.option("--before") {
        Some(n) => Some(n.parse().map_err(|_| anyhow::anyhow!("--before {n:?} is not a number; usage: {}", LOG.usage))?),
        None => None,
    };
    Ok((args.switch("--failures"), before))
}

/// The app id and .wasm path from app.toml in `dir`, checked the way the
/// shell checks it, so a mistake shows before the build.
fn app_info(dir: &Path) -> anyhow::Result<(String, PathBuf)> {
    if !dir.join("app.toml").is_file() {
        anyhow::bail!("no app.toml here; run this in an app's directory, or create one with `appos new <name>`");
    }
    let (m, _, wasm) = appos_shell::Manifest::load(dir).map_err(|e| anyhow::anyhow!("app.toml: {}", e.root_cause()))?;
    Ok((m.app.id, wasm))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn new_takes_a_name_a_group_and_a_language_in_any_order() {
        use scaffold::Lang::{Java, Rust, TypeScript};
        assert_eq!(new_args(&args(&["events"])).unwrap(), ("events", "", Rust));
        assert_eq!(new_args(&args(&["events", "--group", "Marketing"])).unwrap(), ("events", "Marketing", Rust));
        assert_eq!(new_args(&args(&["--group", "Customer service", "events"])).unwrap(), ("events", "Customer service", Rust));
        assert_eq!(new_args(&args(&["--group=Sales", "events"])).unwrap(), ("events", "Sales", Rust));
        assert_eq!(new_args(&args(&["--ts", "events", "--group", "Sales"])).unwrap(), ("events", "Sales", TypeScript));
        assert_eq!(new_args(&args(&["events", "--java"])).unwrap(), ("events", "", Java));
        for bad in [
            &["--group"][..],
            &["events", "--group", " "],
            &["a", "b"],
            &["--grp", "x", "events"],
            &[],
            &["a", "--group", "x", "--group", "y"],
            &["a", "--ts", "--ts"],
            &["a", "--ts=yes"],
            &["a", "--ts", "--java"],
        ] {
            assert!(new_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rollback_takes_a_version_and_a_reason_in_any_order() {
        let parse = |a: &[&str]| {
            let rest = args(a);
            ROLLBACK.parse(&rest).map(|p| (p.first().map(String::from), p.option("--reason").map(String::from)))
        };
        let both = (Some("0.1.0-ab".to_string()), Some("loses leads".to_string()));
        assert_eq!(parse(&[]).unwrap(), (None, None));
        assert_eq!(parse(&["0.1.0-ab", "--reason", "loses leads"]).unwrap(), both);
        assert_eq!(parse(&["--reason=loses leads", "0.1.0-ab"]).unwrap(), both);
        for bad in [&["a", "b"][..], &["--reason"], &["--reason", "x", "--reason", "y"], &["--force"], &["--reasonable"]] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn log_takes_failures_and_a_numeric_before() {
        assert_eq!(log_args(&args(&[])).unwrap(), (false, None));
        assert_eq!(log_args(&args(&["--before", "42", "--failures"])).unwrap(), (true, Some(42)));
        for bad in [&["--before"][..], &["--before", "soon"], &["--failures=yes"], &["3"]] {
            assert!(log_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn every_command_has_help() {
        for cmd in [
            "new",
            "sync",
            "dev",
            "build",
            "review",
            "guide",
            "login",
            "logout",
            "platforms",
            "use",
            "hosts",
            "deploy",
            "pack",
            "versions",
            "log",
            "rollback",
            "off",
            "on",
            "audit",
        ] {
            assert!(help::command(cmd).is_some_and(|h| h.lines().take(2).any(|l| l.starts_with(&format!("appos {cmd}")))), "{cmd}");
        }
    }

    #[test]
    fn app_info_rejects_ids_that_can_change_a_platform_request_path_or_query() {
        let root = tempfile::tempdir().unwrap();
        for id in ["../../apps/other", "other/off?unused=", "test&app=other", "admin"] {
            std::fs::write(root.path().join("app.toml"), format!("[app]\nid = '{id}'\nname = 'X'\nversion = '1.0'\n")).unwrap();
            assert!(app_info(root.path()).is_err(), "accepted invalid id {id:?}");
        }
        std::fs::write(root.path().join("app.toml"), "[app]\nid = 'fine'\nname = 'X'\nversion = '1.0'\n").unwrap();
        assert_eq!(app_info(root.path()).unwrap(), ("fine".to_string(), root.path().join("app.wasm")));
        std::fs::write(root.path().join("app.toml"), "[app]\nid = 'fine'\nname = 'X'\nversion = '1.0'\n[capabilities]\ngrant = ['kv-wrte']\n").unwrap();
        assert!(app_info(root.path()).unwrap_err().to_string().contains("kv-wrte"), "a misspelt grant shows before the build");
    }
}
