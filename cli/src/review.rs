//! `appos review`: appfuzz runs the app many thousands of times against a
//! simulated shell, with the manifest, data and examples the shell writes
//! (appos_shell::review), and says what it found.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context as _;
use appos_shell::review::{MIN_APPFUZZ, Review};
use appos_shell::terminal_println as println;

use crate::{app_info, dev};

const INSTALL: &str = "install it: curl -fsSL https://appfuzz.dev/install.sh | sh";

/// Reviews the app in this directory; returns appfuzz's exit code: 0 no
/// findings, 1 warnings, 2 failures, 3 incomplete.
pub async fn review(time: Option<&str>) -> anyhow::Result<i32> {
    let appfuzz = appfuzz();
    check(&appfuzz)?;
    let dir = dev::build()?;
    let (id, wasm) = app_info(&dir)?;
    let data = dir.join("appos-data");
    let out = data.join("review");
    std::fs::create_dir_all(&out).with_context(|| format!("could not create {}", out.display()))?;
    let name = format!("{id}.wasm");
    std::fs::copy(&wasm, out.join(&name)).with_context(|| format!("could not copy {}", wasm.display()))?;

    let r = appos_shell::review::prepare(&dir, &data, &name).await?;
    std::fs::write(out.join("appfuzz.toml"), &r.manifest)?;
    std::fs::write(out.join("plan.json"), &r.plan)?;
    println!("{}", summary(&r));

    let mut cmd = Command::new(&appfuzz);
    cmd.args(["--human", "review", "--plan", "plan.json"]).current_dir(&out);
    if let Some(time) = time {
        cmd.args(["--time", time]);
    }
    let status = cmd.status().context("could not run appfuzz")?;
    let shown = out.strip_prefix(&dir).unwrap_or(&out);
    println!("\nThe review's files are in {}: appfuzz --human findings there shows them again.", shown.display());
    Ok(status.code().unwrap_or(3))
}

/// What the review starts from.
fn summary(r: &Review) -> String {
    let kept = r.examples.iter().filter(|e| e.kept).count();
    let mut s = format!("review: {} routes, {kept} with a request from appos dev, {} kv entries from appos-data", r.examples.len(), r.entries);
    let missing: Vec<&str> = r.examples.iter().filter(|e| !e.kept).map(|e| e.route.as_str()).collect();
    if !missing.is_empty() {
        s.push_str("\nNo request from appos dev yet for these; open them there (one that works is kept) and the review reaches further:");
        for route in missing {
            s.push_str(&format!("\n  {route}"));
        }
    }
    s
}

/// `APPOS_APPFUZZ`, else appfuzz on the PATH.
fn appfuzz() -> PathBuf {
    std::env::var_os("APPOS_APPFUZZ").map_or_else(|| PathBuf::from("appfuzz"), PathBuf::from)
}

/// appfuzz is there and new enough.
fn check(appfuzz: &Path) -> anyhow::Result<()> {
    let out = Command::new(appfuzz).arg("--version").output().map_err(|_| anyhow::anyhow!("appfuzz is not installed; {INSTALL}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let have = version(&text);
    if have < MIN_APPFUZZ {
        let (a, b, c) = MIN_APPFUZZ;
        anyhow::bail!("{} is too old, appos review needs {a}.{b}.{c} or newer; {INSTALL}", text.trim());
    }
    Ok(())
}

/// "appfuzz 0.7.0" as (0, 7, 0); (0, 0, 0) if it says no version.
fn version(text: &str) -> (u32, u32, u32) {
    let v = text.split_whitespace().last().unwrap_or("");
    let mut parts = v.split(['.', '-', '+']).map(|p| p.parse().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(version("appfuzz 0.7.0\n"), (0, 7, 0));
        assert_eq!(version("appfuzz 0.10.2-dev"), (0, 10, 2));
        assert_eq!(version(""), (0, 0, 0));
        assert!(version("appfuzz 0.10.0") > (0, 7, 0));
        assert!(version("appfuzz 0.7.0") < MIN_APPFUZZ);
        assert!(version("appfuzz 0.8.0") >= MIN_APPFUZZ);
    }
}
