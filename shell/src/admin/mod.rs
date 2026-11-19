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

use askama::Template;
use axum::extract::{RawQuery, State};
use axum::response::sse::{self, KeepAlive, Sse};
use futures_util::{Stream, StreamExt};

use crate::auth::Who;
use crate::kit::{ADMIN_DEPLOY, ADMIN_OFF, ADMIN_READ, ADMIN_WRITE, Context, Grant, Kit, KitLinker, KitRequest, KitRoute, Reply, Respond};
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
            Grant::new(ADMIN_DEPLOY, "Deploy and roll back the apps the policy names, with the platform API (appos deploy, appos rollback)", &[], 0),
            Grant::new(ADMIN_OFF, "Turn the apps the policy names off and on, with the platform API (appos off, appos on)", &[], 0),
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
            // admin-deploy or admin-off for some apps is enough for a token.
            route("GET", "/admin/token", "admin-*", token::token_page),
            // A token acts as its maker, with what policy.csv gives them.
            route("POST", "/admin/token", "admin-*", token::make),
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

/// What isn't there (templates/admin/missing.html).
#[derive(askama::Template)]
#[template(path = "admin/missing.html")]
struct Missing<'a> {
    sign: String,
    what: &'a str,
}

/// A page saying `what` isn't there, with the way back to the overview.
fn missing(shell: &Shell, viewer: &Viewer, state: Tone, what: &str) -> Reply {
    let body = html(&Missing { sign: style::tone_icon(Tone::Caution, 20), what });
    Reply::document(page(shell, viewer, state, "/admin", &format!("Not found · {} admin", shell.name), &body, None)).status(404)
}

/// A template's HTML, or what went wrong drawing it.
fn html(t: &impl Template) -> String {
    t.render().unwrap_or_else(|e| format!("<p>This part could not be drawn: {}</p>", escape(&e.to_string())))
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

/// A page with the header (templates/admin/page.html); `state` is the most
/// pressing tone, for the tab's icon; `here` is the section of NAV it is in. A `live` page fetches
/// itself from that URL when the stream says something changed and swaps
/// in its `<main>` and the header's time.
fn page(shell: &Shell, viewer: &Viewer, state: Tone, here: &str, title: &str, body: &str, live: Option<&str>) -> String {
    let now = chrono::Local::now();
    let layout = Layout {
        picked: (viewer.scheme != Scheme::System).then(|| viewer.scheme.name()),
        title,
        icon: style::favicon(state),
        css: style::CSS.as_str(),
        live,
        htmx: HTMX_PATH,
        sse: SSE_PATH,
        since: if live.is_some() { shell.platform.newest().unwrap_or_default().to_string() } else { String::new() },
        name: &shell.name,
        nav: NAV.iter().map(|&(label, path)| Nav { label, path, current: path == here }).collect(),
        full: now.format("%A, %-d %B %Y, %H:%M:%S").to_string(),
        time: now.format("%H:%M:%S").to_string(),
        schemes: SCHEMES,
        scheme: &viewer.scheme,
        who: viewer.who.identity().map(|id| (id.email.as_str(), id.name.as_str())),
        changes: viewer.changes,
        body,
    };
    layout.render().unwrap_or_else(|e| format!("The page could not be drawn: {e}"))
}

/// A section in the header.
struct Nav {
    label: &'static str,
    path: &'static str,
    current: bool,
}

/// The page around every admin page: the header, and with `live` the
/// scripts, the stream on `<body>` (never swapped, so it stays connected)
/// and a hidden element that fetches the page.
#[derive(askama::Template)]
#[template(path = "admin/page.html")]
struct Layout<'a> {
    picked: Option<&'static str>,
    title: &'a str,
    icon: String,
    css: &'a str,
    live: Option<&'a str>,
    htmx: &'a str,
    sse: &'a str,
    since: String,
    name: &'a str,
    nav: Vec<Nav>,
    full: String,
    time: String,
    schemes: [Scheme; 3],
    scheme: &'a Scheme,
    who: Option<(&'a str, &'a str)>,
    changes: bool,
    body: &'a str,
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
