//! `appos review`: writes what appfuzz (https://appfuzz.dev) needs to
//! review the app, from what the shell knows (appos_shell::review), and
//! prints the command that runs it. It doesn't run appfuzz: the review,
//! its options, findings and exit code are appfuzz's own.

use anyhow::Context as _;
use appos_shell::review::{MIN_APPFUZZ, Review};
use appos_shell::terminal_println as println;

use crate::{app_info, dev};

const INSTALL: &str = "curl -fsSL https://appfuzz.dev/install.sh | sh";

/// Builds the app in this directory and writes appfuzz.toml, plan.json
/// and the app's wasm into ./appos-data/review.
pub async fn review() -> anyhow::Result<()> {
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
    let shown = out.strip_prefix(&dir).unwrap_or(&out).display().to_string();
    println!("\n{}", next(&shown));
    Ok(())
}

/// What appfuzz starts from.
fn summary(r: &Review) -> String {
    let kept = r.examples.iter().filter(|e| e.kept).count();
    let mut s = format!("For appfuzz: {} routes, {kept} with a request from appos dev, {} kv entries from appos-data", r.examples.len(), r.entries);
    let missing: Vec<&str> = r.examples.iter().filter(|e| !e.kept).map(|e| e.route.as_str()).collect();
    if !missing.is_empty() {
        s.push_str("\nNo request from appos dev yet for these; open them there (one that works is kept) and the review reaches further:");
        for route in missing {
            s.push_str(&format!("\n  {route}"));
        }
    }
    s
}

/// The command that runs the review in `dir`, and where appfuzz comes from.
fn next(dir: &str) -> String {
    let (a, b, c) = MIN_APPFUZZ;
    format!(
        "Run the review with appfuzz {a}.{b}.{c} or newer:\n\n  cd {dir} && appfuzz --human review --plan plan.json\n\n\
         appfuzz help review lists its options, like --time. No appfuzz yet? {INSTALL}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_says_how_to_run_appfuzz_and_where() {
        let text = next("appos-data/review");
        assert!(text.contains("cd appos-data/review && appfuzz --human review --plan plan.json"), "{text}");
        assert!(text.contains("appfuzz 0.9.0 or newer") && text.contains("https://appfuzz.dev/install.sh"), "{text}");
    }
}
