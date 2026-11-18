//! The shell's sign-in pages, under `/_auth/` (app ids can't start with
//! `_`):
//!
//!   GET  /_auth/sign-in?to=/path   the provider's page, or a redirect to it
//!   POST /_auth/sign-in            a form the provider showed
//!   GET  /_auth/callback           back from the identity provider
//!   POST /_auth/sign-out
//!   GET  /_auth/denied?app=id      "not allowed", for htmx (web.rs)
//!
//! The shell keeps a sign-in under way in a signed cookie (state, nonce,
//! PKCE verifier, where to go after) and checks the state on the callback
//! before the provider sees it, so no provider can get that wrong.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use serde::{Deserialize, Serialize};

use super::session::Cookies;
use super::session::{SESSION, SIGN_IN, SIGN_IN_SECS, random};
use super::{Auth, Identity, Provider, Site, Who, dev, sign_in_kit};
use crate::shell::Shell;
use crate::web::{self, pairs};

/// The sign-in pages of the listener at `origin`.
pub fn router(origin: &str) -> Router<Arc<Shell>> {
    Router::new()
        .route("/_auth/sign-in", get(start).post(form))
        .route("/_auth/callback", get(callback))
        .route("/_auth/sign-out", post(sign_out))
        .route("/_auth/denied", get(denied))
        .layer(Extension(Site(origin.trim_end_matches('/').into())))
}

/// A sign-in under way, from its start to the callback or form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub state: String,
    pub nonce: String,
    /// PKCE.
    pub verifier: String,
    /// A local path to go to once signed in.
    pub to: String,
}

/// What a provider does next.
pub enum Step {
    /// Show this HTML (a form) in the shell's layout.
    Page(String),
    /// Send the browser there: the identity provider, or the proxy.
    Redirect(String),
    /// Signed in.
    Done(Identity),
    /// Not signed in, and why, for the person.
    Refuse(String),
    /// The identity provider can't be reached; for the person.
    Unavailable(String),
}

/// Where to go after signing in: a path on this site, else the home page.
pub fn local_or_home(to: Option<&str>) -> String {
    to.filter(|t| crate::runtime::is_local(t)).unwrap_or("/").to_string()
}

fn no_sign_in(shell: &Shell, who: &Who) -> Response {
    web::message(shell, who, StatusCode::NOT_FOUND, "Not found", "This site has no sign-in.")
}

async fn start(State(shell): State<Arc<Shell>>, Extension(site): Extension<Site>, who: Who, RawQuery(query): RawQuery) -> Response {
    let Some(auth) = &shell.auth else { return no_sign_in(&shell, &who) };
    let cookies = auth.cookies.on(&site.0);
    let query = pairs(query.as_deref());
    let to = local_or_home(query.iter().find(|(k, _)| k == "to").map(|(_, v)| v.as_str()));
    let pending = Pending { state: random(24), nonce: random(24), verifier: random(48), to };
    let step = match &auth.provider {
        Provider::Dev => dev::start(None),
        Provider::Header(h) => h.start(&pending.to),
        Provider::Oidc(o) => o.start(&pending, &site.0).await,
        Provider::Kit(p) => p.start(sign_in_kit::request(&pending, &site.0, "GET", "/_auth/sign-in", query, vec![])).await,
        Provider::Token(t) => t.start(&query),
    };
    let mut resp = respond(&shell, auth, &site, &who, &pending, step);
    resp.headers_mut().append(header::SET_COOKIE, cookies.set(SIGN_IN, &pending, SIGN_IN_SECS));
    resp
}

async fn form(State(shell): State<Arc<Shell>>, Extension(site): Extension<Site>, who: Who, headers: HeaderMap, body: Bytes) -> Response {
    let Some(auth) = &shell.auth else { return no_sign_in(&shell, &who) };
    let Some(pending) = auth.cookies.read::<Pending>(&headers, SIGN_IN) else { return start_over() };
    let form = pairs(std::str::from_utf8(&body).ok());
    let step = match &auth.provider {
        Provider::Dev => dev::form(&form),
        Provider::Kit(p) => p.finish(sign_in_kit::request(&pending, &site.0, "POST", "/_auth/sign-in", vec![], form)).await,
        Provider::Token(t) => t.form(&form),
        Provider::Header(_) | Provider::Oidc(_) => Step::Refuse("This sign-in has no form.".into()),
    };
    respond(&shell, auth, &site, &who, &pending, step)
}

async fn callback(State(shell): State<Arc<Shell>>, Extension(site): Extension<Site>, who: Who, headers: HeaderMap, RawQuery(query): RawQuery) -> Response {
    let Some(auth) = &shell.auth else { return no_sign_in(&shell, &who) };
    let Some(pending) = auth.cookies.read::<Pending>(&headers, SIGN_IN) else { return start_over() };
    let query = pairs(query.as_deref());
    if query.iter().find(|(k, _)| k == "state").map(|(_, v)| v.as_str()) != Some(pending.state.as_str()) {
        return respond(&shell, auth, &site, &who, &pending, Step::Refuse("This sign-in link is not the one this browser started. Sign in again.".into()));
    }
    let step = match &auth.provider {
        Provider::Oidc(o) => o.callback(&query, &pending, &site.0).await,
        Provider::Kit(p) => p.finish(sign_in_kit::request(&pending, &site.0, "GET", "/_auth/callback", query, vec![])).await,
        Provider::Dev | Provider::Header(_) | Provider::Token(_) => Step::Refuse("This sign-in has no callback.".into()),
    };
    respond(&shell, auth, &site, &who, &pending, step)
}

async fn sign_out(State(shell): State<Arc<Shell>>, Extension(site): Extension<Site>) -> Response {
    let Some(auth) = &shell.auth else { return StatusCode::NOT_FOUND.into_response() };
    let mut resp = see_other(&auth.sign_out(&site.0).await);
    resp.headers_mut().append(header::SET_COOKIE, auth.cookies.on(&site.0).clear(SESSION));
    resp
}

/// The 403 page for htmx requests the policy denied (web.rs `denied`).
/// It works without sign-in too: a policy can deny anyone.
async fn denied(State(shell): State<Arc<Shell>>, who: Who, RawQuery(query): RawQuery) -> Response {
    let query = pairs(query.as_deref());
    let app = query.iter().find(|(k, _)| k == "app").and_then(|(_, id)| shell.state.app(id));
    web::denied_page(&shell, &who, app.as_deref())
}

/// The sign-in cookie expired or never came: begin again.
fn start_over() -> Response {
    see_other("/_auth/sign-in")
}

pub fn see_other(to: &str) -> Response {
    let mut resp = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = HeaderValue::try_from(to) {
        resp.headers_mut().insert(header::LOCATION, v);
    }
    resp
}

fn respond(shell: &Shell, auth: &Auth, site: &Site, who: &Who, pending: &Pending, step: Step) -> Response {
    let cookies = auth.cookies.on(&site.0);
    match step {
        Step::Page(html) => web::page(shell, who, StatusCode::OK, "Sign in", &format!(r#"<div class="appos-page">{html}</div>"#)),
        Step::Redirect(to) => see_other(&to),
        Step::Done(id) => {
            let mut resp = see_other(&pending.to);
            let h = resp.headers_mut();
            h.append(header::SET_COOKIE, auth.sign_in(&id, &site.0));
            h.append(header::SET_COOKIE, cookies.clear(SIGN_IN));
            resp
        }
        Step::Refuse(why) => ended(web::message(shell, who, StatusCode::FORBIDDEN, "Not signed in", &why), &cookies),
        Step::Unavailable(why) => ended(web::message(shell, who, StatusCode::BAD_GATEWAY, "Sign-in unavailable", &why), &cookies),
    }
}

/// A sign-in that ended without one: its cookie goes.
fn ended(mut resp: Response, cookies: &Cookies) -> Response {
    resp.headers_mut().append(header::SET_COOKIE, cookies.clear(SIGN_IN));
    resp
}

#[cfg(test)]
pub(crate) mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;
    use crate::testing::shell_with_auth;

    pub async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, String) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (resp.status(), resp.headers().clone());
        (status, headers, String::from_utf8(to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap())
    }

    pub fn get(uri: &str, cookie: &str) -> Request<Body> {
        Request::get(uri).header(header::HOST, "localhost").header(header::COOKIE, cookie).body(Body::empty()).unwrap()
    }

    pub fn post(uri: &str, cookie: &str, form: &str) -> Request<Body> {
        Request::post(uri)
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_string()))
            .unwrap()
    }

    /// The `name=value` of each Set-Cookie header.
    pub fn cookies(headers: &HeaderMap) -> String {
        headers.get_all(header::SET_COOKIE).iter().map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string()).collect::<Vec<_>>().join("; ")
    }

    /// Signs in with the dev provider; returns the session cookie.
    pub async fn sign_in(app: &Router, form: &str) -> String {
        let (_, headers, _) = send(app, get("/_auth/sign-in?to=/elsewhere", "")).await;
        let (status, headers, _) = send(app, post("/_auth/sign-in", &cookies(&headers), form)).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        cookies(&headers).split("; ").find(|c| c.starts_with("appos-session=")).unwrap().to_string()
    }

    #[tokio::test]
    async fn dev_sign_in_shows_the_name_and_sign_out_ends_it() {
        let app = crate::web::router(shell_with_auth("provider = \"dev\""));
        let (_, headers, body) = send(&app, get("/", "")).await;
        assert!(body.contains(r#"href="/_auth/sign-in?to=%2F""#), "{body}");
        assert!(headers.get(header::CACHE_CONTROL).is_none());

        let (_, headers, _) = send(&app, get("/_auth/sign-in?to=/webinars/team", "")).await;
        let (status, headers, _) = send(&app, post("/_auth/sign-in", &cookies(&headers), "email=Ann%40Acme.com&name=Ann+Lee&groups=sales")).await;
        assert_eq!((status, headers.get(header::LOCATION).unwrap().to_str().unwrap()), (StatusCode::SEE_OTHER, "/webinars/team"));
        let session = cookies(&headers).split("; ").find(|c| c.starts_with("appos-session=")).unwrap().to_string();
        assert!(cookies(&headers).split("; ").any(|c| c == "appos-signin="), "the sign-in cookie is cleared");

        let (_, headers, body) = send(&app, get("/", &session)).await;
        assert!(body.contains("Ann Lee") && body.contains("ann@acme.com") && body.contains(r#"action="/_auth/sign-out""#));
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "private, no-store");

        let (status, headers, _) = send(&app, post("/_auth/sign-out", &session, "")).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert!(cookies(&headers).split("; ").any(|c| c == "appos-session="));
    }

    #[tokio::test]
    async fn sign_in_goes_back_only_to_this_site() {
        let app = crate::web::router(shell_with_auth("provider = \"dev\""));
        for to in ["https://evil.example", "//evil.example", "/\\evil.example"] {
            let (_, headers, _) =
                send(&app, get(&format!("/_auth/sign-in?to={}", form_urlencoded::byte_serialize(to.as_bytes()).collect::<String>()), "")).await;
            let (_, headers, _) = send(&app, post("/_auth/sign-in", &cookies(&headers), "email=a%40b.c")).await;
            assert_eq!(headers.get(header::LOCATION).unwrap(), "/", "{to}");
        }
    }

    #[tokio::test]
    async fn forged_or_foreign_sign_ins_do_nothing() {
        let app = crate::web::router(shell_with_auth("provider = \"dev\""));
        // No sign-in under way: start over.
        let (status, headers, _) = send(&app, post("/_auth/sign-in", "", "email=a%40b.c")).await;
        assert_eq!((status, headers.get(header::LOCATION).unwrap().to_str().unwrap()), (StatusCode::SEE_OTHER, "/_auth/sign-in"));
        // A session from another shell's key is nobody.
        let other = super::super::Auth::parse("provider = \"dev\"", "http://localhost", [8; 32], "127.0.0.1:1").unwrap();
        let forged = other.cookies.seal(SESSION, &Identity::new("", "eve@evil.example", "Eve", []).unwrap(), 60);
        let (_, _, body) = send(&app, get("/", &format!("appos-session={forged}"))).await;
        assert!(!body.contains("Eve") && body.contains("Sign in"));
        // Signing out from another site is refused.
        let req =
            Request::post("/_auth/sign-out").header(header::HOST, "localhost").header(header::ORIGIN, "https://evil.example").body(Body::empty()).unwrap();
        assert_eq!(send(&app, req).await.0, StatusCode::FORBIDDEN);
        // A callback with another state is refused.
        let (_, headers, _) = send(&app, get("/_auth/sign-in", "")).await;
        let (status, _, body) = send(&app, get("/_auth/callback?state=forged&code=x", &cookies(&headers))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }

    fn written(shell: &Shell) -> Option<String> {
        crate::tx::Tx::default().get(&shell.db, "test", "k").unwrap()
    }

    fn htmx(mut req: Request<Body>, current: &str) -> Request<Body> {
        req.headers_mut().insert("hx-request", HeaderValue::from_static("true"));
        req.headers_mut().insert("hx-current-url", HeaderValue::try_from(current).unwrap());
        req
    }

    #[tokio::test]
    async fn a_denied_write_sends_to_sign_in_and_writes_nothing() {
        let policy = "p, anyone, test, kv-read\np, signed-in, test, kv-write\n";
        let (_dir, shell) = crate::testing::shell_with_policy("provider = \"dev\"", policy).await;
        let app = crate::web::router(shell.clone());

        let (status, _, body) = send(&app, get("/test", "")).await;
        assert_eq!((status, body.contains("ok")), (StatusCode::OK, true), "anyone may read: {body}");

        let mut req = post("/test", "", "");
        req.headers_mut().insert(header::REFERER, HeaderValue::from_static("http://localhost/test?week=3"));
        let (status, headers, _) = send(&app, req).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(headers[header::LOCATION], "/_auth/sign-in?to=%2Ftest%3Fweek%3D3");
        assert_eq!(written(&shell), None, "the denied request's writes are dropped");

        let (status, headers, _) = send(&app, htmx(post("/test", "", ""), "http://localhost/test/")).await;
        assert_eq!((status, &headers["hx-redirect"]), (StatusCode::OK, &HeaderValue::from_static("/_auth/sign-in?to=%2Ftest%2F")));

        // A page from elsewhere is not where sign-in comes back to.
        let mut req = post("/test", "", "");
        req.headers_mut().insert(header::REFERER, HeaderValue::from_static("https://evil.example/test"));
        assert_eq!(send(&app, req).await.1[header::LOCATION], "/_auth/sign-in?to=%2Ftest");

        let session = sign_in(&app, "email=ann%40acme.com").await;
        let (status, _, _) = send(&app, post("/test", &session, "")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(written(&shell).as_deref(), Some("v"));
    }

    #[tokio::test]
    async fn signed_in_or_without_sign_in_a_denial_is_a_403_naming_the_owner() {
        let (_dir, shell) = crate::testing::shell_with_policy("provider = \"dev\"", "p, group:sales, test, kv-*\n").await;
        let app = crate::web::router(shell.clone());
        let session = sign_in(&app, "email=ann%40acme.com&groups=marketing").await;
        let (status, _, body) = send(&app, get("/test", &session)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("Not allowed") && body.contains("ask its owner, Tess Tester"), "{body}");
        let session = sign_in(&app, "email=sam%40acme.com&groups=sales").await;
        assert_eq!(send(&app, get("/test", &session)).await.0, StatusCode::OK);

        // A policy without sign-in: everyone is anyone.
        let (_dir, shell) = crate::testing::shell_with_policy("", "p, anyone, test, kv-read\n").await;
        let app = crate::web::router(shell.clone());
        assert_eq!(send(&app, get("/test", "")).await.0, StatusCode::OK);
        let (status, _, body) = send(&app, post("/test", "", "")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("Tess Tester") && !body.contains("Sign in"), "{body}");
        assert_eq!(written(&shell), None);
        // htmx drops a 403's body: it goes to the page instead.
        let (status, headers, _) = send(&app, htmx(post("/test", "", ""), "http://localhost/test")).await;
        assert_eq!((status, &headers["hx-redirect"]), (StatusCode::OK, &HeaderValue::from_static("/_auth/denied?app=test")));
        let (status, _, body) = send(&app, get("/_auth/denied?app=test", "")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("do this in Test. If you should, ask its owner, Tess Tester"), "{body}");
    }

    #[tokio::test]
    async fn a_policy_change_takes_effect_and_is_logged() {
        let (dir, shell) = crate::testing::shell_with_policy("", "p, anyone, test, kv-read\n").await;
        let app = crate::web::router(shell.clone());
        assert_eq!(send(&app, post("/test", "", "")).await.0, StatusCode::FORBIDDEN);
        // A different length, so the change shows even within one mtime tick.
        std::fs::write(dir.path().join("policy.csv"), "p, anyone, test, kv-read\np, anyone, test, kv-write\n").unwrap();
        crate::access::check(&shell).await;
        assert_eq!(send(&app, post("/test", "", "")).await.0, StatusCode::OK);
        std::fs::write(dir.path().join("policy.csv"), "p, anyone, test, kv-fly\n").unwrap();
        crate::access::check(&shell).await;
        assert_eq!(send(&app, post("/test", "", "")).await.0, StatusCode::OK, "a bad file leaves the policy as it was");
        let events = shell.platform.events(Some(""), 10).unwrap();
        let kinds: Vec<_> = events.iter().map(|e| (e.kind, e.reason.clone().unwrap_or_default())).collect();
        assert!(kinds.iter().any(|(k, _)| *k == crate::platform::Kind::PolicyChanged), "{kinds:?}");
        assert!(kinds.iter().any(|(k, r)| *k == crate::platform::Kind::PolicyRefused && r.contains("kv-fly")), "{kinds:?}");
    }

    /// The query of a URL, as pairs.
    fn query_of(url: &str) -> Vec<(String, String)> {
        pairs(url.split_once('?').map(|(_, q)| q))
    }

    fn field<'a>(pairs: &'a [(String, String)], name: &str) -> &'a str {
        pairs.iter().find(|(k, _)| k == name).map_or("", |(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn oidc_signs_in_with_pkce_and_checks_the_id_token() {
        use super::super::oidc::tests::{auth_toml, idp};
        let idp = idp().await;
        let app = crate::web::router(shell_with_auth(&auth_toml(&idp)));

        let (status, headers, _) = send(&app, get("/_auth/sign-in?to=/webinars/team", "")).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let location = headers[header::LOCATION].to_str().unwrap();
        assert!(location.starts_with(&format!("{}/authorize?", idp.url)), "{location}");
        let asked = query_of(location);
        assert_eq!(field(&asked, "client_id"), "appos");
        assert_eq!(field(&asked, "redirect_uri"), "http://localhost/_auth/callback");
        assert_eq!(field(&asked, "scope"), "openid email profile");
        assert_eq!(field(&asked, "code_challenge_method"), "S256");
        *idp.challenge.lock() = field(&asked, "code_challenge").to_string();
        let signing_in = cookies(&headers);
        let claims = |nonce: &str| {
            serde_json::json!({"iss": idp.url, "sub": "u1", "aud": "appos", "exp": crate::auth::session::now() + 300, "nonce": nonce,
                "email": "ann@acme.com", "name": "Ann Lee", "groups": ["sales"]})
        };

        // An id token from another sign-in is refused.
        *idp.claims.lock() = claims("not-this-one");
        let callback = format!("/_auth/callback?code=the-code&state={}", field(&asked, "state"));
        let (status, headers, body) = send(&app, get(&callback, &signing_in)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.contains("belongs to another sign-in") && cookies(&headers).split("; ").any(|c| c == "appos-signin="), "{body}");

        // The right one signs in and goes back.
        *idp.claims.lock() = claims(field(&asked, "nonce"));
        let (status, headers, body) = send(&app, get(&callback, &signing_in)).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{body}");
        assert_eq!(headers[header::LOCATION], "/webinars/team");
        let session = cookies(&headers).split("; ").find(|c| c.starts_with("appos-session=")).unwrap().to_string();
        let (_, _, body) = send(&app, get("/", &session)).await;
        assert!(body.contains("Ann Lee"), "{body}");

        // A wrong code (or PKCE verifier) is turned down by the provider.
        let (status, _, body) = send(&app, get(&callback.replace("the-code", "other"), &signing_in)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("turned the sign-in down"), "{body}");
        // The provider's own refusal is shown.
        let refused = format!("/_auth/callback?error=access_denied&error_description=Not+for+you&state={}", field(&asked, "state"));
        assert!(send(&app, get(&refused, &signing_in)).await.2.contains("The sign-in service said: Not for you"));

        // Signing out ends the provider's session too.
        let (_, headers, _) = send(&app, post("/_auth/sign-out", &session, "")).await;
        let to = headers[header::LOCATION].to_str().unwrap();
        assert!(to.starts_with(&format!("{}/logout?client_id=appos&post_logout_redirect_uri=http%3A%2F%2Flocalhost%2F", idp.url)), "{to}");
    }

    #[tokio::test]
    async fn an_unreachable_identity_provider_stops_sign_in_not_the_site() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        drop(listener);
        let toml = format!("provider = \"oidc\"\nissuer = \"{url}\"\nclient-id = \"appos\"");
        let app = crate::web::router(shell_with_auth(&toml));
        let (status, _, body) = send(&app, get("/_auth/sign-in", "")).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body.contains("be reached right now"), "{body}");
        assert_eq!(send(&app, get("/", "")).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_header_provider_believes_only_its_proxy() {
        use axum::extract::ConnectInfo;
        let app = crate::web::router(shell_with_auth("provider = \"header\"\ntrusted-proxies = [\"10.0.0.2\"]\nsign-in-url = \"/oauth2/start?rd={to}\""));
        let from = |ip: &str| {
            let mut req = get("/", "");
            req.headers_mut().insert("x-forwarded-email", HeaderValue::from_static("ann@acme.com"));
            req.extensions_mut().insert(ConnectInfo(std::net::SocketAddr::new(ip.parse().unwrap(), 4000)));
            req
        };
        let (_, headers, body) = send(&app, from("10.0.0.2")).await;
        assert!(body.contains("ann@acme.com") && body.contains(r#"action="/_auth/sign-out""#), "{body}");
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");
        let (_, _, body) = send(&app, from("10.0.0.9")).await;
        assert!(!body.contains("ann@acme.com") && body.contains("Sign in"), "{body}");
        let (status, headers, _) = send(&app, get("/_auth/sign-in?to=/x", "")).await;
        assert_eq!((status, headers[header::LOCATION].to_str().unwrap()), (StatusCode::SEE_OTHER, "/oauth2/start?rd=%2Fx"));
    }

    #[tokio::test]
    async fn without_auth_there_is_no_sign_in() {
        let app = crate::web::router(crate::testing::shell(Default::default()));
        let (_, headers, body) = send(&app, get("/", "")).await;
        assert!(!body.contains("Sign in") && headers.get(header::VARY).is_none());
        assert_eq!(send(&app, get("/_auth/sign-in", "")).await.0, StatusCode::NOT_FOUND);
    }

    // ---- the admin screen ---------------------------------------------------

    /// Platform admins, and anyone may read the test app.
    const ADMINS: &str = "p, group:platform, *, admin-*\np, anyone, test, kv-read\n";

    /// The site and the admin on one listener, as without --platform-addr.
    fn both(shell: &Arc<Shell>) -> Router {
        crate::web::router(shell.clone()).merge(crate::web::platform_router(shell.clone(), None))
    }

    fn location(headers: &HeaderMap) -> &str {
        headers.get(header::LOCATION).map_or("", |v| v.to_str().unwrap())
    }

    /// A platform API call with a token, as the CLI makes it.
    fn api(method: &str, uri: &str, token: &str) -> Request<Body> {
        let body = if method == "POST" { "{}" } else { "" };
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    /// Makes a token on the admin screen as the admin signed in with `cookie`.
    async fn make_token(app: &Router, cookie: &str) -> String {
        let (status, _, body) = send(app, post("/admin/token", cookie, "")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let at = body.find("appos_").unwrap();
        body[at..].split('<').next().unwrap().to_string()
    }

    #[tokio::test]
    async fn the_platform_api_takes_a_token_of_an_admin() {
        let (dir, shell) = crate::testing::shell_with_policy("provider = \"dev\"", ADMINS).await;
        let app = both(&shell);
        // Without one: what to do. Spec, for appos login, is open and says so.
        let (status, headers, body) = send(&app, get("/_platform/log", "")).await;
        assert_eq!((status, &headers[header::WWW_AUTHENTICATE]), (StatusCode::UNAUTHORIZED, &"Bearer".parse::<axum::http::HeaderValue>().unwrap()));
        assert!(body.contains("appos login http://localhost"), "{body}");
        let (status, _, body) = send(&app, get("/_platform/spec", "")).await;
        assert!(status == StatusCode::OK && body.contains(r#""token":true"#), "{body}");
        assert_eq!(send(&app, api("GET", "/_platform/log", "appos_guess")).await.0, StatusCode::UNAUTHORIZED);

        // An admin makes one on the admin screen; nobody else gets there.
        let ann = sign_in(&app, "email=ann%40acme.com&groups=platform").await;
        let (_, _, body) = send(&app, get("/admin/token", &ann)).await;
        assert!(body.contains("Make a token") && body.contains("ann@acme.com"), "{body}");
        let bob = sign_in(&app, "email=bob%40acme.com").await;
        assert_eq!(send(&app, post("/admin/token", &bob, "")).await.0, StatusCode::FORBIDDEN);
        let token = make_token(&app, &ann).await;

        // It is ann: the CLI's changes are logged with her email.
        let (status, _, body) = send(&app, api("GET", "/_platform/me", &token)).await;
        assert!(status == StatusCode::OK && body.contains(r#""email":"ann@acme.com""#), "{body}");
        assert_eq!(send(&app, api("POST", "/_platform/apps/test/off", &token)).await.0, StatusCode::OK);
        let e = &shell.platform.events(Some("test"), 1).unwrap()[0];
        assert_eq!((e.kind, e.actor, e.person.as_deref()), (crate::platform::Kind::TurnedOff, crate::platform::Actor::Cli, Some("ann@acme.com")));

        // Out of the policy, her token stops at once.
        std::fs::write(dir.path().join("policy.csv"), "p, group:ops, *, admin-*\n").unwrap();
        crate::access::check(&shell).await;
        let (status, _, body) = send(&app, api("GET", "/_platform/log", &token)).await;
        assert!(status == StatusCode::FORBIDDEN && body.contains("ann@acme.com may not use the platform API"), "{body}");

        // appos logout ends it.
        let logout = || Request::post("/_platform/logout").header(header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap();
        assert_eq!(send(&app, logout()).await.0, StatusCode::NO_CONTENT);
        assert_eq!(send(&app, logout()).await.0, StatusCode::NOT_FOUND);
        assert_eq!(send(&app, api("GET", "/_platform/me", &token)).await.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn with_no_auth_the_platform_api_takes_no_token() {
        let shell = crate::testing::shell(Default::default());
        let app = both(&shell);
        assert_eq!(send(&app, get("/_platform/log", "")).await.0, StatusCode::OK);
        let (_, _, body) = send(&app, get("/_platform/spec", "")).await;
        assert!(body.contains(r#""token":false"#), "{body}");
        let (_, _, body) = send(&app, get("/admin/token", "")).await;
        assert!(body.contains("takes no token"), "{body}");
    }

    #[tokio::test]
    async fn the_admin_screen_is_for_admins_only() {
        let (_dir, shell) = crate::testing::shell_with_policy("provider = \"dev\"", ADMINS).await;
        let app = both(&shell);
        // Not signed in: to sign in, and back to the page after.
        for (path, to) in [("/admin", "%2Fadmin"), ("/admin/log?q=x", "%2Fadmin%2Flog%3Fq%3Dx"), ("/admin/stream", "%2Fadmin%2Fstream")] {
            let (status, headers, _) = send(&app, get(path, "")).await;
            assert_eq!((status, location(&headers)), (StatusCode::SEE_OTHER, format!("/_auth/sign-in?to={to}").as_str()), "{path}");
        }
        // A change goes nowhere but to sign in.
        let (status, headers, _) = send(&app, post("/admin/apps/test/off", "", "")).await;
        assert_eq!((status, location(&headers)), (StatusCode::SEE_OTHER, "/_auth/sign-in?to=%2Fadmin"));
        assert!(!shell.state.is_off("test"));
        // Signed in, not an admin.
        let bob = sign_in(&app, "email=bob%40acme.com&groups=sales").await;
        let (status, _, body) = send(&app, get("/admin", &bob)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.contains("This needs admin-read, and policy.csv doesn&#39;t give it to you."), "{body}");
        assert_eq!(send(&app, get("/admin/stream", &bob)).await.0, StatusCode::FORBIDDEN);
        assert_eq!(send(&app, post("/admin/apps/test/off", &bob, "")).await.0, StatusCode::FORBIDDEN);
        assert!(!shell.state.is_off("test"));
        // An admin, with the name and Sign out in the header.
        let ann = sign_in(&app, "email=ann%40acme.com&name=Ann+Lee&groups=platform").await;
        let (status, _, body) = send(&app, get("/admin", &ann)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains(r#"<span title="ann@acme.com">Ann Lee</span>"#) && body.contains(r#"action="/_auth/sign-out""#), "{body}");
        // The platform API takes a token, not the admin screen's cookie.
        assert_eq!(send(&app, get("/_platform/log", "")).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(send(&app, get("/_platform/log", &ann)).await.0, StatusCode::UNAUTHORIZED);
    }

    /// admin-read without admin-write: the screen, its controls inert, and
    /// a change refused; the API reads and doesn't change.
    #[tokio::test]
    async fn admin_read_looks_and_changes_nothing() {
        let csv = "p, group:support, *, admin-read\np, anyone, test, kv-read\n";
        let (_dir, shell) = crate::testing::shell_with_policy("provider = \"dev\"", csv).await;
        let app = both(&shell);
        let sue = sign_in(&app, "email=sue%40acme.com&groups=support").await;
        for path in ["/admin", "/admin/apps/test", "/admin/log", "/admin/messages", "/admin/policy", "/admin/token"] {
            let (status, _, body) = send(&app, get(path, &sue)).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
            assert!(body.contains(r#"<span class="looks""#), "{path}");
        }
        let (_, _, body) = send(&app, get("/admin/apps/test", &sue)).await;
        assert!(body.contains(r#"<form inert class="c-apwr" method="post""#), "{body}");
        assert!(body.contains(r#"<form class="scheme" method="post" action="/admin/scheme""#), "the switch is hers: {body}");
        assert!(!body.contains(r#"<form class="c-apwr""#), "{body}");
        let (status, _, body) = send(&app, post("/admin/apps/test/off", &sue, "")).await;
        assert!(status == StatusCode::FORBIDDEN && body.contains("This needs admin-write"), "{body}");
        assert!(!shell.state.is_off("test"));
        // The scheme and a token are hers.
        assert_eq!(send(&app, post("/admin/scheme", &sue, "scheme=dark")).await.0, StatusCode::SEE_OTHER);
        let token = make_token(&app, &sue).await;
        assert_eq!(send(&app, api("GET", "/_platform/log", &token)).await.0, StatusCode::OK);
        let (status, _, body) = send(&app, api("POST", "/_platform/apps/test/off", &token)).await;
        assert!(status == StatusCode::FORBIDDEN && body.contains("it needs admin-write"), "{body}");
        assert!(!shell.state.is_off("test"));
    }

    #[tokio::test]
    async fn without_sign_in_only_a_policy_naming_anyone_lets_in() {
        let (_dir, shell) = crate::testing::shell_with_policy("", ADMINS).await;
        let (status, _, _) = send(&both(&shell), get("/admin", "")).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "no one to sign in as");
        let (_dir, shell) = crate::testing::shell_with_policy("", "p, anyone, *, admin-*\n").await;
        assert_eq!(send(&both(&shell), get("/admin", "")).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn an_admins_change_is_logged_with_their_email() {
        let (_dir, shell) = crate::testing::shell_with_policy("provider = \"dev\"", ADMINS).await;
        let app = both(&shell);
        let ann = sign_in(&app, "email=Ann%40Acme.com&groups=platform").await;
        assert_eq!(send(&app, post("/admin/apps/test/off", &ann, "")).await.0, StatusCode::SEE_OTHER);
        assert!(shell.state.is_off("test"));
        let e = &shell.platform.events(Some("test"), 1).unwrap()[0];
        assert_eq!((e.kind, e.actor, e.person.as_deref()), (crate::platform::Kind::TurnedOff, crate::platform::Actor::Admin, Some("ann@acme.com")));
        // The log shows it and finds it by the email.
        let (_, _, body) = send(&app, get("/admin/log?q=ann%40acme", &ann)).await;
        assert!(body.contains("Admin screen, ann@acme.com"), "{body}");
        let (_, _, body) = send(&app, get("/admin/log?q=bob%40acme", &ann)).await;
        assert!(!body.contains("Admin screen, ann@acme.com"), "{body}");
        let (_, _, body) = send(&app, get("/admin/apps/test", &ann)).await;
        assert!(body.contains("by ann@acme.com"), "{body}");
        // So does the platform API, for appos log.
        let token = make_token(&app, &ann).await;
        let (_, _, body) = send(&app, api("GET", "/_platform/log?q=ann%40acme", &token)).await;
        assert!(body.contains(r#""person":"ann@acme.com""#), "{body}");
        // The CLI's deploy has no one.
        let events = shell.platform.events(Some("test"), 10).unwrap();
        let deployed = events.iter().find(|e| e.kind == crate::platform::Kind::Deployed).unwrap();
        assert_eq!((deployed.actor, deployed.person.as_deref()), (crate::platform::Actor::Cli, None));
    }

    #[tokio::test]
    async fn the_policy_page_reads_the_rules_and_says_when_a_change_is_refused() {
        let csv = "p, group:platform, *, admin-*\np, anyone, test, kv-*\np, signed-in, apps:Tests, kv-write\np, anyone, tset, kv-read\np, anyone, *, score-read\ng, ann@acme.com, group:platform\n";
        let (dir, shell) = crate::testing::shell_with_policy("provider = \"dev\"", csv).await;
        let app = both(&shell);
        // Ann is an admin by the file's g line alone.
        let ann = sign_in(&app, "email=ann%40acme.com").await;
        let (status, _, body) = send(&app, get("/admin/policy", &ann)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for said in [
            "Anyone",
            "Everyone signed in",
            "Group, with ann@acme.com",
            "In platform",
            "the admin screen and the platform API",
            ">admin-write<",
            ">kv-read<",
            "(kv-*)",
            r#"href="/admin/apps/test">Test</a>"#,
            "the apps of group <b>Tests</b>",
            "<b>tset</b> <span class=\"quiet\">(no app of that id here)</span>",
            "<b>score-read</b> <span class=\"quiet\">(no kit offers it now)</span>",
            "6 lines from",
        ] {
            assert!(body.contains(said), "{said}: {body}");
        }
        assert!(!body.contains("not taken"));
        // A mistake: the rules stay, and the page and the console say so.
        std::fs::write(dir.path().join("policy.csv"), "p, group:platform, *, admin-*\np, nobody, test, kv-read\n").unwrap();
        crate::access::check(&shell).await;
        let (_, _, body) = send(&app, get("/admin/policy", &ann)).await;
        assert!(body.contains("policy.csv not taken") && body.contains("nobody") && body.contains("the admin screen and the platform API"), "{body}");
        let (_, _, body) = send(&app, get("/admin", &ann)).await;
        assert!(body.contains(r#"href="/admin/policy""#) && body.contains("Policy refused"), "{body}");
        // Fixed: the problem goes.
        std::fs::write(dir.path().join("policy.csv"), "p, group:platform, *, admin-*\n").unwrap();
        crate::access::check(&shell).await;
        let (_, _, body) = send(&app, get("/admin", &ann)).await;
        assert!(!body.contains("Policy refused"), "{body}");
    }

    #[tokio::test]
    async fn a_platform_listener_of_its_own_signs_in_on_its_own_origin() {
        let (_dir, shell) = crate::testing::shell_with_policy("provider = \"dev\"", ADMINS).await;
        let platform = crate::web::platform_router(shell.clone(), Some("https://admin.acme.com"));
        let (_, headers, _) = send(&platform, get("/", "")).await;
        assert_eq!(location(&headers), "/admin");
        let (_, headers, _) = send(&platform, get("/admin", "")).await;
        let (_, headers, _) = send(&platform, get(location(&headers), "")).await;
        let (status, headers, _) = send(&platform, post("/_auth/sign-in", &cookies(&headers), "email=ann%40acme.com&groups=platform")).await;
        assert_eq!((status, location(&headers)), (StatusCode::SEE_OTHER, "/admin"));
        let session = headers.get_all(header::SET_COOKIE).iter().map(|v| v.to_str().unwrap()).find(|c| c.starts_with("appos-session=")).unwrap();
        assert!(session.ends_with("; Secure"), "https: {session}");
        let cookie = session.split(';').next().unwrap();
        assert_eq!(send(&platform, get("/admin", cookie)).await.0, StatusCode::OK);
        // Merged into the site's listener, the site's sign-in pages serve.
        assert_eq!(send(&crate::web::platform_router(shell, None), get("/_auth/sign-in", "")).await.0, StatusCode::NOT_FOUND);
    }
}
