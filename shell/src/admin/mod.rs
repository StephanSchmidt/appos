//! The admin screen, at /admin next to the platform API: AdminKit's pages,
//! a kit built into the shell. Looking needs admin-read from policy.csv,
//! changing admin-write; a page shows its changes to those without it, but
//! they don't act. The overview: the
//! console (what needs you, every app on a screen, knobs and keys to look
//! and change, Freeze and Stop all), the last hour in numbers and charts,
//! one app in detail (its routes, versions with "Make live" and last
//! failures), the inventory, and what was refused, rolled back or turned
//! off. The log, at /admin/log: every event, newest first.
//!
//! Every change acts at once, like a control on a device: it is logged as
//! admin and the console's toast offers Undo for ten minutes.
//! The overview is live: htmx listens to `/admin/stream` and fetches the
//! page again when something happened, swapping in its main part. Its
//! scripts are its own files (`/admin/assets/`), the only ones the CSP
//! allows; without them it is a page like the others, which run no scripts.
//! Details are native tooltips and `<details>`. It follows the OS's light
//! or dark mode; a switch in the header overrides that with a cookie.
//!
//! Modules: `model` reads what the pages show, `overview` and `app` render
//! the overview, `console` its top, `actions` every change (an app off and
//! on, Make live, Seen, Freeze, Stop all, Start all, limits, Undo), `undo`
//! works out what reverses a change, `log` the log, `kpis` the numbers of
//! the last 30 days, `charts` draws, `tokens` holds the colours as numbers,
//! `style` the CSS and small visual parts, `format` turns numbers and times
//! into text.

mod actions;
mod app;
mod charts;
mod console;
mod format;
mod kpis;
mod log;
mod messages;
mod model;
mod overview;
mod policy;
mod style;
mod token;
mod tokens;
mod undo;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{RawQuery, State};
use axum::response::sse::{self, KeepAlive, Sse};
use futures_util::{Stream, StreamExt};

use crate::auth::Who;
use crate::kit::{ADMIN_READ, ADMIN_WRITE, Context, Grant, Kit, KitLinker, KitRequest, KitRoute, Reply, Respond};
use crate::platform::{Newest, now_ms};
use crate::shell::Shell;
use crate::web::{HTMX, escape};

use model::Tone;

type S = State<Arc<Shell>>;

/// The admin screen as a kit: its pages are its routes, on the platform's
/// listener; its grants are what policy.csv gives the people running the
/// platform.
pub struct AdminKit;

impl Kit for AdminKit {
    fn wit(&self) -> Vec<(String, String)> {
        vec![]
    }

    fn grants(&self) -> Vec<Grant> {
        vec![
            Grant::new(ADMIN_READ, "Look at the admin screen and read the platform API: apps, the log, messages, the policy", &[], 0),
            Grant::new(ADMIN_WRITE, "Change apps and the platform: deploy, make a version live, turn apps off and on, limits, Freeze, Stop all, Undo", &[], 0),
        ]
    }

    fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
        Ok(())
    }

    fn routes(&self) -> Vec<KitRoute> {
        let (look, change) = (ADMIN_READ, ADMIN_WRITE);
        vec![
            route("GET", "/admin", look, overview::overview),
            route("GET", "/admin/log", look, log::log_page),
            route("GET", "/admin/policy", look, policy::policy_page),
            route("GET", "/admin/messages", look, messages::messages_page),
            route("POST", "/admin/messages/:id/retry", change, messages::retry),
            route("GET", "/admin/token", look, token::token_page),
            // A token acts as its maker, with what policy.csv gives them.
            route("POST", "/admin/token", look, token::make),
            route("GET", "/admin/apps/:id", look, app::app_page),
            route("POST", "/admin/apps/:id/rollback", change, actions::make_live),
            route("POST", "/admin/apps/:id/off", change, actions::turn_off),
            route("POST", "/admin/apps/:id/on", change, actions::turn_on),
            route("POST", "/admin/apps/:id/seen", change, actions::seen),
            route("POST", "/admin/apps/:id/limits", change, actions::limits),
            route("POST", "/admin/freeze", change, actions::freeze),
            route("POST", "/admin/stop-all", change, actions::stop_all),
            route("POST", "/admin/start-all", change, actions::start_all),
            route("POST", "/admin/undo", change, actions::undo),
            route("GET", "/admin/icons/:id", look, icon),
            // Light or dark is the viewer's own.
            route("POST", "/admin/scheme", look, set_scheme),
            route("GET", "/admin/assets/:file", look, asset),
            route("GET", "/admin/*rest", look, not_found),
        ]
    }
}

/// What an admin page gets: the shell, who looks, and the request.
pub(super) struct Ask {
    shell: Arc<Shell>,
    viewer: Viewer,
    req: KitRequest,
}

impl Ask {
    /// This platform's URL as the browser reached it, for `appos login`.
    fn platform_url(&self) -> String {
        crate::web::url_of(self.req.header("host"), self.req.header("x-forwarded-proto"))
    }

    /// The path's `:id`.
    fn id(&self) -> String {
        self.req.param("id").unwrap_or_default().to_string()
    }

    /// The form, a value per name (the last).
    fn form(&self) -> BTreeMap<String, String> {
        self.req.form.iter().cloned().collect()
    }
}

/// One of AdminKit's pages, on the platform's listener: `page` answers an
/// `Ask`.
fn route<F, Fut>(method: &str, path: &str, grant: &str, page: F) -> KitRoute
where
    F: Fn(Ask) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Reply> + Send + 'static,
{
    let answer = move |cx: Context, req: KitRequest| -> Respond {
        let Some((shell, who)) = cx.own() else { return Box::pin(async { anyhow::bail!("the admin screen is the shell's own") }) };
        let scheme = req.cookie(SCHEME_COOKIE).and_then(Scheme::parse).unwrap_or(Scheme::System);
        let viewer = Viewer { who: who.clone(), scheme, changes: cx.may(ADMIN_WRITE) };
        let reply = page(Ask { shell, viewer, req });
        Box::pin(async move { Ok(reply.await) })
    };
    let route = if method == "POST" { KitRoute::post(path, grant, answer) } else { KitRoute::get(path, grant, answer) };
    route.platform()
}

/// Any other admin address: a page that says so, with the way back.
async fn not_found(ask: Ask) -> Reply {
    let Ask { shell, viewer, .. } = ask;
    read(shell, move |shell| {
        let (apps, _) = model::load(shell, now_ms());
        missing(shell, &viewer, model::state(&apps), "No such page.")
    })
    .await
}

/// A page saying `what` isn't there, with the way back to the overview.
fn missing(shell: &Shell, viewer: &Viewer, state: Tone, what: &str) -> Reply {
    let body = format!(
        r#"<div class="card summary tone-caution"><span class="sign" aria-hidden="true">{}</span><p>{} <a href="/admin"><u>Back to the overview</u></a></p></div>"#,
        style::tone_icon(Tone::Caution, 20),
        escape(what)
    );
    Reply::document(page(shell, viewer, state, "/admin", &format!("Not found · {} admin", shell.name), &body, None)).status(404)
}

/// Plain text, for a request the screen's own forms never send.
fn plain(status: u16, text: impl Into<String>) -> Reply {
    Reply::file("text/plain; charset=utf-8", text.into()).status(status)
}

/// Draws a page off the async workers: pages read platform.db, whose lock
/// and disk can wait, and requests to the apps go on meanwhile.
async fn read(shell: Arc<Shell>, draw: impl FnOnce(&Shell) -> Reply + Send + 'static) -> Reply {
    match tokio::task::spawn_blocking(move || draw(&shell)).await {
        Ok(page) => page,
        Err(e) => plain(500, format!("The page could not be drawn: {e}")),
    }
}

/// A page of the admin screen to send a browser to: "/admin" or below it,
/// with a query at most, in the characters a path and query need. So it
/// can't lead off the site: no scheme, "//", "\\", "..", space or fragment.
fn admin_path(to: &str) -> Option<&str> {
    let path = to.split('?').next().unwrap_or("");
    let under = path == "/admin" || path.starts_with("/admin/");
    let plain = to.bytes().all(|b| b.is_ascii_alphanumeric() || b"/?&=%+-._~*".contains(&b));
    let steps = path.split('/').all(|s| s != ".." && s != ".");
    (under && plain && steps && !to.contains("//")).then_some(to)
}

/// htmx 2.0.4 (the shell's copy, also served to apps) and its SSE extension
/// 2.2.2 (htmx-ext-sse on npm), both 0BSD, unchanged; their names carry the
/// version so browsers keep them. htmx 4 renamed the attributes: pinned.
const SSE_JS: &str = include_str!("../../assets/htmx-ext-sse.js");
const HTMX_PATH: &str = "/admin/assets/htmx-2.0.4.min.js";
const SSE_PATH: &str = "/admin/assets/htmx-ext-sse-2.2.2.js";

/// The admin screen's own script: ES modules, admin.js first, which
/// imports the others by these names.
const MODULES: [(&str, &str); 9] = [
    ("admin.js", include_str!("../../assets/admin.js")),
    ("nav.js", include_str!("../../assets/nav.js")),
    ("dom.js", include_str!("../../assets/dom.js")),
    ("dial.js", include_str!("../../assets/dial.js")),
    ("screen.js", include_str!("../../assets/screen.js")),
    ("knobs.js", include_str!("../../assets/knobs.js")),
    ("faders.js", include_str!("../../assets/faders.js")),
    ("pins.js", include_str!("../../assets/pins.js")),
    ("keys.js", include_str!("../../assets/keys.js")),
];

async fn asset(ask: Ask) -> Reply {
    script(ask.req.param("file").unwrap_or_default())
}

fn script(file: &str) -> Reply {
    let (body, cache) = match file {
        "htmx-2.0.4.min.js" => (HTMX, "public, max-age=31536000, immutable"),
        "htmx-ext-sse-2.2.2.js" => (SSE_JS, "public, max-age=31536000, immutable"),
        // Changes with the shell; small enough to fetch with each page.
        name => match MODULES.iter().find(|(n, _)| *n == name) {
            Some((_, body)) => (*body, "no-cache"),
            None => return plain(404, "No such file."),
        },
    };
    Reply::file("text/javascript; charset=utf-8", body).header("cache-control", cache)
}

/// How often the stream looks for news, and at least how often it says
/// "changed": while requests come in (charts), and anyway (time moves on).
const LOOK: Duration = Duration::from_secs(1);
const REQUESTS_EVERY: Duration = Duration::from_secs(5);
const ANYWAY_EVERY: Duration = Duration::from_secs(15);
const RECONNECT: Duration = Duration::from_secs(2);

/// `GET /admin/stream?since=<newest>`: a "changed" event when the page
/// should fetch itself again: within a second of an event or deploy, every
/// 5 s while requests come in, every 15 s anyway. `since` is what the page
/// showed, so nothing between drawing it and connecting is missed, and a
/// reconnect catches up at once. Not one of AdminKit's pages, which answer
/// once: web.rs serves it, for admin-read too.
pub(crate) async fn stream(State(shell): S, RawQuery(query): RawQuery) -> Sse<impl Stream<Item = Result<sse::Event, !>>> {
    let asked = query.as_deref().and_then(|q| form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "since")).and_then(|(_, v)| Newest::parse(&v));
    let since = match asked {
        Some(since) => since,
        None => shell.platform.run(|db| db.newest()).await.unwrap_or_default(),
    };
    let changes = futures_util::stream::unfold((shell, since, Instant::now()), |(shell, seen, sent)| async move {
        loop {
            tokio::time::sleep(LOOK).await;
            let now = shell.platform.run(|db| db.newest()).await.unwrap_or(seen);
            let due = if (now.events, now.versions) != (seen.events, seen.versions) {
                true
            } else if now.requests != seen.requests {
                sent.elapsed() >= REQUESTS_EVERY
            } else {
                sent.elapsed() >= ANYWAY_EVERY
            };
            if due {
                let event = sse::Event::default().event("changed").data(now_ms().to_string());
                return Some((Ok(event), (shell, now, Instant::now())));
            }
        }
    });
    // First a line saying how soon to reconnect: it also sends the headers
    // now, so the page shows "Live" at once, not with the first change.
    let hello = futures_util::stream::once(async { Ok(sse::Event::default().retry(RECONNECT)) });
    Sse::new(hello.chain(changes)).keep_alive(KeepAlive::default())
}

/// Light or dark, as picked with the switch in the header. Without a pick
/// the page follows the OS.
#[derive(Clone, Copy, PartialEq)]
enum Scheme {
    System,
    Light,
    Dark,
}

const SCHEMES: [Scheme; 3] = [Scheme::System, Scheme::Light, Scheme::Dark];
const SCHEME_COOKIE: &str = "appos-scheme";

impl Scheme {
    fn parse(s: &str) -> Option<Self> {
        SCHEMES.into_iter().find(|scheme| scheme.name() == s)
    }

    fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "Follow the system",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// A screen, a sun, a moon.
    fn icon(self) -> &'static str {
        match self {
            Self::System => r#"<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>"#,
            Self::Light => {
                r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>"#
            }
            Self::Dark => r#"<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z"/>"#,
        }
    }
}

/// The sections in the header.
const NAV: &[(&str, &str)] = &[("Overview", "/admin"), ("Log", "/admin/log"), ("Messages", "/admin/messages"), ("Policy", "/admin/policy")];

/// Who looks at a page, in which scheme, and whether they may change
/// what it shows (admin-write).
pub(super) struct Viewer {
    who: Who,
    scheme: Scheme,
    changes: bool,
}

/// The switch: keeps the pick in a cookie for a year (or forgets it for
/// System) and goes back to the page it was on.
async fn set_scheme(ask: Ask) -> Reply {
    let Some(scheme) = ask.req.form_value("scheme").and_then(Scheme::parse) else {
        return plain(400, "scheme must be system, light or dark");
    };
    let age = if scheme == Scheme::System { 0 } else { 365 * 24 * 3600 };
    let cookie = format!("{SCHEME_COOKIE}={}; Path=/admin; Max-Age={age}; SameSite=Lax; HttpOnly", scheme.name());
    Reply::redirect(&back(ask.req.header("referer"))).header("set-cookie", cookie)
}

/// The admin page a form was sent from (the Referer; the platform sends
/// it same-origin only), without its query, or the overview.
fn back(referer: Option<&str>) -> String {
    let referer = referer.unwrap_or("");
    let path = referer.split_once("://").and_then(|(_, rest)| rest.find('/').map(|i| &rest[i..])).unwrap_or("");
    admin_path(path.split(['?', '#']).next().unwrap_or("")).unwrap_or("/admin").to_string()
}

/// A page with the header; `state` is the most pressing tone, for the tab's
/// icon; `here` is the section of NAV it is in. A `live` page fetches
/// itself from that URL when the stream says something changed and swaps
/// in its `<main>` and the header's time.
fn page(shell: &Shell, viewer: &Viewer, state: Tone, here: &str, title: &str, body: &str, live: Option<&str>) -> String {
    let scheme = viewer.scheme;
    let picked = match scheme {
        Scheme::System => String::new(),
        s => format!(r#" data-scheme="{}""#, s.name()),
    };
    let switch: String = SCHEMES
        .iter()
        .map(|s| {
            format!(
                r#"<button name="scheme" value="{}" aria-pressed="{}" aria-label="{label}" title="{label}"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{}</svg></button>"#,
                s.name(),
                *s == scheme,
                s.icon(),
                label = s.label()
            )
        })
        .collect();
    let nav: String = NAV
        .iter()
        .map(|(label, path)| {
            let current = if *path == here { r#" aria-current="page""# } else { "" };
            format!(r#"<a href="{path}"{current}>{label}</a>"#)
        })
        .collect();
    // Who is signed in, and the way out; looking only, it says so.
    let looks = match viewer.changes {
        true => "",
        false => r#"<span class="looks" title="policy.csv gives you admin-read, not admin-write: the controls show, they don't act">Look only</span>"#,
    };
    let user = match viewer.who.identity() {
        Some(id) => format!(
            r#"<span class="user">{looks}<span title="{}">{}</span><form method="post" action="/_auth/sign-out"><button>Sign out</button></form></span>"#,
            escape(&id.email),
            escape(&id.name)
        ),
        None => looks.to_string(),
    };
    let body = if viewer.changes { body.to_string() } else { look_only(body) };
    let now = chrono::Local::now();
    let icon = style::favicon(state);
    // Live: the scripts, the stream on <body> (never swapped, so it stays
    // connected), and a hidden element that fetches the page.
    let (scripts, stream, refresh, updated) = match live {
        Some(url) => (
            format!(
                r#"<meta name="htmx-config" content='{{"allowEval":false,"allowScriptTags":false,"includeIndicatorStyles":false,"historyEnabled":false}}'>
<script src="{HTMX_PATH}" defer></script>
<script src="{SSE_PATH}" defer></script>
<script type="module" src="/admin/assets/admin.js"></script>
"#
            ),
            format!(r#" hx-ext="sse" sse-connect="/admin/stream?since={}""#, shell.platform.newest().unwrap_or_default()),
            format!(
                r##"<div id="refresh" hidden hx-get="{}" hx-trigger="sse:changed, refresh" hx-select-oob="#live,#updated" hx-swap="none"></div>"##,
                escape(url)
            ),
            r#" id="updated""#,
        ),
        None => (String::new(), String::new(), String::new(), ""),
    };
    format!(
        r#"<!doctype html>
<html lang="en"{picked}>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<link rel="icon" href="{icon}">
<style>{}</style>
{scripts}</head>
<body{stream}>
<div class="wrap">
<header><a class="brand" href="/admin"><span class="mark" aria-hidden="true"></span>AppOS</a><span class="platform" title="This platform">{name}</span>
<nav aria-label="Admin">{nav}</nav>
<span class="where"{updated} title="{full}"><span class="live">Live</span><span class="lost">Reconnecting</span><span class="stale">Stale</span><span>Updated {time}</span></span>
<form class="scheme" method="post" action="/admin/scheme" aria-label="Light or dark">{switch}</form>{user}</header>
{refresh}<main id="live" data-icon="{icon}">
{body}
</main>
</div>
</body>
</html>"#,
        style::CSS.as_str(),
        title = escape(title),
        name = escape(&shell.name),
        time = now.format("%H:%M:%S"),
        full = now.format("%A, %-d %B %Y, %H:%M:%S"),
    )
}

/// The page for someone who may look, not change: every form that changes
/// something is inert. The light or dark switch and the token are theirs.
fn look_only(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 256);
    let mut rest = body;
    while let Some(at) = rest.find("<form ") {
        let (before, form) = rest.split_at(at);
        let tag = &form[..form.find('>').map_or(form.len(), |end| end + 1)];
        let action = tag.split_once(r#"action=""#).and_then(|(_, a)| a.split('"').next()).unwrap_or("");
        let changes = tag.contains(r#"method="post""#) && action.starts_with("/admin/") && !["/admin/scheme", "/admin/token"].contains(&action);
        out.push_str(before);
        out.push_str(if changes { "<form inert " } else { "<form " });
        rest = &form["<form ".len()..];
    }
    out.push_str(rest);
    out
}

async fn icon(ask: Ask) -> Reply {
    match ask.shell.state.app(&ask.id()).and_then(|a| a.icon.clone()) {
        Some(svg) => Reply::file("image/svg+xml", svg),
        None => plain(404, "No such icon."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{StatusCode, header};

    fn from(referer: &str) -> String {
        back(Some(referer))
    }

    /// Every module admin.js imports, directly or not, is served as
    /// JavaScript under its own name.
    #[tokio::test]
    async fn every_imported_module_is_served() {
        for (name, body) in MODULES {
            for line in body.lines().filter(|l| l.starts_with("import ")) {
                let file = line.rsplit_once("\"./").and_then(|(_, f)| f.strip_suffix("\";")).unwrap_or_else(|| panic!("{name}: {line}"));
                assert!(MODULES.iter().any(|(n, _)| *n == file), "{name} imports {file}, which isn't served");
            }
            let r = script(name);
            assert_eq!(r.status, 200);
            assert_eq!(r.file.unwrap().0, "text/javascript; charset=utf-8");
        }
        assert_eq!(script("../admin.js").status, 404);
    }

    /// The page around the console has what the script swaps and names:
    /// #live (with its icon) as the live refresh's out-of-band part (the
    /// script redoes the page as it lands), #updated, #refresh, every id
    /// the script looks up, and the script as a module the CSP allows.
    #[tokio::test]
    async fn the_page_has_what_the_script_swaps() {
        use axum::body::{Body, to_bytes};
        use tower::ServiceExt;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        crate::deploy::deploy(&shell, &crate::testing::bundle("1", None).pack().unwrap(), crate::platform::Actor::Cli).await.unwrap();
        let script: String = MODULES.iter().map(|(_, body)| *body).collect();
        let named = |marker: &str| -> Vec<String> {
            script
                .match_indices(marker)
                .map(|(i, _)| script[i + marker.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect())
                .collect()
        };
        let ids: std::collections::BTreeSet<String> = named("getElementById(\"").into_iter().chain(named("\"#")).filter(|id| !id.is_empty()).collect();
        assert!(["live", "updated", "refresh", "console", "c-q"].iter().all(|id| ids.contains(*id)), "{ids:?}");
        for path in ["/admin", "/admin/apps/test?page=app", "/admin/apps/test?page=log", "/admin/apps/test?page=versions", "/admin/apps/test?page=map"] {
            let response =
                crate::web::platform_router(shell.clone(), None).oneshot(axum::http::Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            let csp = response.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap().to_string();
            assert!(csp.split(';').any(|d| d.trim().starts_with("script-src 'self'")), "{csp}");
            let html = String::from_utf8(to_bytes(response.into_body(), 4 << 20).await.unwrap().to_vec()).unwrap();
            assert!(html.contains(r#"<main id="live" data-icon=""#), "{path}");
            assert!(html.contains(r##"hx-select-oob="#live,#updated""##), "{path}");
            assert!(html.contains(r#"<script type="module" src="/admin/assets/admin.js"></script>"#), "{path}");
            assert!(!html.contains("<script>"), "no inline script: {path}");
            for id in &ids {
                // Find (c-q) is on the list only.
                if id != "c-q" || path == "/admin" {
                    assert!(html.contains(&format!(r#" id="{id}""#)), "{path} has no #{id}");
                }
            }
        }
    }

    #[test]
    fn the_switch_goes_back_to_admin_pages_only() {
        assert_eq!(from("http://127.0.0.1:8081/admin/apps/calc?msg=x#detail"), "/admin/apps/calc");
        assert_eq!(from("https://ops.example.com/admin"), "/admin");
        assert_eq!(from("https://ops.example.com/administrator"), "/admin");
        assert_eq!(from("https://ops.example.com//evil.example/admin"), "/admin");
        assert_eq!(from("https://evil.example"), "/admin");
        assert_eq!(from("https://ops.example.com/admin/../x"), "/admin");
        assert_eq!(back(None), "/admin");
    }

    #[test]
    fn changes_go_back_to_admin_pages_with_their_query() {
        assert_eq!(admin_path("/admin/apps/a+b?page=log&at=60000"), Some("/admin/apps/a+b?page=log&at=60000"));
        assert_eq!(admin_path("/admin"), Some("/admin"));
        for bad in ["https://evil.example/admin", "//evil.example/admin", "/admin//evil", "/admin/./x", "/admin#x", "/admin\\x", "/admin x", "/admins", ""] {
            assert_eq!(admin_path(bad), None, "{bad}");
        }
    }
}
