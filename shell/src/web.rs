//! HTTP: the shell's own pages, and dispatch of `/<app-id>/...` to apps.
//!
//! The shell owns the page: layout, menu, scripts. Per request an app sets
//! named HTML ("page" is the whole page content), may add to `<head>`, and
//! calls done. For htmx requests every name but "page" is swapped into the
//! element with that id. htmx swaps 422 (invalid input) like 200, so an app
//! can show what's wrong with a form.
//!
//! A host call the policy denies ends the request (access.rs, `denied`);
//! with `--no-auth` there is no policy and every route is public. The admin
//! screen is for those the policy makes admins (`admin_only`), and so is the
//! platform API, with a token from `appos login` (`api_only`). Both answer
//! only under their own names (`own_host`).

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequest, Path, Query, RawQuery, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde::Serialize;

use crate::api;
use crate::auth::Who;
use crate::deploy::Ask;
use crate::log_filter;
use crate::platform::{Actor, Kind, Outcome, Usage, now_ms};
use crate::runtime::{self, LoadedApp, QUEUE_WAIT};
use crate::shell::Shell;
use crate::{terminal_errorln as eprintln, terminal_logln as println};

pub(crate) const HTMX: &str = include_str!("../assets/htmx.min.js");
const CSS: &str = include_str!("../assets/app.css");

/// Only the shell's own scripts may run. Apps bring their own design:
/// <style> in the head (page::add-header) and style="" attributes.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                   form-action 'self'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'";

/// Icons are untrusted documents, including when opened directly on the
/// private admin origin. They must not load the shell's trusted scripts
/// or acquire that origin's ability to submit admin actions.
const ICON_CSP: &str = "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src 'self' data:; form-action 'none'; frame-ancestors 'none'; base-uri 'none'; sandbox";

type S = State<Arc<Shell>>;

pub fn router(shell: Arc<Shell>) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/assets/htmx.min.js", get(|| async { ([(header::CONTENT_TYPE, "text/javascript")], HTMX) }))
        .route("/assets/app.css", get(|| async { ([(header::CONTENT_TYPE, "text/css")], CSS) }))
        .route("/assets/icons/{id}", get(icon))
        .merge(crate::auth::router(&shell.origin))
        .fallback(dispatch)
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(axum::middleware::from_fn_with_state(shell.clone(), crate::auth::identify))
        .layer(axum::middleware::from_fn(same_origin_writes))
        .layer(axum::middleware::from_fn_with_state(own_hosts(&[&shell.origin]), own_host))
        .layer(axum::middleware::map_response(security_headers))
        .with_state(shell)
}

/// Answers only under its own names, so a web page can't reach the shell
/// under a name of the page's own (DNS rebinding) and use what is open
/// there: the hosts of `origins`, localhost, and IP addresses, which no
/// other site's page can be served from. Without a Host, it isn't a
/// browser.
async fn own_host(State(own): State<Arc<[String]>>, req: Request, next: axum::middleware::Next) -> Response {
    match req.headers().get(header::HOST).map(|h| h.to_str().unwrap_or("")) {
        Some(host) if !known_host(host, &own) => (
            StatusCode::MISDIRECTED_REQUEST,
            format!(
                "This shell answers to {}, localhost and IP addresses, not {host:?}. Behind a proxy, give --origin the address people use.",
                own.join(", ")
            ),
        )
            .into_response(),
        _ => next.run(req).await,
    }
}

/// The hosts of `origins`, as `own_host` compares them.
fn own_hosts(origins: &[&str]) -> Arc<[String]> {
    origins.iter().filter_map(|o| o.parse::<axum::http::Uri>().ok()?.host().map(plain_host)).collect()
}

fn plain_host(host: &str) -> String {
    host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase()
}

fn known_host(host: &str, own: &[String]) -> bool {
    let Ok(authority) = host.parse::<axum::http::uri::Authority>() else { return false };
    let host = plain_host(authority.host());
    host.parse::<std::net::IpAddr>().is_ok() || host == "localhost" || host.ends_with(".localhost") || own.contains(&host)
}

async fn security_headers(resp: Response) -> Response {
    common_headers(resp, CSP)
}

/// The headers of every response: `csp`, or ICON_CSP for an icon.
fn common_headers(mut resp: Response, csp: &'static str) -> Response {
    let svg = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.split(';').next().is_some_and(|mime| mime.trim().eq_ignore_ascii_case("image/svg+xml")));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(if svg { ICON_CSP } else { csp }));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("same-origin"));
    resp
}

/// A query string or form body as pairs.
pub(crate) fn pairs(query: Option<&str>) -> Vec<(String, String)> {
    query.map(|q| form_urlencoded::parse(q.as_bytes()).into_owned().collect()).unwrap_or_default()
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

// ---- layout ----------------------------------------------------------------

/// Where a menu entry's `path` of app `id` is.
fn href(id: &str, path: &str) -> String {
    if path == "/" { format!("/{id}") } else { format!("/{id}{path}") }
}

/// Every app's menu entries, as (label, href).
fn menu(shell: &Shell) -> Vec<(String, String)> {
    let mut links = Vec::new();
    for (id, app) in shell.state.apps() {
        for entry in &app.menu {
            links.push((entry.label.clone(), href(&id, &entry.path)));
        }
    }
    links
}

/// The full page. `head` is HTML from the app; if it has a <title>, that
/// replaces the default `title`.
/// The menu entry the page at `current` is under: of /survey and
/// /survey/results, only the second on /survey/results.
fn active<'a>(links: &'a [(String, String)], current: &str) -> Option<&'a String> {
    links
        .iter()
        .map(|(_, href)| href)
        .filter(|href| current.strip_prefix(href.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')))
        .max_by_key(|href| href.len())
}

fn layout(shell: &Shell, who: &Who, current: &str, title: &str, head: &[String], main: &str) -> String {
    let user = user_bar(shell, who, current);
    let links = menu(shell);
    let active = active(&links, current);
    let nav: String = links
        .iter()
        .map(|(label, href)| {
            let class = if Some(href) == active { r#" class="active""# } else { "" };
            format!(r#"<a href="{}"{class}>{}</a>"#, escape(href), escape(label))
        })
        .collect();
    // The tab shows the app's icon.svg, unless the app set an icon itself.
    let id = current.trim_start_matches('/').split('/').next().unwrap_or("");
    let has_icon = shell.state.app(id).is_some_and(|a| a.icon.is_some());
    let icon = if has_icon && !head.iter().any(|h| h.contains("rel=\"icon\"")) {
        format!("<link rel=\"icon\" href=\"/assets/icons/{}\">\n", escape(id))
    } else {
        String::new()
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="htmx-config" content='{{"includeIndicatorStyles":false,"responseHandling":[{{"code":"204","swap":false}},{{"code":"[23]..","swap":true}},{{"code":"409","swap":true}},{{"code":"422","swap":true}},{{"code":"[45]..","swap":false,"error":true}}]}}'>
{title}{icon}<link rel="stylesheet" href="/assets/app.css">
<script src="/assets/htmx.min.js" defer></script>
{head}
</head>
<body>
<header class="appos-topbar"><a class="appos-brand" href="/">AppOS</a><nav>{nav}</nav>{user}</header>
<main>{main}</main>
</body>
</html>"#,
        title = if head.iter().any(|h| h.contains("<title")) { String::new() } else { format!("<title>{}</title>\n", escape(title)) },
        head = head.join("\n"),
    )
}

/// One of the shell's own pages: a heading and a sentence.
pub(crate) fn message(shell: &Shell, who: &Who, status: StatusCode, title: &str, text: &str) -> Response {
    let main = format!(r#"<div class="appos-page"><h1>{}</h1><p>{}</p></div>"#, escape(title), escape(text));
    page(shell, who, status, title, &main)
}

/// One of the shell's own pages, `main` being its HTML.
pub(crate) fn page(shell: &Shell, who: &Who, status: StatusCode, title: &str, main: &str) -> Response {
    (status, Html(layout(shell, who, "", title, &[], main))).into_response()
}

/// The right end of the top bar: sign in, or who is signed in and sign out.
/// Nothing without sign-in.
fn user_bar(shell: &Shell, who: &Who, current: &str) -> String {
    if shell.auth.is_none() {
        return String::new();
    }
    match who.identity() {
        Some(id) => format!(
            r#"<div class="appos-user"><span title="{}">{}</span><form method="post" action="/_auth/sign-out"><button>Sign out</button></form></div>"#,
            escape(&id.email),
            escape(&id.name)
        ),
        None => {
            let to =
                if current.is_empty() { String::new() } else { format!("?to={}", form_urlencoded::byte_serialize(current.as_bytes()).collect::<String>()) };
            format!(r#"<div class="appos-user"><a href="/_auth/sign-in{}">Sign in</a></div>"#, escape(&to))
        }
    }
}

// ---- shell pages -----------------------------------------------------------

/// Every app once, by name, with its icon; it links to its first menu
/// entry.
async fn home(State(shell): S, who: Who) -> Response {
    let mut apps: Vec<(String, String, bool)> =
        shell.state.apps().iter().map(|(id, a)| (a.manifest.app.name.clone(), href(id, a.menu.first().map_or("/", |m| &m.path)), a.icon.is_some())).collect();
    apps.sort();
    let list: String = if apps.is_empty() {
        r#"<p class="appos-muted">No apps yet.</p>"#.into()
    } else {
        let items: String = apps
            .iter()
            .map(|(name, href, icon)| {
                let id = href.trim_start_matches('/').split('/').next().unwrap_or("");
                let img = if *icon { format!(r#"<img src="/assets/icons/{}" alt="" width="40" height="40">"#, escape(id)) } else { String::new() };
                format!(
                    r#"<a class="appos-card" href="{}">{img}<strong>{}</strong><span class="appos-muted">{}</span></a>"#,
                    escape(href),
                    escape(name),
                    escape(href)
                )
            })
            .collect();
        format!(r#"<div class="appos-apps">{items}</div>"#)
    };
    let main = format!(r#"<div class="appos-page"><h1>Apps</h1>{list}</div>"#);
    Html(layout(&shell, &who, "/", "Apps", &[], &main)).into_response()
}

/// An app's icon.svg. The document policy also blocks trusted external
/// scripts when someone opens the icon directly.
async fn icon(State(shell): S, Path(id): Path<String>) -> Response {
    let icon = shell.state.app(&id).and_then(|a| a.icon.clone());
    match icon {
        Some(svg) => ([(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "no-cache")], svg).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ---- platform API, for the appos CLI --------------------------------------

/// The contract apps build against, served to `appos new`.
const WIT: &[(&str, &str)] = &[("appos.wit", include_str!("../../wit/appos.wit"))];
/// The contract of sign-in plugins (auth/plugin.rs).
const SIGN_IN_WIT: &str = include_str!("../../wit-auth/auth.wit");

/// The WIT package apps build against, as appos.wit names it:
/// "appos:framework@0.1.0".
fn framework() -> &'static str {
    WIT[0].1.lines().find_map(|l| l.trim().strip_prefix("package ")?.strip_suffix(';')).unwrap_or_default()
}

/// A platform API answer: its JSON, or what went wrong.
fn json<T: Serialize>(result: anyhow::Result<T>) -> Response {
    match result {
        Ok(body) => axum::Json(body).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

/// The platform API and the admin screen. `own` is this listener's origin
/// when it listens on its own (`--platform-addr`), with sign-in pages of its
/// own; merged into the site's listener it uses the site's.
pub fn platform_router(shell: Arc<Shell>, own: Option<&str>) -> Router {
    let api = Router::new()
        .route("/_platform/me", get(me))
        .route("/_platform/deploy", post(deploy).layer(DefaultBodyLimit::max(crate::bundle::MAX)))
        .route("/_platform/apps/{id}/versions", get(versions))
        .route("/_platform/packages", get(packages))
        .route("/_platform/apps/{id}/news", get(news))
        .route("/_platform/log", get(log))
        .route("/_platform/apps/{id}/rollback", post(rollback))
        .route("/_platform/apps/{id}/off", post(turn_off))
        .route("/_platform/apps/{id}/on", post(turn_on))
        .route_layer(axum::middleware::from_fn_with_state(shell.clone(), api_only));
    let router = Router::new()
        .route("/_platform/spec", get(spec))
        .route("/_platform/install", get(install))
        .route("/_platform/logout", post(logout))
        .merge(api)
        .merge(crate::admin::router().route_layer(axum::middleware::from_fn_with_state(shell.clone(), admin_only)));
    let router = match own {
        // Its home is the admin screen, where signing out lands too.
        Some(origin) => router.route("/", get(|| async { go("/admin", false) })).merge(crate::auth::router(origin)),
        None => router,
    };
    let origins: Vec<&str> = [Some(shell.origin.as_str()), own].into_iter().flatten().collect();
    router
        .layer(axum::middleware::from_fn_with_state(shell.clone(), crate::auth::identify))
        .layer(axum::middleware::from_fn(same_origin_writes))
        .layer(axum::middleware::from_fn_with_state(own_hosts(&origins), own_host))
        .layer(axum::middleware::map_response(platform_headers))
        .with_state(shell)
}

/// The platform API takes a token from `appos login` (auth/tokens.rs) as
/// `Authorization: Bearer`, of someone the policy makes an admin, checked
/// at every call; a browser's cookie doesn't count. Without a policy
/// (`--no-auth`) it is open. Spec, install and logout need none.
async fn api_only(State(shell): S, mut req: Request, next: axum::middleware::Next) -> Response {
    let Some(policies) = &shell.policy else { return next.run(req).await };
    let login = format!("appos login {}", platform_url(req.headers()));
    let refuse = |status: StatusCode, why: String| {
        let mut resp = (status, why).into_response();
        if status == StatusCode::UNAUTHORIZED {
            resp.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        resp
    };
    let Some(token) = crate::auth::tokens::bearer(req.headers()) else {
        return refuse(StatusCode::UNAUTHORIZED, format!("{} takes a token: {login}", shell.name));
    };
    let identity = match crate::auth::tokens::who(&shell, token).await {
        Ok(Some(identity)) => identity,
        Ok(None) => return refuse(StatusCode::UNAUTHORIZED, format!("this token is unknown, expired or logged out: {login}")),
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("could not check the token: {e:#}")),
    };
    let who = Who::signed_in(identity);
    if !policies.current().may(&who, crate::access::ADMIN) {
        let email = who.person().unwrap_or_default();
        return refuse(StatusCode::FORBIDDEN, format!("{email} may not use the platform API: the policy gives it to admins"));
    }
    req.extensions_mut().insert(who);
    next.run(req).await
}

/// `appos login`: whom the token signs in.
async fn me(who: Who) -> Response {
    match who.identity() {
        Some(id) => axum::Json(api::Me { email: id.email.clone(), name: id.name.clone() }).into_response(),
        None => (StatusCode::NOT_FOUND, "this platform runs with --no-auth: it takes no token").into_response(),
    }
}

/// `appos logout`: ends the token it is sent with.
async fn logout(State(shell): S, headers: HeaderMap) -> Response {
    let Some(token) = crate::auth::tokens::bearer(&headers) else {
        return (StatusCode::BAD_REQUEST, "send the token to end as Authorization: Bearer").into_response();
    };
    match crate::auth::tokens::drop(&shell, token).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "no such token: it ended already").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("could not end the token: {e:#}")).into_response(),
    }
}

/// The admin screen (pages, changes, the stream, its assets) is for those
/// policy.csv gives `admin`; without a policy, for whoever reaches the
/// platform listener. The platform API under `/_platform` has its own
/// gate, with a token instead of a cookie (`api_only`).
async fn admin_only(State(shell): S, who: Who, req: Request, next: axum::middleware::Next) -> Response {
    let Some(policies) = &shell.policy else { return next.run(req).await };
    if policies.current().may(&who, crate::access::ADMIN) {
        return next.run(req).await;
    }
    let htmx = req.headers().contains_key("hx-request");
    if shell.auth.is_some() && who.identity().is_none() {
        let back = match *req.method() {
            Method::GET | Method::HEAD => req.uri().path_and_query().map_or("/admin", |p| p.as_str()),
            _ => "/admin",
        };
        let to: String = form_urlencoded::byte_serialize(back.as_bytes()).collect();
        return go(&format!("/_auth/sign-in?to={to}"), htmx);
    }
    message(&shell, &who, StatusCode::FORBIDDEN, "Not allowed", "The admin screen is for admins, and policy.csv says who they are.")
}

/// Local/internal listeners are still reachable through a visitor's
/// browser. Refuse foreign browser writes while allowing CLI requests,
/// which do not carry Origin or Fetch Metadata headers.
async fn same_origin_writes(req: Request, next: axum::middleware::Next) -> Response {
    if !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        let headers = req.headers();
        let foreign_site = headers.get("sec-fetch-site").is_some_and(|site| site != "same-origin" && site != "none");
        let foreign_origin = headers.get(header::ORIGIN).is_some_and(|origin| {
            let same_origin = || -> Option<bool> {
                let origin: axum::http::Uri = origin.to_str().ok()?.parse().ok()?;
                let scheme = origin.scheme_str()?;
                // Axum serves HTTP; a TLS-terminating proxy supplies the
                // original scheme. Browsers cannot add this header to a
                // cross-origin form submission.
                let request_scheme = req.uri().scheme_str().or_else(|| headers.get("x-forwarded-proto")?.to_str().ok()).unwrap_or("http");
                if scheme != request_scheme {
                    return Some(false);
                }
                let default_port = match scheme {
                    "http" => 80,
                    "https" => 443,
                    _ => return None,
                };
                let authority = origin.authority()?;
                let host: axum::http::uri::Authority = headers.get(header::HOST)?.to_str().ok()?.parse().ok()?;
                Some(
                    !authority.as_str().contains('@')
                        && origin.path() == "/"
                        && origin.query().is_none()
                        && authority.host().eq_ignore_ascii_case(host.host())
                        && authority.port_u16().unwrap_or(default_port) == host.port_u16().unwrap_or(default_port),
                )
            };
            same_origin() != Some(true)
        });
        if foreign_site || foreign_origin {
            return (StatusCode::FORBIDDEN, "writes must come from the same origin").into_response();
        }
    }
    next.run(req).await
}

/// The admin screen runs only its own scripts (htmx, its SSE extension and
/// admin.js, from /admin/assets/) and talks only to the platform; icons are
/// shown as <img>, and an app's icon.svg opened on its own runs nothing.
const PLATFORM_CSP: &str = "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; img-src 'self' data:; \
                            form-action 'self'; frame-ancestors 'none'; base-uri 'none'";

async fn platform_headers(resp: Response) -> Response {
    let mut resp = common_headers(resp, PLATFORM_CSP);
    let h = resp.headers_mut();
    if !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    resp
}

/// `appos deploy`: the body is app.zip (bundle.rs); the shell loads the app
/// and makes it live (see deploy.rs). An app the shell refuses gets 422 and
/// why.
async fn deploy(State(shell): S, who: Who, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    match crate::deploy::deploy_as(&shell, &body, Actor::Cli, who.person()).await {
        Ok(crate::deploy::Deployed { app, held }) => {
            let m = &app.manifest.app;
            let status = match &held {
                None => {
                    println!("deployed {} {} at /{} ({} routes)", m.id, app.label, m.id, app.routes.len());
                    StatusCode::OK
                }
                Some(why) => {
                    println!("held {} {}: {why}", m.id, app.label);
                    StatusCode::ACCEPTED
                }
            };
            let admin = held.is_some().then(|| format!("{}/admin/apps/{}?page=versions#console", platform_url(&headers), m.id));
            let body = api::Deployed {
                id: m.id.clone(),
                name: m.name.clone(),
                version: m.version.clone(),
                label: app.label.clone(),
                url: format!("{}/{}", shell.origin, m.id),
                held,
                admin,
            };
            (status, axum::Json(body)).into_response()
        }
        Err(e) => {
            eprintln!("refused deploy: {e:#}");
            (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")).into_response()
        }
    }
}

/// `appos versions`: an app's versions, newest first.
async fn versions(State(shell): S, Path(id): Path<String>) -> Response {
    let app_id = id.clone();
    match shell.platform.run(move |db| db.versions(&app_id, 50)).await {
        Ok(v) if v.is_empty() => (StatusCode::NOT_FOUND, format!("{id} was never deployed here")).into_response(),
        result => json(result),
    }
}

/// `appos audit`: what each live app is built with.
async fn packages(State(shell): S) -> Response {
    let live = shell.platform.run(|db| db.live_packages()).await;
    json(live.map(|live| {
        live.into_iter()
            .map(|(v, packages)| crate::api::LivePackages {
                id: v.app_id,
                name: v.name,
                version: v.version,
                label: v.label,
                owner: v.owner,
                group: v.group,
                packages,
            })
            .collect::<Vec<_>>()
    }))
}

/// What a builder hears about: the platform or an admin acting on their app.
const NEWS: [Kind; 6] = [Kind::AutoRolledBack, Kind::RolledBack, Kind::Deployed, Kind::Refused, Kind::TurnedOff, Kind::TurnedOn];
/// Without an earlier look, the news of this long.
const FIRST_LOOK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// At most this many, the newest.
const NEWS_MAX: usize = 10;

#[derive(serde::Deserialize)]
struct Since {
    /// The `now` of the last look, unix ms.
    since: Option<i64>,
}

/// `appos deploy` and `appos versions`: what happened to an app since the
/// builder last looked (rollbacks, refusals, versions an admin made live,
/// turned off or on), whether its deploys wait for an admin and whether it
/// is off. `now` is the next `since`.
async fn news(State(shell): S, headers: HeaderMap, Path(id): Path<String>, Query(q): Query<Since>) -> Response {
    let now = now_ms();
    let since = match q.since {
        None => now - FIRST_LOOK_MS,
        Some(s) => match s.checked_add(1) {
            Some(since) => since,
            None => return (StatusCode::BAD_REQUEST, "since is outside the supported timestamp range").into_response(),
        },
    };
    let admin = format!("{}/admin/apps/{id}", platform_url(&headers));
    json(
        shell
            .platform
            .run(move |db| {
                let mut events: Vec<_> =
                    db.events_since(Some(&id), since, &NEWS)?.into_iter().filter(|e| e.kind != Kind::Deployed || e.actor == Actor::Admin).collect();
                let more = events.len().saturating_sub(NEWS_MAX);
                events.drain(..more);
                let events = events
                    .into_iter()
                    .map(|e| api::NewsEvent { at: e.at, label: e.label, kind: e.kind, actor: e.actor, message: e.message, reason: e.reason })
                    .collect();
                // The console's pages: its versions, and the app with its on/off slide.
                let paused = db.paused(&id)?.map(|e| api::Paused { at: e.at, reason: e.reason, admin: format!("{admin}?page=versions#console") });
                let off = db.off(&id)?.map(|e| api::Off { at: e.at, reason: e.reason, actor: e.actor, admin: format!("{admin}#console") });
                Ok(api::News { now, events, more, paused, off })
            })
            .await,
    )
}

/// Events on a page of `/_platform/log`, unless `limit` says otherwise,
/// and the most it may say.
const LOG_PAGE: u32 = 50;
const LOG_MAX: u32 = 500;
/// Failed requests told with an event.
const LOG_FAILED: u32 = 5;

/// `appos log`, and any agent: the log, newest first, narrowed like the
/// admin's (log_filter.rs), `limit` events at a time. `older` is the next
/// page's `before`. An automatic rollback or a limit hit carries the
/// failed requests behind it with their errors: why the app failed.
async fn log(State(shell): S, RawQuery(query): RawQuery) -> Response {
    let pairs = pairs(query.as_deref());
    let (filters, before) = match log_filter::Filters::parse(&pairs) {
        Ok(parsed) => parsed,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let limit = match pairs.iter().find(|(k, _)| k == "limit").map(|(_, v)| v.parse::<u32>()) {
        None => LOG_PAGE,
        Some(Ok(n)) if (1..=LOG_MAX).contains(&n) => n,
        Some(_) => return (StatusCode::BAD_REQUEST, format!("limit is a number from 1 to {LOG_MAX}")).into_response(),
    };
    let q = filters.query(before);
    json(
        shell
            .platform
            .run(move |db| {
                let mut events = db.log(&q, limit + 1)?;
                let older = (events.len() > limit as usize).then(|| events[limit as usize - 1].id);
                events.truncate(limit as usize);
                let mut out = vec![];
                for e in events {
                    let failed = crate::usage::behind(db, &e, LOG_FAILED)?.map(|(reqs, count)| api::Failed {
                        count,
                        first: reqs
                            .into_iter()
                            .map(|r| api::FailedRequest {
                                at: r.at,
                                label: r.label.to_string(),
                                route: r.route.to_string(),
                                outcome: r.outcome,
                                wall_us: r.wall_us,
                                error: r.error,
                            })
                            .collect(),
                    });
                    out.push(api::LogEvent {
                        id: e.id,
                        at: e.at,
                        app: e.app_id,
                        label: e.label,
                        kind: e.kind,
                        severity: e.severity,
                        actor: e.actor,
                        message: e.message,
                        reason: e.reason,
                        risk: e.risk,
                        recovery_ms: e.recovery_ms,
                        person: e.person,
                        failed,
                    });
                }
                Ok(api::LogPage { events: out, older })
            })
            .await,
    )
}

/// `appos off`: the kill switch.
async fn turn_off(State(shell): S, who: Who, Path(id): Path<String>, axum::Json(req): axum::Json<api::SwitchRequest>) -> Response {
    turn(&shell, &who, &id, false, req).await
}

/// `appos on`, unless an admin turned it off.
async fn turn_on(State(shell): S, who: Who, Path(id): Path<String>, axum::Json(req): axum::Json<api::SwitchRequest>) -> Response {
    turn(&shell, &who, &id, true, req).await
}

async fn turn(shell: &Shell, who: &Who, id: &str, on: bool, req: api::SwitchRequest) -> Response {
    let reason = req.reason.as_deref().and_then(crate::deploy::reason);
    match shell.state.turn(id, on, Actor::Cli, who.person(), reason).await {
        Ok(_) => {
            println!("turned {id} {}", if on { "on" } else { "off" });
            axum::Json(api::Switched { id: id.to_string(), on, url: format!("{}/{id}", shell.origin) }).into_response()
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")).into_response(),
    }
}

/// `appos rollback`: makes an earlier version live again.
async fn rollback(State(shell): S, who: Who, Path(id): Path<String>, axum::Json(req): axum::Json<api::RollbackRequest>) -> Response {
    let reason = req.reason.as_deref().and_then(crate::deploy::reason);
    match crate::deploy::rollback(&shell, &id, req.label.as_deref(), Ask::Cli { reason, person: who.person() }).await {
        Ok((v, _)) => {
            println!("rolled {id} back to {}", v.label);
            axum::Json(api::RolledBack { url: format!("{}/{id}", shell.origin), id, label: v.label }).into_response()
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")).into_response(),
    }
}

/// What this platform offers: `appos login` saves it, `appos hosts` shows it,
/// `appos new` writes its WIT files into the app.
async fn spec(State(shell): S) -> Response {
    let apps = shell
        .state
        .apps()
        .values()
        .map(|a| api::SpecApp { id: a.manifest.app.id.clone(), name: a.manifest.app.name.clone(), version: a.manifest.app.version.clone() })
        .collect();
    axum::Json(api::Spec {
        name: shell.name.clone(),
        origin: shell.origin.clone(),
        framework: framework().to_string(),
        capabilities: shell.kits.names(),
        risk: shell.risk.levels().clone(),
        apps,
        git: shell.git.clone(),
        wit: WIT.iter().map(|(n, t)| (n.to_string(), t.to_string())).chain(shell.kits.wit()).collect(),
        sign_in: std::collections::BTreeMap::from([("auth.wit".to_string(), SIGN_IN_WIT.to_string())]),
        token: shell.policy.is_some(),
    })
    .into_response()
}

/// This platform's URL as the client reached it: what `appos login` takes.
pub fn platform_url(headers: &HeaderMap) -> String {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("127.0.0.1:8080");
    let scheme = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).filter(|s| matches!(*s, "http" | "https")).unwrap_or("http");
    format!("{scheme}://{host}")
}

/// `curl -fsSL http://<platform>/_platform/install | sh` connects the local
/// appos CLI to this shell, like a kubeconfig.
async fn install(headers: HeaderMap) -> Response {
    // Host is request data. Single quotes prevent command substitution;
    // embedded single quotes must leave and re-enter the quoted string.
    let url = platform_url(&headers).replace('\'', "'\"'\"'");
    let script = format!(
        r#"#!/bin/sh
set -e
if ! command -v appos >/dev/null 2>&1; then
  echo "appos is not installed: brew install stephanschmidt/appos/appos" >&2
  exit 1
fi
appos login '{url}'
"#
    );
    ([(header::CONTENT_TYPE, "text/x-shellscript")], script).into_response()
}

// ---- app dispatch ----------------------------------------------------------

/// Matches a route against the path below the app's base.
fn match_route<'a>(app: &'a LoadedApp, method: &str, rest: &[&str]) -> Option<(&'a runtime::Route, Vec<(String, String)>)> {
    'routes: for route in &app.routes {
        if route.method != method || route.segments.len() != rest.len() {
            continue;
        }
        let mut params = Vec::new();
        for (pattern, actual) in route.segments.iter().zip(rest) {
            match pattern.strip_prefix(':') {
                Some(name) => params.push((name.to_string(), actual.to_string())),
                None if pattern == actual => {}
                None => continue 'routes,
            }
        }
        return Some((route, params));
    }
    None
}

async fn dispatch(State(shell): S, who: Who, req: Request) -> Response {
    // The bundled htmx also sends HX-Request on history cache misses, which
    // replace the whole body and therefore require the complete document.
    let htmx = {
        let headers = req.headers();
        let is_true = |name: &str| headers.get(name).is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"true"));
        is_true("hx-request") && !is_true("hx-history-restore-request")
    };
    let mut response = dispatch_app(shell, who, req, htmx).await;
    response.headers_mut().append(header::VARY, HeaderValue::from_static("HX-Request, HX-History-Restore-Request"));
    response
}

/// A request to an app, read: its path below the app, query and form.
struct AppRequest {
    method: Method,
    path: String,
    query: Vec<(String, String)>,
    form: Vec<(String, String)>,
}

async fn dispatch_app(shell: Arc<Shell>, who: Who, req: Request, htmx: bool) -> Response {
    let back = back_to(&shell, &req, htmx);
    let req = match read_request(&shell, req).await {
        Ok(req) => req,
        Err(e) => return e.into_response(),
    };
    let decoded: Vec<String> = req.path.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
    let Some((app_id, rest)) = decoded.split_first() else {
        return message(&shell, &who, StatusCode::NOT_FOUND, "Not found", "No such page.");
    };
    let Some(app) = shell.state.app(app_id) else {
        return message(&shell, &who, StatusCode::NOT_FOUND, "Not found", "No app lives here.");
    };
    if shell.state.is_off(app_id) {
        return turned_off(&shell, &who, &app);
    }
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    let Some((route, params)) = match_route(&app, req.method.as_str(), &rest) else {
        return message(&shell, &who, StatusCode::NOT_FOUND, "Not found", "This app has no such page.");
    };
    let request = runtime::Request {
        method: req.method.to_string(),
        path: format!("/{}", rest.join("/")),
        params,
        query: req.query,
        form: req.form,
        base: format!("/{app_id}"),
        origin: shell.origin.clone(),
        htmx,
        now: chrono::Local::now().to_rfc3339(),
    };
    let back = back.unwrap_or_else(|| format!("/{app_id}"));
    run(&shell, &who, &app, route, request, &req.path, &back).await
}

/// The page to come back to after signing in: this one for a GET, the one
/// a form or an htmx request was sent from otherwise. None if that is not
/// on this site.
fn back_to(shell: &Shell, req: &Request, htmx: bool) -> Option<String> {
    let page = |name: HeaderName| {
        let url = req.headers().get(name)?.to_str().ok()?;
        url.strip_prefix(shell.origin.as_str()).filter(|p| runtime::is_local(p)).map(str::to_string)
    };
    if htmx {
        page(HeaderName::from_static("hx-current-url"))
    } else if matches!(*req.method(), Method::GET | Method::HEAD) {
        req.uri().path_and_query().map(|p| p.as_str().to_string())
    } else {
        page(header::REFERER)
    }
}

/// The policy doesn't let this person do what the app tried: sign in
/// first, or a 403. htmx drops the body of a 4xx, so it is sent to the
/// 403 page instead.
fn denied(shell: &Shell, who: &Who, app: &LoadedApp, back: &str, htmx: bool) -> Response {
    if who.identity().is_none() && shell.auth.is_some() {
        let to: String = form_urlencoded::byte_serialize(back.as_bytes()).collect();
        return go(&format!("/_auth/sign-in?to={to}"), htmx);
    }
    if htmx {
        let id: String = form_urlencoded::byte_serialize(app.manifest.app.id.as_bytes()).collect();
        return go(&format!("/_auth/denied?app={id}"), htmx);
    }
    denied_page(shell, who, Some(app))
}

/// "You can't do this here", with whom to ask.
pub(crate) fn denied_page(shell: &Shell, who: &Who, app: Option<&LoadedApp>) -> Response {
    let text = match app.map(|a| &a.manifest.app) {
        Some(m) => match m.owner.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
            Some(owner) => format!("You can't do this in {}. If you should, ask its owner, {owner}.", m.name),
            None => format!("You can't do this in {}.", m.name),
        },
        None => "You can't do this here.".to_string(),
    };
    message(shell, who, StatusCode::FORBIDDEN, "Not allowed", &text)
}

/// Sends the browser to `to`; htmx is told to go there itself, as it
/// would follow a 303 and swap the page in.
fn go(to: &str, htmx: bool) -> Response {
    let (status, name) = if htmx { (StatusCode::OK, HeaderName::from_static("hx-redirect")) } else { (StatusCode::SEE_OTHER, header::LOCATION) };
    let mut resp = status.into_response();
    if let Ok(v) = HeaderValue::try_from(to) {
        resp.headers_mut().insert(name, v);
    }
    resp
}

/// Reads the method, path, query and form; a body that is no form gets 400.
async fn read_request(shell: &Arc<Shell>, req: Request) -> Result<AppRequest, (StatusCode, String)> {
    // HEAD (link checkers, uptime monitors) is a GET without the body;
    // hyper leaves the body out.
    let method = if req.method() == Method::HEAD { Method::GET } else { req.method().clone() };
    let path = req.uri().path().to_string();
    let query = pairs(req.uri().query());
    let body = axum::body::Bytes::from_request(req, shell).await.map_err(|e| (e.status(), e.body_text()))?;
    let form = if method == Method::GET {
        vec![]
    } else {
        let text = std::str::from_utf8(&body).map_err(|_| (StatusCode::BAD_REQUEST, "form body is not UTF-8".to_string()))?;
        pairs(Some(text))
    };
    Ok(AppRequest { method, path, query, form })
}

fn turned_off(shell: &Shell, who: &Who, app: &LoadedApp) -> Response {
    let name = &app.manifest.app.name;
    message(shell, who, StatusCode::SERVICE_UNAVAILABLE, "Turned off", &format!("{name} is turned off for now. Try again later."))
}

/// Runs the route's handler once a permit is free and records the usage.
async fn run(shell: &Arc<Shell>, who: &Who, app: &LoadedApp, route: &runtime::Route, request: runtime::Request, path: &str, back: &str) -> Response {
    let app_id = &app.manifest.app.id;
    let mut usage = Usage {
        at: now_ms(),
        app_id: app_id.clone(),
        label: app.label.clone(),
        route: route.pattern(),
        outcome: Outcome::Busy,
        status: 503,
        wall_us: 0,
        queue_us: 0,
        fuel: 0,
        io: 0,
        runs: 0,
        error: None,
        risk: shell.risk.of_app(&app.manifest),
    };

    // As many requests at once as the machine has cores; the rest wait in
    // line, and only one that waits too long is turned away.
    let queued = Instant::now();
    let Ok(Ok(permit)) = tokio::time::timeout(QUEUE_WAIT, app.concurrency.clone().acquire_owned()).await else {
        usage.queue_us = queued.elapsed().as_micros() as u64;
        shell.usage.record(usage);
        return message(shell, who, StatusCode::SERVICE_UNAVAILABLE, "Busy", "This app is busy. Try again in a moment.");
    };
    usage.queue_us = queued.elapsed().as_micros() as u64;
    // Turning an app off also stops requests that arrived while it was on
    // but have not started running. The queue can outlive the admin action.
    if shell.state.is_off(app_id) {
        drop(permit);
        usage.error = Some("app was turned off while the request waited in the queue".into());
        shell.usage.record(usage);
        return turned_off(shell, who, app);
    }
    let access = crate::access::of(shell, who, app);
    let started = Instant::now();
    let user = crate::access::user(shell, who);
    let (result, cost) = runtime::call(app, &route.handler, &request, shell.state.budget(app), access, user).await;
    drop(permit);
    if cost.sent > 0 {
        shell.outbox.wake();
    }
    usage.wall_us = started.elapsed().as_micros() as u64;
    (usage.fuel, usage.io, usage.runs) = (cost.fuel, cost.io, cost.runs);

    let resp = match result {
        Ok(page) => {
            let resp = render(shell, who, app, path, request.htmx, page);
            usage.outcome = Outcome::of_status(resp.status().as_u16());
            resp
        }
        Err(f) if f.kind == Outcome::Denied => {
            let whom = if who.identity().is_some() { "signed in" } else { "not signed in" };
            println!("[{app_id}] {} ({whom})", f.error);
            usage.outcome = Outcome::Denied;
            usage.error = Some(f.error.to_string());
            denied(shell, who, app, back, request.htmx)
        }
        Err(f) => {
            // The wasm backtrace has no names in a release build; the app's
            // own output (a panic message) is printed just before this.
            let cause = f.error.root_cause().to_string();
            eprintln!("[{app_id}] failed ({}): {cause}", f.kind);
            usage.outcome = f.kind;
            usage.error = Some(std::iter::once(cause).chain(f.output).collect::<Vec<_>>().join("\n"));
            // Conflicts come from load, not from a broken version.
            if f.kind.counts_toward_rollback() {
                crate::deploy::note_failure(shell, app_id, &app.label);
            }
            message(shell, who, StatusCode::SERVICE_UNAVAILABLE, "Unavailable", "This app is unavailable right now. The rest of the site works.")
        }
    };
    // A denial is a 403, also when it was answered with a redirect.
    usage.status = if usage.outcome == Outcome::Denied { 403 } else { resp.status().as_u16() };
    if shell.keep_examples && usage.status < 400 {
        crate::review::keep(shell, app_id, &route.handler, &request);
    }
    shell.usage.record(usage);
    resp
}

fn percent_decode(s: &str) -> String {
    form_urlencoded::parse(format!("x={}", s.replace('+', "%2B").replace('&', "%26")).as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Turns what the app built into HTTP: a redirect, the page in the layout,
/// or htmx swaps.
fn render(shell: &Shell, who: &Who, app: &LoadedApp, path: &str, htmx: bool, page: runtime::Page) -> Response {
    let app_id = &app.manifest.app.id;
    let status = StatusCode::from_u16(page.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    if let Some(to) = page.redirect {
        return go(&to, htmx);
    }

    if let Some((bad, _)) = page.html.iter().find(|(n, _)| !valid_name(n)) {
        eprintln!("[{app_id}] set-html name {bad:?} is not valid");
        return message(shell, who, StatusCode::INTERNAL_SERVER_ERROR, "App error", "The app set HTML with an invalid name.");
    }

    let body = if htmx {
        let mut body = String::new();
        for (name, html) in &page.html {
            if name == "page" {
                body.push_str(html);
            } else {
                body.push_str(&format!(r#"<div id="{name}" hx-swap-oob="innerHTML">{html}</div>"#));
            }
        }
        body
    } else {
        if let Some((other, _)) = page.html.iter().find(|(n, _)| n != "page") {
            eprintln!("[{app_id}] set-html {other:?} on a full page; this shell only places \"page\"");
            return message(
                shell,
                who,
                StatusCode::INTERNAL_SERVER_ERROR,
                "App error",
                &format!("The app set HTML named {other:?}. Full pages only take \"page\"; other names are for htmx requests."),
            );
        }
        match page.html.iter().find(|(n, _)| n == "page") {
            Some((_, html)) => layout(shell, who, path, &app.manifest.app.name, &page.head, html),
            None => String::new(),
        }
    };

    let mut out = (status, Body::from(body)).into_response();
    out.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    async fn mounted_app() -> (tempfile::TempDir, Router) {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        crate::deploy::deploy(&shell, &crate::testing::bundle("1", None).pack().unwrap(), Actor::Cli).await.unwrap();
        (dir, router(shell))
    }

    #[tokio::test]
    async fn icon_documents_are_sandboxed_on_both_listeners() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let svg = "<svg xmlns='http://www.w3.org/2000/svg'><style>path { fill: red }</style><path d='M0 0h10v10z'/></svg>";
        crate::deploy::deploy(&shell, &crate::testing::bundle("1", Some(svg)).pack().unwrap(), Actor::Cli).await.unwrap();
        for (app, path) in [(router(shell.clone()), "/assets/icons/test"), (platform_router(shell, None), "/admin/icons/test")] {
            let response = app.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "image/svg+xml");
            let csp = response.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
            assert!(csp.split(';').any(|d| d.trim() == "sandbox"), "{path}: {csp}");
            assert!(csp.split(';').any(|d| d.trim() == "script-src 'none'"), "{path}: {csp}");
            assert_eq!(to_bytes(response.into_body(), 64 * 1024).await.unwrap().as_ref(), svg.as_bytes());
        }
    }

    #[tokio::test]
    async fn the_platform_tells_what_each_live_app_is_built_with() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let deploy = |version: &str, packages: Option<&[&str]>| {
            let deps = packages.map(|p| crate::deps::to_json(p.iter().map(|p| p.to_string())));
            let bundle = crate::bundle::Bundle { deps, ..crate::testing::bundle(version, None) };
            let shell = shell.clone();
            async move { crate::deploy::deploy(&shell, &bundle.pack().unwrap(), Actor::Cli).await.unwrap() }
        };
        deploy("1", Some(&["pkg:cargo/time@0.1.43"])).await;
        deploy("2", Some(&["pkg:cargo/time@0.3.47", "pkg:cargo/serde@1.0.228"])).await;
        let other =
            crate::bundle::Bundle { manifest: crate::testing::bundle("1", None).manifest.replace("'test'", "'other'"), ..crate::testing::bundle("1", None) };
        crate::deploy::deploy(&shell, &other.pack().unwrap(), Actor::Cli).await.unwrap();
        let response = platform_router(shell, None).oneshot(Request::builder().uri("/_platform/packages").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let live: Vec<crate::api::LivePackages> = serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap();
        let seen: Vec<_> = live.iter().map(|a| (a.id.as_str(), a.version.as_str(), a.packages.clone())).collect();
        assert_eq!(seen, [("other", "1", None), ("test", "2", Some(vec!["pkg:cargo/serde@1.0.228".into(), "pkg:cargo/time@0.3.47".into()]))]);
    }

    #[tokio::test]
    async fn the_install_script_keeps_https_from_the_reverse_proxy() {
        let dir = tempfile::tempdir().unwrap();
        // Behind the proxy, --origin is the name people use.
        let parts = crate::testing::parts(dir.path().to_path_buf(), Default::default());
        let shell = Shell::new(crate::shell::Parts { origin: "https://platform.example.test".into(), ..parts }).unwrap();
        let response = platform_router(shell, None)
            .oneshot(
                Request::builder()
                    .uri("/_platform/install")
                    .header(header::HOST, "platform.example.test")
                    .header("x-forwarded-proto", "https")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(std::str::from_utf8(&body).unwrap().contains("appos login 'https://platform.example.test'"));
    }

    #[tokio::test]
    async fn admin_queries_with_extreme_timestamps_do_not_crash() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        crate::deploy::deploy(&shell, &crate::testing::bundle("1", None).pack().unwrap(), Actor::Cli).await.unwrap();
        let app = platform_router(shell, None);
        for query in [
            "done=-9223372036854775808&msg=old",
            "done=9223372036854775807&msg=future",
            "page=log&at=9223372036854775807",
            "page=log&zoom=day&at=-9223372036854775808",
        ] {
            let response = app.clone().oneshot(Request::builder().uri(format!("/admin/apps/test?{query}")).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{query}");
            let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            let body = std::str::from_utf8(&body).unwrap();
            assert!(!body.contains("✓ old ·") && !body.contains("✓ future ·"), "invalid timestamps created a toast: {query}");
        }
        for at in [chrono::DateTime::<chrono::Utc>::MIN_UTC.timestamp_millis(), chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp_millis()] {
            let response =
                app.clone().oneshot(Request::builder().uri(format!("/admin/apps/test?page=log&at={at}")).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{at}");
        }
    }

    #[tokio::test]
    async fn appos_dev_keeps_each_routes_last_request_that_worked() {
        let queries = |shell: &Shell| -> Vec<serde_json::Value> {
            let kept = crate::tx::examples(&shell.db, "test").unwrap();
            kept.iter().map(|(_, r)| serde_json::from_str::<serde_json::Value>(r).unwrap()["query"].clone()).collect()
        };
        let dir = tempfile::tempdir().unwrap();
        let parts = crate::testing::parts(dir.path().to_path_buf(), Default::default());
        let shell = Shell::new(crate::shell::Parts { keep_examples: true, ..parts }).unwrap();
        crate::deploy::deploy(&shell, &crate::testing::bundle("1", None).pack().unwrap(), Actor::Cli).await.unwrap();
        for q in ["1", "2"] {
            let response = router(shell.clone()).oneshot(Request::builder().uri(format!("/test?q={q}")).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            // Kept off the request's thread.
            let deadline = Instant::now() + std::time::Duration::from_secs(2);
            while queries(&shell) != [serde_json::json!([["q", q]])] {
                assert!(Instant::now() < deadline, "the request with q={q} is not the one kept: {:?}", queries(&shell));
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        crate::deploy::deploy(&shell, &crate::testing::bundle("1", None).pack().unwrap(), Actor::Cli).await.unwrap();
        router(shell.clone()).oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(queries(&shell).is_empty(), "only appos dev keeps examples");
    }

    #[tokio::test]
    async fn a_false_hx_request_header_still_gets_a_complete_document() {
        let (_dir, app) = mounted_app().await;
        for value in ["false", "", "0"] {
            let response = app.clone().oneshot(Request::builder().uri("/test").header("hx-request", value).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            assert!(std::str::from_utf8(&body).unwrap().starts_with("<!doctype html>"), "HX-Request: {value:?} returned a fragment");
        }
        let response = app.oneshot(Request::builder().uri("/test").header("hx-request", "true").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(to_bytes(response.into_body(), 64 * 1024).await.unwrap().as_ref(), b"ok");
    }

    #[tokio::test]
    async fn a_history_cache_miss_restores_the_whole_document() {
        let (_dir, app) = mounted_app().await;
        // Bundled htmx 2.0.4 sends both headers on a history cache miss.
        let response = app
            .oneshot(Request::builder().uri("/test").header("hx-request", "true").header("hx-history-restore-request", "true").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body = std::str::from_utf8(&body).unwrap();
        assert!(body.starts_with("<!doctype html>"));
        assert!(body.contains("appos-topbar"));
        assert!(body.contains("<main>ok</main>"));
    }

    #[tokio::test]
    async fn caches_can_distinguish_documents_fragments_and_history_restores() {
        let (_dir, app) = mounted_app().await;
        for (hx, restore) in [("false", "false"), ("true", "false"), ("true", "true")] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri("/test").header("hx-request", hx).header("hx-history-restore-request", restore).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let vary: Vec<_> =
                response.headers().get_all(header::VARY).iter().flat_map(|v| v.to_str().unwrap().split(',').map(|s| s.trim().to_ascii_lowercase())).collect();
            assert!(vary.iter().any(|v| v == "hx-request"), "the document and fragment share a cache entry");
            assert!(vary.iter().any(|v| v == "hx-history-restore-request"), "history restoration and ordinary swaps share a cache entry");
        }
    }

    #[tokio::test]
    async fn requests_waiting_for_a_permit_do_not_run_after_the_app_is_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let app = crate::deploy::deploy(&shell, &crate::testing::bundle("1", None).pack().unwrap(), Actor::Cli).await.unwrap().app;
        let router = router(shell.clone());
        let healthy = router.clone().oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(healthy.status(), StatusCode::OK);
        let permits = app.concurrency.clone().acquire_many_owned(app.concurrency.available_permits() as u32).await.unwrap();
        let mut waiting = Box::pin(router.oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap()));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), waiting.as_mut()).await.is_err());
        shell.state.turn("test", false, Actor::Admin, None, None).await.unwrap();
        drop(permits);
        let response = waiting.await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(String::from_utf8(to_bytes(response.into_body(), 64 * 1024).await.unwrap().to_vec()).unwrap().contains("turned off"));
    }

    fn shell() -> Arc<Shell> {
        crate::testing::shell(Default::default())
    }

    #[tokio::test]
    async fn platform_rejects_cross_origin_form_posts() {
        let app = platform_router(shell(), None);
        for (origin, fetch_site, expected) in [
            (Some("https://evil.example"), Some("cross-site"), StatusCode::FORBIDDEN),
            (Some("null"), None, StatusCode::FORBIDDEN),
            (Some("http://localhost.evil.example"), None, StatusCode::FORBIDDEN),
            (Some("https://localhost"), None, StatusCode::FORBIDDEN),
            (None, Some("cross-site"), StatusCode::FORBIDDEN),
            (Some("http://localhost"), Some("same-origin"), StatusCode::SEE_OTHER),
            (None, None, StatusCode::SEE_OTHER),
        ] {
            let mut req = Request::builder()
                .method("POST")
                .uri("/admin/scheme")
                .header(header::HOST, "localhost")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
            if let Some(origin) = origin {
                req = req.header(header::ORIGIN, origin);
            }
            if let Some(site) = fetch_site {
                req = req.header("sec-fetch-site", site);
            }
            let response = app.clone().oneshot(req.body(Body::from("scheme=dark")).unwrap()).await.unwrap();
            assert_eq!(response.status(), expected, "origin {origin:?}, fetch site {fetch_site:?}");
        }
        let req = Request::builder()
            .method("POST")
            .uri("/admin/scheme")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "https://localhost")
            .header("x-forwarded-proto", "https")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("scheme=dark"))
            .unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn app_writes_also_reject_foreign_browser_origins() {
        let app = router(shell());
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            for origin in ["https://evil.example", "null", "https://localhost"] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method(method)
                            .uri("/missing")
                            .header(header::HOST, "localhost")
                            .header(header::ORIGIN, origin)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} from {origin}");
            }
        }
        for site in ["cross-site", "same-site"] {
            let response = app
                .clone()
                .oneshot(Request::builder().method("POST").uri("/missing").header("sec-fetch-site", site).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "fetch site {site}");
        }
        // Local navigation and non-browser clients retain normal dispatch.
        for method in ["GET", "POST"] {
            let response = app
                .clone()
                .oneshot(Request::builder().method(method).uri("/missing").header(header::HOST, "localhost").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    /// DNS rebinding: a page under another name reaches neither listener.
    #[tokio::test]
    async fn only_own_names_localhost_and_ip_addresses_are_answered() {
        let parts = crate::testing::parts(Default::default(), Default::default());
        let shell = Shell::new(crate::shell::Parts { origin: "https://apps.acme.com".into(), ..parts }).unwrap();
        let site = router(shell.clone());
        let platform = platform_router(shell, Some("http://ops.acme.internal:9090"));
        let status = |app: &Router, host: &'static str| {
            let app = app.clone();
            async move { app.oneshot(Request::builder().uri("/_platform/spec").header(header::HOST, host).body(Body::empty()).unwrap()).await.unwrap().status() }
        };
        for host in ["apps.acme.com", "APPS.acme.com.", "localhost:8080", "app.localhost", "127.0.0.1:8080", "10.0.0.5", "[::1]:8080"] {
            assert_ne!(status(&site, host).await, StatusCode::MISDIRECTED_REQUEST, "site, {host}");
            assert_eq!(status(&platform, host).await, StatusCode::OK, "platform, {host}");
        }
        assert_eq!(status(&platform, "ops.acme.internal:9090").await, StatusCode::OK);
        for host in ["evil.example", "evil.example:8080", "apps.acme.com.evil.example", "localhost.evil.example", "", "a b"] {
            assert_eq!(status(&site, host).await, StatusCode::MISDIRECTED_REQUEST, "site, {host}");
            assert_eq!(status(&platform, host).await, StatusCode::MISDIRECTED_REQUEST, "platform, {host}");
        }
        assert_eq!(status(&site, "ops.acme.internal").await, StatusCode::MISDIRECTED_REQUEST, "the platform's name is the platform's");
        // No Host: not a browser.
        let resp = platform.oneshot(Request::builder().uri("/_platform/spec").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_oversized_app_request_is_rejected_before_dispatch() {
        let app = router(shell());
        let response = app.oneshot(Request::builder().method("POST").uri("/missing").body(Body::from(vec![b'x'; 256 * 1024 + 1])).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn invalid_request_bodies_do_not_become_empty_forms() {
        let app = router(shell());
        let response = app.clone().oneshot(Request::builder().method("POST").uri("/missing").body(Body::from(vec![0xff])).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let broken = futures_util::stream::once(async { Err::<axum::body::Bytes, _>(std::io::Error::other("read failed")) });
        let response = app.oneshot(Request::builder().method("POST").uri("/missing").body(Body::from_stream(broken)).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_news_cursor_cannot_overflow() {
        let app = platform_router(shell(), None);
        let response = app.oneshot(Request::builder().uri(format!("/_platform/apps/a/news?since={}", i64::MAX)).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn the_framework_is_the_wit_package() {
        assert!(framework().starts_with("appos:framework@"), "{}", framework());
    }

    #[test]
    fn path_segments_preserve_url_delimiters() {
        for (encoded, decoded) in [("a&b", "a&b"), ("a%26b", "a&b"), ("a+b", "a+b"), ("a%2Fb", "a/b"), ("a=b", "a=b"), ("caf%C3%A9", "café")] {
            assert_eq!(percent_decode(encoded), decoded);
        }
    }

    #[tokio::test]
    async fn install_treats_the_host_as_shell_data() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        for host in ["localhost:8080", "$(printf injected)", "`printf injected`", "x\";printf injected;#", "x'quoted"] {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
            let response = install(headers).await;
            let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            let mut shell = Command::new("sh").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            let mut input = shell.stdin.take().unwrap();
            input.write_all(b"appos() { printf '%s\\n' \"$@\"; }\n").unwrap();
            input.write_all(&body).unwrap();
            drop(input);
            let output = shell.wait_with_output().unwrap();
            assert!(output.status.success());
            assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("login\nhttp://{host}\n"), "host {host:?}");
        }
    }

    #[test]
    fn only_the_most_specific_menu_entry_is_active() {
        let links: Vec<(String, String)> =
            [("Survey", "/survey"), ("Results", "/survey/results"), ("Surveys old", "/surveys")].iter().map(|(l, h)| (l.to_string(), h.to_string())).collect();
        let on = |path: &str| active(&links, path).map(String::as_str);
        assert_eq!(on("/survey"), Some("/survey"));
        assert_eq!(on("/survey/thanks"), Some("/survey"));
        assert_eq!(on("/survey/results"), Some("/survey/results"));
        assert_eq!(on("/survey/results/2026"), Some("/survey/results"));
        // A shared prefix is not a parent.
        assert_eq!(on("/surveys"), Some("/surveys"));
        assert_eq!(on("/surveyor"), None);
        assert_eq!(on("/"), None);
    }
}
