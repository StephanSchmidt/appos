//! The AppOS shell.
//!
//!   appos-shell [--addr 127.0.0.1:8080] [--platform-addr IP:PORT] [--name NAME]
//!               [--git URL] [--origin URL] [--data DIR] [--risk FILE]
//!               [--auth FILE --policy FILE | --no-auth] [APP...]
//!
//! Each APP is a directory with an `app.toml`, or the `app.toml` itself.
//! `--risk` rates the grants from 0 to 10, one `grant = level` per line.
//! `--auth` sets up sign-in (auth.toml) and `--policy` says who may do what
//! (policy.csv); they go together. Without them the shell runs on this
//! machine only and prints a link with a token that signs in as an admin
//! who may do everything, and nobody else may do anything. `--no-auth` runs
//! it open: anyone may do anything, on the admin screen and the platform API
//! too.

use appos_shell::{terminal_eprintln as eprintln, terminal_println as println};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let result = async {
        if let Some(opts) = parse_options(std::env::args().skip(1))? {
            appos_shell::serve(opts).await?;
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(error) = result {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn parse_options(mut args: impl Iterator<Item = String>) -> anyhow::Result<Option<appos_shell::Options>> {
    fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> anyhow::Result<String> {
        args.next().filter(|v| !v.starts_with("--") && v != "-h").ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))
    }
    let mut opts = appos_shell::Options {
        addr: "127.0.0.1:8080".into(),
        platform_addr: None,
        platform_api: true,
        name: "local".into(),
        git: None,
        origin: None,
        data: PathBuf::from("appos-data"),
        apps: vec![],
        reload: None,
        risk: Default::default(),
        keep_examples: false,
        auth: None,
        policy: None,
        no_auth: false,
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
            "--risk" => {
                let path = value(&mut args, &arg)?;
                opts.risk = appos_shell::Risk::load(std::path::Path::new(&path))?;
            }
            "-h" | "--help" => {
                println!(
                    "appos-shell [--addr 127.0.0.1:8080] [--platform-addr IP:PORT] [--name NAME] [--git URL] [--origin URL] [--data DIR] [--risk FILE] [--auth FILE --policy FILE | --no-auth] [APP...]"
                );
                return Ok(None);
            }
            _ if arg.starts_with('-') => anyhow::bail!("unknown option {arg:?}"),
            _ => opts.apps.push(PathBuf::from(arg)),
        }
    }
    Ok(Some(opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_values_never_fall_back_to_other_listener_settings() {
        for flag in ["--addr", "--origin", "--platform-addr", "--git", "--name", "--data", "--risk", "--auth", "--policy"] {
            assert!(parse_options(["--addr", "0.0.0.0:8080", flag].into_iter().map(String::from)).is_err(), "accepted missing {flag} value");
        }
    }

    #[test]
    fn no_auth_is_asked_for() {
        let parse = |args: &[&str]| parse_options(args.iter().map(|a| a.to_string())).unwrap().unwrap();
        assert!(!parse(&[]).no_auth);
        assert!(parse(&["--no-auth"]).no_auth);
    }

    #[test]
    fn unknown_options_are_errors() {
        assert!(parse_options(["--platfrom-addr"].into_iter().map(String::from)).is_err());
    }

    #[test]
    fn flags_are_not_consumed_as_values_and_valid_settings_survive() {
        assert!(parse_options(["--platform-addr", "--name", "test"].into_iter().map(String::from)).is_err());
        let opts = parse_options(["--addr", "0.0.0.0:8080", "--platform-addr", "127.0.0.1:9090", "--name", "test", "app"].into_iter().map(String::from))
            .unwrap()
            .unwrap();
        assert_eq!(opts.addr, "0.0.0.0:8080");
        assert_eq!(opts.platform_addr.as_deref(), Some("127.0.0.1:9090"));
        assert_eq!(opts.name, "test");
        assert_eq!(opts.apps, [PathBuf::from("app")]);
    }
}
