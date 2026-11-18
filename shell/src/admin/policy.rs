//! The policy page: what policy.csv says, read as sentences about each
//! one it names: which of the apps' grants they may use where, and whether
//! they may use the admin screen.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::response::{Html, IntoResponse, Response};

use crate::access::{ADMIN, grants_of};
use crate::auth::{ANYONE, SIGNED_IN};
use crate::platform::now_ms;
use crate::shell::Shell;
use crate::web::escape;

use super::format::{clock, plural};
use super::model::{App, Tone, load, name_of, state};
use super::style::{grant_label, tone_icon};
use super::{S, Viewer, page, read};

/// `GET /admin/policy`.
pub(super) async fn policy_page(State(shell): S, viewer: Viewer) -> Response {
    read(shell, move |shell| policy_html(shell, &viewer)).await
}

fn policy_html(shell: &Shell, viewer: &Viewer) -> Response {
    let (apps, _) = load(shell, now_ms());
    let head = r#"<div class="cardhead"><h2 id="policy-h">Policy <small>· who may use the apps' grants, and the admin screen</small></h2></div>"#;
    let body = match &shell.policy {
        Some(policies) => {
            let policy = policies.current();
            let (rules, members) = policy.lines();
            let file = escape(&policies.file_name());
            let refused = match policies.refused() {
                Some((at, why)) => format!(
                    r#"<div class="warn tone-warning banner">{}<p><b>{file} not taken</b> at {}: {}. The rules below stay in force until the file is fixed.</p></div>"#,
                    tone_icon(Tone::Warning, 16),
                    clock(at),
                    escape(&why)
                ),
                None => String::new(),
            };
            let list = if rules.is_empty() && members.is_empty() {
                format!(r#"<p class="empty">{file} has no rules: nobody may use a grant, and nobody may use the admin screen.</p>"#)
            } else {
                table(shell, &apps, rules, members)
            };
            let foot = match policies.path() {
                Some(path) => format!(
                    "{} from <code>{}</code>, read at {}. The shell checks the file every 2 seconds; what no line allows is denied.",
                    plural(policy.rules(), "line"),
                    escape(&path.display().to_string()),
                    clock(policies.since()),
                ),
                None => format!(
                    "Built in: the shell started without <code>--auth</code> and <code>--policy</code>, so the link it printed signs in as <b>{}</b>, who may do everything, and nobody else may do anything. Start it with <code>--auth auth.toml --policy policy.csv</code> to say who may do what.",
                    crate::auth::token::EMAIL
                ),
            };
            format!(r#"<section class="card logcard policy" aria-labelledby="policy-h">{head}{refused}{list}<p class="foot">{foot}</p></section>"#)
        }
        None => format!(
            r#"<section class="card logcard policy" aria-labelledby="policy-h">{head}<p class="empty">The shell runs with <code>--no-auth</code>: no sign-in and no policy, so anyone may use every grant an app holds, and this screen. Start it with <code>--auth auth.toml --policy policy.csv</code> to say who may do what.</p></section>"#
        ),
    };
    Html(page(shell, viewer, state(&apps), "/admin/policy", &format!("Policy · {} admin", shell.name), &body, None)).into_response()
}

/// What the file says about one subject.
#[derive(Default)]
struct About {
    /// Its rules, as sentences.
    may: Vec<String>,
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

fn table(shell: &Shell, apps: &[App], rules: &[Vec<String>], members: &[Vec<String>]) -> String {
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
    let rows: String = about
        .values()
        .map(|(who, a)| {
            let may = match a.may.as_slice() {
                [] => r#"<span class="quiet">Nothing of its own</span>"#.to_string(),
                may => format!("<ul>{}</ul>", may.iter().map(|m| format!("<li>{m}</li>")).collect::<String>()),
            };
            format!(r#"<tr><th scope="row">{}</th><td class="may">{may}</td></tr>"#, subject(who, a))
        })
        .collect();
    format!(r#"<div class="scroll"><table><thead><tr><th scope="col">Who</th><th scope="col">May</th></tr></thead><tbody>{rows}</tbody></table></div>"#)
}

/// Who, in words, with the groups the file links it to.
fn subject(who: &str, a: &About) -> String {
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
    let what = if what.is_empty() { String::new() } else { format!(r#"<span class="id">{}</span>"#, escape(&what)) };
    format!(r#"<span class="names"><span>{}</span>{what}</span>"#, escape(&name))
}

/// One rule as a sentence: the grants (with their risk) and where.
fn may(shell: &Shell, apps: &[App], app: &str, what: &str) -> String {
    if what == ADMIN {
        return "Use the admin screen".to_string();
    }
    let names = shell.kits.now().names();
    let offered = grants_of(what, &names);
    // A Wasm kit's grant, while the kit isn't deployed, or a typo.
    let grants: String = match offered.is_empty() {
        true => format!(r#"<b>{}</b> <span class="quiet">(no kit offers it now)</span>"#, escape(what)),
        false => offered.iter().map(|g| grant_label(g, shell.risk.of(g))).collect(),
    };
    let pattern = if offered.is_empty() || offered == [what] { String::new() } else { format!(r#" <span class="quiet">({})</span>"#, escape(what)) };
    let place = match app {
        "*" => "every app".to_string(),
        _ => match app.strip_prefix("apps:") {
            Some(group) => format!("the apps of group <b>{}</b>", escape(group)),
            // A typo shows: the app isn't here.
            None if !apps.iter().any(|a| a.id == app) => format!(r#"<b>{}</b> <span class="quiet">(no app of that id here)</span>"#, escape(app)),
            None => {
                format!(r#"<a href="/admin/apps/{}">{}</a>"#, form_urlencoded::byte_serialize(app.as_bytes()).collect::<String>(), escape(name_of(apps, app)))
            }
        },
    };
    format!("{grants}{pattern} in {place}")
}
