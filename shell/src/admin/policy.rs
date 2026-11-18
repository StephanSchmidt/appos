//! The policy page: what policy.csv says, read as sentences about each
//! one it names: which of the apps' grants they may use where, and whether
//! they may use the admin screen.

use std::collections::BTreeMap;

use crate::access::{ADMIN_PREFIX, grants_of};
use crate::auth::{ANYONE, SIGNED_IN};
use crate::platform::now_ms;
use crate::shell::Shell;

use super::format::{clock, plural};
use super::model::{App, Tone, load, name_of, state};
use super::style::{grant_label, tone_icon};
use super::{Ask, Viewer, html, page, read};
use crate::kit::Reply;

/// `GET /admin/policy`.
pub(super) async fn policy_page(ask: Ask) -> Reply {
    let Ask { shell, viewer, .. } = ask;
    read(shell, move |shell| policy_html(shell, &viewer)).await
}

fn policy_html(shell: &Shell, viewer: &Viewer) -> Reply {
    let (apps, _) = load(shell, now_ms());
    let file = shell.policy.as_ref().map(|policies| {
        let policy = policies.current();
        let (rules, members) = policy.lines();
        File {
            name: policies.file_name(),
            refused: policies.refused().map(|(at, why)| (clock(at), why)),
            rows: rows(shell, &apps, rules, members),
            path: policies.path().map(|p| p.display().to_string()),
            lines: plural(policy.rules(), "line"),
            since: clock(policies.since()),
        }
    });
    let body = html(&Policy { file, warn: tone_icon(Tone::Warning, 16), email: crate::auth::token::EMAIL });
    Reply::document(page(shell, viewer, state(&apps), "/admin/policy", &format!("Policy · {} admin", shell.name), &body, None))
}

/// The page's card (templates/admin/policy.html); no file with `--no-auth`.
#[derive(askama::Template)]
#[template(path = "admin/policy.html")]
struct Policy {
    file: Option<File>,
    warn: String,
    /// Who the built-in policy lets in.
    email: &'static str,
}

/// The policy in force: the file's name, why a newer one wasn't taken,
/// the rows, where it is (none when built in), its lines and when read.
struct File {
    name: String,
    refused: Option<(String, String)>,
    rows: Vec<Row>,
    path: Option<String>,
    lines: String,
    since: String,
}

/// One subject: its name, what it is, what it may.
struct Row {
    name: String,
    what: String,
    may: Vec<Rule>,
}

/// One rule as a sentence: the grants (their labels; none when no kit
/// offers them), the pattern as written when it says more, and where.
struct Rule {
    grants: Option<String>,
    what: String,
    pattern: bool,
    place: Place,
}

enum Place {
    /// admin-read, admin-write: the platform's.
    Platform,
    Every,
    Group(String),
    /// A typo shows: no app of that id is here.
    Unknown(String),
    /// Its path segment and its name.
    App(String, String),
}

/// What the file says about one subject.
#[derive(Default)]
struct About {
    /// Its rules.
    may: Vec<Rule>,
    /// For a group: who the file puts in it.
    members: Vec<String>,
    /// The groups the file puts it in.
    groups: Vec<String>,
}

/// Anyone first, then everyone signed in, the groups, the people.
fn rank(who: &str) -> (u8, String) {
    match who {
        ANYONE => (0, String::new()),
        SIGNED_IN => (1, String::new()),
        _ if who.starts_with("group:") => (2, who.to_string()),
        _ => (3, who.to_string()),
    }
}

fn rows(shell: &Shell, apps: &[App], rules: &[Vec<String>], members: &[Vec<String>]) -> Vec<Row> {
    let mut about: BTreeMap<(u8, String), (String, About)> = BTreeMap::new();
    fn of<'a>(about: &'a mut BTreeMap<(u8, String), (String, About)>, who: &str) -> &'a mut About {
        &mut about.entry(rank(who)).or_insert_with(|| (who.to_string(), About::default())).1
    }
    for r in rules {
        of(&mut about, &r[0]).may.push(may(shell, apps, &r[1], &r[2]));
    }
    for m in members {
        of(&mut about, &m[1]).members.push(m[0].clone());
        of(&mut about, &m[0]).groups.push(m[1].trim_start_matches("group:").to_string());
    }
    about.into_values().map(|(who, a)| subject(&who, a)).collect()
}

/// Who, in words, with the groups the file links it to.
fn subject(who: &str, a: About) -> Row {
    let (name, what) = match who {
        ANYONE => ("Anyone".to_string(), "Signed in or not".to_string()),
        SIGNED_IN => ("Everyone signed in".to_string(), String::new()),
        _ => match who.strip_prefix("group:") {
            Some(group) if a.members.is_empty() => (group.to_string(), "Group".to_string()),
            Some(group) => (group.to_string(), format!("Group, with {}", a.members.join(", "))),
            None => (who.to_string(), String::new()),
        },
    };
    let what = match (what.is_empty(), a.groups.is_empty()) {
        (_, true) => what,
        (true, false) => format!("In {}", a.groups.join(", ")),
        (false, false) => format!("{what}; in {}", a.groups.join(", ")),
    };
    Row { name, what, may: a.may }
}

/// One rule: the grants (with their risk) and where.
fn may(shell: &Shell, apps: &[App], app: &str, what: &str) -> Rule {
    let names = shell.kits.now().names();
    let offered = grants_of(what, &names);
    let place = match app {
        "*" if !offered.is_empty() && offered.iter().all(|g| g.starts_with(ADMIN_PREFIX)) => Place::Platform,
        "*" => Place::Every,
        _ => match app.strip_prefix("apps:") {
            Some(group) => Place::Group(group.to_string()),
            None if !apps.iter().any(|a| a.id == app) => Place::Unknown(app.to_string()),
            None => Place::App(form_urlencoded::byte_serialize(app.as_bytes()).collect(), name_of(apps, app).to_string()),
        },
    };
    Rule {
        // A Wasm kit's grant, while the kit isn't deployed, or a typo.
        grants: (!offered.is_empty()).then(|| offered.iter().map(|g| grant_label(g, shell.risk.of(g))).collect()),
        what: what.to_string(),
        pattern: !offered.is_empty() && offered != [what],
        place,
    }
}
