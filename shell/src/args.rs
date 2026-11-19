//! The command line of a platform's binary, `appos-shell` or one built
//! with `Platform::main` (lib.rs).

use std::path::PathBuf;

use crate::terminal_println as println;

const USAGE: &str = "\
PROGRAM [--addr 127.0.0.1:8080] [--platform-addr IP:PORT] [--name NAME] [--git URL]
        [--origin URL] [--data DIR] [--risk FILE] [--auth FILE --policy FILE | --no-auth]
        [--kit DIR]... [--review COMMAND] [APP...]
PROGRAM --contract    the WIT apps build against, as JSON (appos sync --shell)
PROGRAM --fakes DIR   writes the kits' fakes into DIR, for appos dev (appos sync --shell)
PROGRAM --dev APP...  what appos dev --shell runs: the apps, open, reloading on each line on stdin

--kit DIR: a kit in Wasm, its kit.toml in DIR; once per kit
--review COMMAND: each app deploy waits until COMMAND passes (exit 0), run
  in a folder with appfuzz.toml, plan.json and the app's .wasm, e.g.
  --review 'appfuzz review --plan plan.json'
PROGRAM --version";

/// What the command line asks for.
pub(crate) enum Run {
    Serve(Box<crate::Options>),
    /// Serve as `appos dev` does: each line on stdin reloads the apps
    /// (`stdin_reloads`).
    Dev(Box<crate::Options>),
    /// The contract with these Wasm kits.
    Contract(Vec<PathBuf>),
    /// Write the fakes into this directory.
    Fakes(PathBuf),
    /// --help, --version: printed already.
    Nothing,
}

pub(crate) fn parse(mut args: impl Iterator<Item = String>) -> anyhow::Result<Run> {
    fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> anyhow::Result<String> {
        args.next().filter(|v| !v.starts_with("--") && v != "-h").ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))
    }
    let (mut dev, mut contract, mut fakes) = (false, false, None);
    let mut opts = crate::Options {
        addr: "127.0.0.1:8080".into(),
        platform_addr: None,
        platform_api: true,
        name: "local".into(),
        git: None,
        origin: None,
        data: PathBuf::from("appos-data"),
        apps: vec![],
        kits: vec![],
        reload: None,
        risk: Default::default(),
        keep_examples: false,
        review: None,
        auth: None,
        policy: None,
        no_auth: false,
        dev_sign_in: false,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" => opts.addr = value(&mut args, &arg)?,
            "--origin" => opts.origin = Some(value(&mut args, &arg)?),
            "--platform-addr" => opts.platform_addr = Some(value(&mut args, &arg)?),
            "--git" => opts.git = Some(value(&mut args, &arg)?),
            "--name" => opts.name = value(&mut args, &arg)?,
            "--data" => opts.data = value(&mut args, &arg)?.into(),
            "--auth" => opts.auth = Some(value(&mut args, &arg)?.into()),
            "--policy" => opts.policy = Some(value(&mut args, &arg)?.into()),
            "--no-auth" => opts.no_auth = true,
            "--dev" => dev = true,
            "--contract" => contract = true,
            "--fakes" => fakes = Some(PathBuf::from(value(&mut args, &arg)?)),
            "--kit" => opts.kits.push(value(&mut args, &arg)?.into()),
            "--review" => opts.review = Some(value(&mut args, &arg)?),
            "--risk" => {
                let path = value(&mut args, &arg)?;
                opts.risk = crate::Risk::load(std::path::Path::new(&path))?;
            }
            "-V" | "--version" => {
                println!("{} (appos-shell {})", program(), env!("CARGO_PKG_VERSION"));
                return Ok(Run::Nothing);
            }
            "-h" | "--help" => {
                println!("{}", USAGE.replace("PROGRAM", &program()));
                return Ok(Run::Nothing);
            }
            _ if arg.starts_with('-') => anyhow::bail!("unknown option {arg:?}"),
            _ => opts.apps.push(PathBuf::from(arg)),
        }
    }
    if contract {
        return Ok(Run::Contract(opts.kits));
    }
    if let Some(dir) = fakes {
        return Ok(Run::Fakes(dir));
    }
    if dev {
        let Some(first) = opts.apps.first() else { anyhow::bail!("--dev runs apps: give their directories") };
        let mut dev = crate::Options::dev(first.clone(), opts.addr.clone(), tokio::sync::mpsc::channel(1).1);
        dev.apps = opts.apps.clone();
        dev.kits = opts.kits.clone();
        dev.risk = opts.risk;
        return Ok(Run::Dev(Box::new(dev)));
    }
    Ok(Run::Serve(Box::new(opts)))
}

/// Each line on stdin reloads the apps; the end of stdin, `appos dev`
/// stopping, ends the process.
pub(crate) fn stdin_reloads() -> tokio::sync::mpsc::Receiver<()> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            if line.is_err() {
                break;
            }
            let _ = tx.try_send(());
        }
        std::process::exit(0);
    });
    rx
}

fn program() -> String {
    std::env::args().next().and_then(|p| std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "appos-shell".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The options to serve with; None for --help and --version.
    fn serve(args: impl Iterator<Item = String>) -> anyhow::Result<Option<crate::Options>> {
        Ok(match parse(args)? {
            Run::Serve(opts) => Some(*opts),
            _ => None,
        })
    }

    #[test]
    fn missing_values_never_fall_back_to_other_listener_settings() {
        for flag in ["--addr", "--origin", "--platform-addr", "--git", "--name", "--data", "--risk", "--auth", "--policy", "--review"] {
            assert!(serve(["--addr", "0.0.0.0:8080", flag].into_iter().map(String::from)).is_err(), "accepted missing {flag} value");
        }
    }

    #[test]
    fn no_auth_is_asked_for() {
        let parse = |args: &[&str]| serve(args.iter().map(|a| a.to_string())).unwrap().unwrap();
        assert!(!parse(&[]).no_auth);
        assert!(parse(&["--no-auth"]).no_auth);
    }

    #[test]
    fn dev_serves_the_apps_given() {
        let Run::Dev(opts) = parse(["--dev", "a", "b"].into_iter().map(String::from)).unwrap() else { panic!("not dev") };
        assert_eq!(opts.apps, [PathBuf::from("a"), PathBuf::from("b")]);
        assert!(opts.no_auth && opts.dev_sign_in && !opts.platform_api);
        assert!(parse(["--dev"].into_iter().map(String::from)).is_err());
    }

    #[test]
    fn version_and_help_start_nothing() {
        for flag in ["--version", "-V", "--help", "-h"] {
            assert!(serve([flag].into_iter().map(String::from)).unwrap().is_none(), "{flag} would start the shell");
        }
    }

    #[test]
    fn unknown_options_are_errors() {
        assert!(serve(["--platfrom-addr"].into_iter().map(String::from)).is_err());
    }

    #[test]
    fn flags_are_not_consumed_as_values_and_valid_settings_survive() {
        assert!(serve(["--platform-addr", "--name", "test"].into_iter().map(String::from)).is_err());
        let opts =
            serve(["--addr", "0.0.0.0:8080", "--platform-addr", "127.0.0.1:9090", "--name", "test", "app"].into_iter().map(String::from)).unwrap().unwrap();
        assert_eq!(opts.addr, "0.0.0.0:8080");
        assert_eq!(opts.platform_addr.as_deref(), Some("127.0.0.1:9090"));
        assert_eq!(opts.name, "test");
        assert_eq!(opts.apps, [PathBuf::from("app")]);
    }
}
