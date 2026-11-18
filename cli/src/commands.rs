//! The commands that talk to a platform: login, platforms, use, hosts,
//! pack and deploy, versions, log, rollback, off and on.

use std::path::Path;

use anyhow::Context as _;
use appos_shell::api::{Actor, Kind};
use appos_shell::bundle::{self, Bundle};
// Platform strings and stored logs are untrusted terminal data.
use appos_shell::{terminal_eprintln as eprintln, terminal_println as println};

use crate::platform::{self, Config, Platform};
use crate::{app_info, dev, scaffold};

/// Connects to a platform. One that takes a token (spec.token) gets the
/// one kept for its URL if it still works, or one pasted from its admin
/// screen.
pub fn login(url: &str) -> anyhow::Result<()> {
    let url = url.trim_end_matches('/').to_string();
    let spec = platform::fetch_spec(&url)?;
    let (token, me) = match spec.token {
        false => (None, None),
        true => {
            let kept = Config::load()?.platforms.into_iter().find(|p| p.url == url).and_then(|p| p.token);
            match kept.and_then(|t| platform::me(&url, &t).ok().map(|me| (t, me))) {
                Some((token, me)) => (Some(token), Some(me)),
                None => {
                    let token = paste_token(&spec.name, &url)?;
                    let me = platform::me(&url, &token)?;
                    (Some(token), Some(me))
                }
            }
        }
    };
    Config::update(|config| {
        config.upsert(Platform { name: spec.name.clone(), url: url.clone(), token });
        Ok(())
    })?;
    let as_whom = me.map(|me| format!(" as {}", me.email)).unwrap_or_default();
    println!(
        "Connected to {} at {url}{as_whom}; it is now the current platform.\n  site:         {}\n  capabilities: {}\n\nNext: appos new <name>, or appos deploy in an app's directory",
        spec.name,
        spec.origin,
        spec.capabilities.join(", ")
    );
    Ok(())
}

/// A token made on the platform's admin screen: one line on stdin (`echo
/// $TOKEN | appos login <url>`), or typed at the terminal when stdin
/// brings none (`curl … | sh` runs this with the script as stdin).
fn paste_token(name: &str, url: &str) -> anyhow::Result<String> {
    use std::io::{BufRead, IsTerminal};

    eprintln!("{name} takes a token. Make one on its admin screen, signed in as an admin:\n  {url}/admin/token\nand paste it here:");
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    if line.trim().is_empty()
        && !std::io::stdin().is_terminal()
        && let Ok(tty) = std::fs::File::open("/dev/tty")
    {
        std::io::BufReader::new(tty).read_line(&mut line)?;
    }
    match line.trim() {
        "" => anyhow::bail!("no token given; make one at {url}/admin/token"),
        token => Ok(token.to_string()),
    }
}

/// Ends the current platform's token, there and here.
pub fn logout() -> anyhow::Result<()> {
    let p = current()?;
    let Some(token) = &p.token else {
        println!("{} takes no token here: nothing to log out of.", p.name);
        return Ok(());
    };
    let ended = platform::logout(&p.url, token);
    Config::update(|config| {
        if let Some(kept) = config.platforms.iter_mut().find(|k| k.name == p.name && k.url == p.url) {
            kept.token = None;
        }
        Ok(())
    })?;
    match ended {
        Ok(()) => println!("Logged out of {}: the token is ended.", p.name),
        Err(e) => eprintln!(
            "Logged out of {} here, but the platform could not end the token ({e:#}): it works until it expires, unless the policy drops you.",
            p.name
        ),
    }
    Ok(())
}

/// The current platform, for commands that need one.
pub(crate) fn current() -> anyhow::Result<Platform> {
    let config = Config::load()?;
    match config.current() {
        Some(p) => Ok(p.clone()),
        None => anyhow::bail!("no platform yet; connect to one with `appos login <url>`. For local development, use `appos dev`."),
    }
}

/// The current platform and the id of the app in this directory.
fn here() -> anyhow::Result<(Platform, String)> {
    let p = current()?;
    let (id, _) = app_info(&std::env::current_dir()?)?;
    Ok((p, id))
}

/// The zip's name: app.zip, or kit.zip for a Wasm kit.
fn zip_name(dir: &Path) -> &'static str {
    if dir.join(appos_shell::KIT_TOML).is_file() { "kit.zip" } else { "app.zip" }
}

/// The app in this directory as app.zip, or the kit as kit.zip: checks
/// app.toml or kit.toml first (a deploy needs an owner, no point building
/// without), builds, packs. Returns the id and the zip.
fn pack_here() -> anyhow::Result<(String, Vec<u8>)> {
    let dir = std::env::current_dir()?;
    app_info(&dir)?;
    let (_, manifest, _) = appos_shell::Manifest::load(&dir)?;
    let id = bundle::check(&manifest)?;
    dev::build()?;
    let bundle = Bundle { deps: crate::deps::json(&dir), ..Bundle::from_dir(&dir)? };
    let zip = bundle.pack()?;
    if zip.len() > bundle::MAX {
        anyhow::bail!("{} is {} MB, more than the {} MB a platform takes", zip_name(&dir), zip.len() >> 20, bundle::MAX >> 20);
    }
    Ok((id, zip))
}

pub fn pack(file: Option<&str>) -> anyhow::Result<()> {
    let (_, zip) = pack_here()?;
    // target/ is in the app's .gitignore.
    let packed = format!("target/{}", zip_name(&std::env::current_dir()?));
    let path = Path::new(file.unwrap_or(&packed));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, &zip).with_context(|| format!("could not write {}", path.display()))?;
    println!("packed {} ({} KB); deploy it with: appos deploy {}", path.display(), zip.len().div_ceil(1024), path.display());
    Ok(())
}

/// Builds and packs the app here, or reads `file`, and sends it.
pub fn deploy(file: Option<&str>) -> anyhow::Result<()> {
    let p = current()?;
    let (id, zip) = match file {
        None => {
            // Best effort: an unreachable platform fails the deploy itself.
            // A kit builds against what it chooses, not an app's contract.
            let dir = std::env::current_dir()?;
            if !dir.join(appos_shell::KIT_TOML).is_file()
                && let Ok(spec) = platform::fetch_spec(&p.url)
            {
                scaffold::warn_if_stale(&dir, &spec.wit, &format!("platform {}", p.name));
            }
            pack_here()?
        }
        Some(f) => {
            let zip = bundle::read_file(Path::new(f)).with_context(|| format!("could not read {f}"))?;
            let app = Bundle::unpack(&zip).with_context(|| f.to_string())?;
            (bundle::check(&app.manifest).with_context(|| f.to_string())?, zip)
        }
    };
    // The held answer says why deploys wait, so not the news too.
    let seen = tell_news(&p, &id, false);
    let done = platform::deploy(&p, &zip);
    if done.is_err()
        && let Some(since) = seen
    {
        // The refusal just printed is not news next time; anything else is.
        let mine = |e: &platform::NewsEvent| e.kind == Kind::Refused && e.actor == Actor::Cli;
        let _ = platform::news(&p, &id, Some(since)).map(|n| print_news(&p, &id, n, false, mine));
    }
    let done = done?;
    match done.held {
        None => println!("Deployed {} {} to {}: {}", done.id, done.label, p.name, done.url),
        Some(why) => {
            println!("Stored {} {} on {}, but it is not live.", done.id, done.label, p.name);
            println!("{why}{}", done.admin.map(|url| format!(":\n  {url}")).unwrap_or_default());
        }
    }
    Ok(())
}

pub fn versions() -> anyhow::Result<()> {
    let (p, id) = here()?;
    let versions = platform::versions(&p, &id)?;
    tell_news(&p, &id, true);
    println!("{id} on {}, newest first:", p.name);
    for v in versions {
        let when = local_time(v.deployed_at);
        println!("  {:<20} {:<8} {when}", v.label, v.status);
        if let Some(reason) = v.reason {
            println!("  {:<20} {}", "", first_line(&reason));
        }
    }
    Ok(())
}

/// Events on a page of `appos log`, and failed requests shown per event.
const LOG_PAGE: u32 = 20;

pub fn log(failures: bool, before: Option<i64>) -> anyhow::Result<()> {
    let (p, id) = here()?;
    let page = platform::log(&p, &id, failures, before, LOG_PAGE)?;
    if page.events.is_empty() {
        println!("{id} on {}: {}.", p.name, if before.is_some() { "nothing older" } else { "nothing yet" });
        return Ok(());
    }
    println!("{id} on {}, {}newest first:", p.name, if failures { "failures only, " } else { "" });
    for e in &page.events {
        println!();
        println!("{}  {}  {}  {}, by {}", clock(e.at, "%Y-%m-%d %H:%M:%S"), e.kind, e.label, e.severity, e.person.as_deref().unwrap_or(who(e.actor, e.kind)));
        let why = e.reason.as_deref().map(|r| format!(": {}", first_line(r))).unwrap_or_default();
        println!("  {}{why}", e.message);
        if let Some(f) = &e.failed {
            match f.count {
                0 => println!("  Its failed requests are no longer kept."),
                n if n as usize > f.first.len() => println!("  {n} failed requests behind it, the first {}:", f.first.len()),
                1 => println!("  The failed request behind it:"),
                n => println!("  The {n} failed requests behind it:"),
            }
            for r in &f.first {
                let error = r.error.as_deref().map(first_line).unwrap_or("");
                println!("    {}  {}  {}  {error}", clock(r.at, "%H:%M:%S"), r.route, r.outcome);
            }
        }
    }
    if let Some(older) = page.older {
        println!("\nOlder: appos log{} --before {older}", if failures { " --failures" } else { "" });
    }
    Ok(())
}

/// Who did it, in words: an admin, the platform, or the appos command.
fn who(actor: Actor, kind: Kind) -> &'static str {
    match (actor, kind) {
        (Actor::Admin, _) => "an admin",
        (Actor::Shell, _) => "the platform",
        (_, Kind::RolledBack) => "appos rollback",
        (_, Kind::TurnedOff) => "appos off",
        (_, Kind::TurnedOn) => "appos on",
        (_, Kind::Deployed | Kind::Refused) => "appos deploy",
        _ => "appos",
    }
}

/// Prints what happened to the app since the last look and remembers this
/// look; returns the platform's time of it. Best effort: a platform that
/// can't tell doesn't stop the command.
fn tell_news(p: &Platform, app_id: &str, paused: bool) -> Option<i64> {
    let since = platform::Seen::load().get(p, app_id);
    let news = platform::news(p, app_id, since).ok()?;
    Some(print_news(p, app_id, news, paused, |_| false))
}

/// Prints the news but those `skip` says were already told, remembers
/// the look, and returns its time.
fn print_news(p: &Platform, app_id: &str, news: platform::News, paused: bool, skip: impl Fn(&platform::NewsEvent) -> bool) -> i64 {
    let lines: Vec<String> = news.events.iter().filter(|e| !skip(e)).map(news_line).collect();
    if !lines.is_empty() {
        println!("Since your last look on {}:", p.name);
        if news.more > 0 {
            println!("  ({} earlier, see the admin screen)", news.more);
        }
        for l in lines {
            println!("  {l}");
        }
        println!();
    }
    if let Some(o) = &news.off {
        let why = o.reason.as_deref().map(|r| format!(": {}", first_line(r))).unwrap_or_default();
        println!("{app_id} is turned off since {}, by {}{why}. Visitors get a page saying so.", local_time(o.at), who(o.actor, Kind::TurnedOff));
        match o.actor {
            Actor::Admin => println!("It goes back on from the admin screen:\n  {}\n", o.admin),
            _ => println!("Turn it on with: appos on\n"),
        }
    }
    if paused && let Some(w) = &news.paused {
        let why = w.reason.as_deref().map(|r| format!(" ({})", first_line(r))).unwrap_or_default();
        println!("Deploys wait: the platform rolled {app_id} back at {}{why}.", local_time(w.at));
        println!("New deploys are stored but not live until an admin makes a version live:\n  {}\n", w.admin);
    }
    let mut seen = platform::Seen::load();
    seen.set(p, app_id, news.now);
    if let Err(e) = seen.save() {
        eprintln!("warning: {e:#}");
    }
    news.now
}

/// One event as a line: when, what, who, why. What the platform did by
/// itself, and refusals, need no who.
fn news_line(e: &platform::NewsEvent) -> String {
    let what = match e.kind {
        Kind::Refused => format!("{} refused", e.label),
        _ => e.message.clone(),
    };
    let who = match (e.kind, e.actor) {
        (Kind::AutoRolledBack | Kind::Refused, _) | (_, Actor::Shell) => String::new(),
        (kind, actor) => format!(", by {}", who(actor, kind)),
    };
    let why = e.reason.as_deref().map(|r| format!(": {}", first_line(r))).unwrap_or_default();
    format!("{}  {what}{who}{why}", local_time(e.at))
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

pub fn turn(on: bool, reason: Option<&str>) -> anyhow::Result<()> {
    let (p, id) = here()?;
    platform::turn(&p, &id, on, reason)?;
    if on {
        println!("{id} is on again on {}.", p.name);
    } else if std::env::current_dir()?.join(appos_shell::KIT_TOML).is_file() {
        println!("{id} is off on {}: its pages, and the apps that use it, answer with a page saying so. Turn it on with: appos on", p.name);
    } else {
        println!("{id} is off on {}: every route answers with a page saying so. Turn it on with: appos on", p.name);
    }
    Ok(())
}

pub fn rollback(label: Option<&str>, reason: Option<&str>) -> anyhow::Result<()> {
    let (p, id) = here()?;
    let done = platform::rollback(&p, &id, label, reason)?;
    println!("{} {} is live again on {}: {}", done.id, done.label, p.name, done.url);
    Ok(())
}

/// Unix milliseconds as local "YYYY-MM-DD HH:MM".
fn local_time(ms: i64) -> String {
    clock(ms, "%Y-%m-%d %H:%M")
}

/// A time on this machine's clock.
fn clock(ms: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&chrono::Local).format(format).to_string()).unwrap_or_default()
}

const NO_PLATFORM: &str = "No platform yet. You don't need one for local development: `appos new` and \
                           `appos dev` work without. To build for a shell, connect with `appos login <url>`.";

pub fn platforms() -> anyhow::Result<()> {
    let config = Config::load()?;
    println!("config: {}", platform::path().display());
    if config.platforms.is_empty() {
        println!("{NO_PLATFORM}");
    }
    for p in &config.platforms {
        let mark = if config.current.as_deref() == Some(p.name.as_str()) { "*" } else { " " };
        println!("{mark} {:<16} {}", p.name, p.url);
    }
    Ok(())
}

pub fn use_platform(name: &str) -> anyhow::Result<()> {
    Config::update(|config| {
        if !config.platforms.iter().any(|p| p.name == name) {
            anyhow::bail!("no platform named {name:?}; see `appos platforms`");
        }
        config.current = Some(name.to_string());
        Ok(())
    })?;
    println!("Current platform: {name}");
    Ok(())
}

pub fn hosts(show_wit: bool) -> anyhow::Result<()> {
    let config = Config::load()?;
    let Some(p) = config.current() else {
        println!("{NO_PLATFORM}\nThe built-in contract offers the capabilities: {} (see `appos guide storage`).", scaffold::builtin_capabilities().join(", "));
        return Ok(());
    };
    let spec = platform::fetch_spec(&p.url)?;
    if show_wit {
        for (name, text) in &spec.wit {
            println!("// ---- {name}\n{text}");
        }
        return Ok(());
    }
    println!("{} ({})\n  site:     {}\n  contract: {}", spec.name, p.url, spec.origin, spec.framework);
    println!("\nCapabilities an app can be granted in app.toml, with the risk the platform gives them (0 to 10):");
    for c in &spec.capabilities {
        let risk = spec.risk.get(c).map_or("unrated".to_string(), |l| l.to_string());
        println!("  {c:<12} {risk}");
    }
    println!("\nApps running there:");
    if spec.apps.is_empty() {
        println!("  none");
    }
    for a in &spec.apps {
        println!("  /{:<14} {} {}", a.id, a.name, a.version);
    }
    println!("\nThe full contract: appos hosts --wit");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What, who and the first line of why; the time is local.
    #[test]
    fn news_lines_say_what_who_and_why() {
        let e = |kind: Kind, actor: Actor, message: &str, reason: Option<&str>| platform::NewsEvent {
            at: 0,
            label: "0.2.0-ab".into(),
            kind,
            actor,
            message: message.into(),
            reason: reason.map(String::from),
        };
        let line = |e| news_line(&e).split_once("  ").unwrap().1.to_string();
        assert_eq!(
            line(e(Kind::AutoRolledBack, Actor::Shell, "Rolled back automatically from b to a", Some("5 failed"))),
            "Rolled back automatically from b to a: 5 failed"
        );
        assert_eq!(
            line(e(Kind::RolledBack, Actor::Admin, "a made live again, replacing b", Some("loses leads"))),
            "a made live again, replacing b, by an admin: loses leads"
        );
        assert_eq!(line(e(Kind::TurnedOff, Actor::Cli, "Turned off", None)), "Turned off, by appos off");
        assert_eq!(line(e(Kind::Refused, Actor::Cli, "Deploy refused", Some("needs kv-write\nmore"))), "0.2.0-ab refused: needs kv-write");
    }

    #[test]
    fn the_log_and_the_news_name_who_the_same_way() {
        assert_eq!(who(Actor::Admin, Kind::TurnedOff), "an admin");
        assert_eq!(who(Actor::Shell, Kind::AutoRolledBack), "the platform");
        assert_eq!(who(Actor::Cli, Kind::RolledBack), "appos rollback");
        assert_eq!(who(Actor::Cli, Kind::Deployed), "appos deploy");
    }
}
