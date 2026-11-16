//! `appos build` and `appos dev`: cargo, the dev lock, the watcher.

use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use appos_shell::{terminal_eprintln as eprintln, terminal_println as println};

use crate::{app_info, scaffold, sources};

/// Builds the app in this directory; returns the app directory.
pub fn build() -> anyhow::Result<PathBuf> {
    let dir = std::env::current_dir()?;
    let (_, wasm) = app_info(&dir)?;
    compile(&dir, &wasm)?;
    let size = std::fs::metadata(&wasm).map(|m| m.len()).unwrap_or(0);
    println!("built {} ({} KB)", wasm.trim_prefix(&dir).display(), size / 1024);
    Ok(dir)
}

/// Builds the app's .wasm: with scriptc for a TypeScript app, zig for a Zig
/// app, Maven and TeaVM for a Java app, else Cargo.
fn compile(dir: &Path, wasm: &Path) -> anyhow::Result<()> {
    if crate::ts::is_ts(dir) {
        crate::ts::build(dir, wasm)
    } else if crate::zig::is_zig(dir) {
        crate::zig::build(dir, wasm)
    } else if crate::java::is_java(dir) {
        crate::java::build(dir, wasm)
    } else {
        cargo_build(dir)
    }
}

fn cargo_build(dir: &Path) -> anyhow::Result<()> {
    let status = Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .current_dir(dir)
        .status()
        .map_err(|_| anyhow::anyhow!("cargo not found; install Rust from https://rustup.rs"))?;
    if !status.success() {
        check_target(dir)?;
        anyhow::bail!("build failed (see above)");
    }
    Ok(())
}

fn rebuild(dir: &Path, force: bool) -> anyhow::Result<()> {
    if crate::ts::is_ts(dir) || crate::zig::is_zig(dir) || crate::java::is_java(dir) {
        let (_, wasm) = app_info(dir)?;
        return compile(dir, &wasm);
    }
    if force {
        // Cargo's mtime freshness check cannot see a backdated content
        // change that our snapshot detected. Keep dependency caches, but
        // invalidate this workspace's app artifacts before rebuilding.
        let status = Command::new("cargo")
            .args(["clean", "--workspace", "--release", "--target", "wasm32-wasip2"])
            .current_dir(dir)
            .status()
            .context("could not invalidate stale app build artifacts")?;
        if !status.success() {
            anyhow::bail!("could not invalidate stale app build artifacts");
        }
    }
    cargo_build(dir)
}

/// After a failed build: a missing wasm target is the likely cause on a
/// fresh machine.
fn check_target(dir: &Path) -> anyhow::Result<()> {
    let out = Command::new("rustup").args(["target", "list", "--installed"]).current_dir(dir).output();
    if let Ok(out) = out
        && !String::from_utf8_lossy(&out.stdout).lines().any(|l| l.trim() == "wasm32-wasip2")
    {
        anyhow::bail!("the wasm32-wasip2 target is missing; add it with: rustup target add wasm32-wasip2");
    }
    Ok(())
}

pub async fn dev(addr: Option<&str>) -> anyhow::Result<()> {
    let addr = addr.unwrap_or("127.0.0.1:8080").to_string();
    let dir = std::env::current_dir()?;
    let (id, wasm) = app_info(&dir)?;

    // One dev server per app: it reloads on every change, so a second one
    // is never needed.
    let lock = dir.join("appos-data").join("dev.pid");
    let mut dev_lock = match claim_dev(&lock)? {
        Some(file) => file,
        None => {
            if let Some((pid, running_addr)) = running_dev(&lock) {
                println!(
                    "appos dev is already running for this app: http://{running_addr}/{id} (process {pid}).\n\
                     It rebuilds and reloads on every change; there's no need to start it again.\n\
                     To restart it: kill {pid}, then appos dev."
                );
            } else {
                println!("appos dev is already running for this app; stop its existing server before restarting.");
            }
            return Ok(());
        }
    };
    // Check the port before the (possibly long) build.
    if let Err(e) = std::net::TcpListener::bind(&addr) {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            let port = addr.rsplit(':').next().unwrap_or("");
            let next = alternative_port(port);
            anyhow::bail!(
                "port {port} is already in use by another program (perhaps appos dev for another app).\n\
                 Use another port: appos dev --addr 127.0.0.1:{next}"
            );
        }
        anyhow::bail!("can't listen on {addr}: {e}");
    }
    dev_lock.set_len(0)?;
    dev_lock.rewind()?;
    dev_lock.write_all(format!("{} {addr}\n", std::process::id()).as_bytes())?;
    dev_lock.flush()?;
    // The local shell is this appos's: its contract and SDK are the ones
    // the app should build against.
    scaffold::warn_if_stale(&dir, &scaffold::builtin_wit(), "this appos");

    // Preserve the inputs from before the first build: a save while Cargo
    // is running must still differ from the watcher's initial snapshot.
    let seen = sources::snapshot(&dir);
    let built_at = SystemTime::now();
    compile(&dir, &wasm)?;

    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let watch_dir = dir.clone();
    std::thread::spawn(move || watch(&watch_dir, tx, seen, built_at));

    println!(
        "\nappos dev: http://{addr}/{id}\n  \
         watching sources, templates, WIT and build configuration: changes rebuild and reload\n  \
         kv data in ./appos-data (stop, then delete it to start fresh), with each\n  \
         route's last request that worked, for appos review\n  \
         Ctrl-C to stop, or kill {}\n",
        std::process::id()
    );
    appos_shell::serve(appos_shell::Options {
        addr,
        platform_addr: None,
        platform_api: false,
        name: "dev".into(),
        git: None,
        origin: None,
        data: dir.join("appos-data"),
        apps: vec![dir],
        reload: Some(rx),
        risk: Default::default(),
        keep_examples: true,
        auth: None,
        policy: None,
        no_auth: true,
    })
    .await
}

fn alternative_port(port: &str) -> u16 {
    port.parse::<u16>().ok().and_then(|p| p.checked_add(1)).unwrap_or(8081)
}

/// Keep this file open throughout dev: the OS releases its lock even after
/// a crash, so a stale or reused PID can never exclude a new server.
fn claim_dev(path: &Path) -> std::io::Result<Option<std::fs::File>> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::options().read(true).write(true).create(true).truncate(false).open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Only trust the pid/address while another open file actually owns the lock.
fn running_dev(lock: &Path) -> Option<(u32, String)> {
    let file = std::fs::File::options().read(true).write(true).open(lock).ok()?;
    if !matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
        return None;
    }
    let mut text = String::new();
    file.take(1024).read_to_string(&mut text).ok()?;
    let mut parts = text.split_whitespace();
    let pid: u32 = parts.next()?.parse().ok()?;
    let addr = parts.next()?.to_string();
    (pid != 0).then_some((pid, addr))
}

/// Polls the app's sources; on a change, rebuilds and asks the shell to
/// reload. A failed build leaves the last good version running.
fn watch(dir: &Path, reload: tokio::sync::mpsc::Sender<()>, mut seen: std::collections::BTreeMap<PathBuf, sources::Stamp>, mut built_at: SystemTime) {
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let now = sources::snapshot(dir);
        if now == seen {
            continue;
        }
        // Let a burst of saves settle.
        std::thread::sleep(Duration::from_millis(300));
        let current = sources::snapshot(dir);
        let force = sources::requires_fresh_build(&seen, &current, built_at);
        seen = current;
        println!("\nchange detected, rebuilding...");
        built_at = SystemTime::now();
        match rebuild(dir, force) {
            Ok(()) => {
                let _ = reload.blocking_send(());
            }
            Err(_) => eprintln!("build failed; the last good version keeps running"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dev_lock_excludes_another_server_and_is_released_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("appos-data/dev.pid");
        let first = claim_dev(&path).unwrap().unwrap();
        assert!(claim_dev(&path).unwrap().is_none());
        drop(first);
        // Parallel tests spawn subprocesses. Between fork and exec a child
        // can briefly retain this descriptor, even with close-on-exec set.
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        loop {
            if claim_dev(&path).unwrap().is_some() {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the dropped dev lock was not released");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn suggesting_another_port_cannot_overflow() {
        assert_eq!(alternative_port("8080"), 8081);
        assert_eq!(alternative_port("65535"), 8081);
    }

    #[cfg(unix)]
    #[test]
    fn an_unlocked_stale_pid_file_does_not_identify_an_unrelated_process_as_a_server() {
        use std::process::Stdio;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("dev.pid");
        let mut other = Command::new("sh").args(["-c", "read line"]).stdin(Stdio::piped()).spawn().unwrap();
        std::fs::write(&path, format!("{} 127.0.0.1:8080\n", other.id())).unwrap();
        let found = running_dev(&path);
        drop(other.stdin.take());
        other.wait().unwrap();
        assert!(found.is_none(), "a reused PID made the CLI recommend stopping an unrelated process");
    }
}
