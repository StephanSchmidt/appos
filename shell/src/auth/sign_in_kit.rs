//! The kit provider: sign-in by a kit in Wasm, for what the built-in
//! providers don't do (GitHub, an in-house identity service, a gateway to
//! LDAP). Its world is `appos:auth/sign-in` (wit-auth/auth.wit).
//!
//! ```toml
//! # auth.toml
//! provider = "kit"
//! kit = "acme-sign-in"             # the kit's directory, next to auth.toml
//!
//! # acme-sign-in/kit.toml (wasm_kit.rs)
//! [kit]
//! id = "acme-sign-in"
//! name = "Acme sign-in"
//! version = "1.0.0"
//! wasm = "acme-auth.wasm"
//! [sign-in]
//! hosts = ["login.acme.com"]       # the only hosts it may call
//! [config]                         # what config.get returns
//! tenant = "acme"
//! [config-env]                     # secrets, read from the environment
//! client-secret = "ACME_SECRET"
//! ```
//!
//! The kit is a sandbox like an app: it may call `config.get` and
//! `http.fetch`, and nothing else. No files, no environment, no kv; fetch
//! only to `hosts`, over https (http only to localhost), without following
//! redirects. Each step runs in a fresh instance with an app's fuel and
//! memory, for at most 10 seconds. A kit that fails shows "sign-in
//! unavailable"; the site goes on.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use sha2::{Digest, Sha256};
use tokio::time::{Instant, timeout_at};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use super::oidc::secure;
use super::routes::{Pending, Step};
use super::{Identity, need, take_str};
use crate::runtime::{ALLOWED_WASI, FUEL, MEMORY, imports, is_local};
use crate::terminal_errorln as eprintln;

wasmtime::component::bindgen!({
    world: "sign-in",
    path: "../wit-auth",
    imports: {
        "appos:auth/http.fetch": async | trappable,
        default: trappable,
    },
    exports: { default: async },
});

use appos::auth::{config, http, types};
pub use types::Request;

/// One step of a sign-in, all fetches included.
const TIME: Duration = Duration::from_secs(10);
/// Fetches in one step.
const MAX_FETCHES: u32 = 20;
/// Bytes of an answer to a fetch, and of a page the kit shows.
const MAX_BODY: u64 = 1 << 20;
const MAX_PAGE: usize = 256 * 1024;
/// Bytes of what one step prints.
const MAX_OUTPUT: usize = 16 * 1024;
const PACKAGE: &str = "appos:auth/";
/// The interfaces of the sign-in world.
const OWN: &[&str] = &["types", "config", "http"];

pub struct SignInKit {
    /// The kit's id, for messages.
    file: String,
    pre: SignInPre<Ctx>,
    hosts: Arc<Vec<String>>,
    config: Arc<BTreeMap<String, String>>,
}

impl SignInKit {
    /// Reads the kit auth.toml names, from its directory next to auth.toml
    /// (`dir`), and compiles it: a kit that can't run stops the shell from
    /// starting.
    pub fn parse(table: &mut toml::Table, dir: &Path, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<SignInKit> {
        let kit = need(take_str(table, "kit")?, "kit", "kit = \"sign-in\", the kit's directory")?;
        let found = crate::wasm_kit::sign_in(&dir.join(&kit), env).with_context(|| format!("kit = {kit:?}"))?;
        for h in &found.hosts {
            if h.contains(['/', ':', '@', '?', '#']) || h.chars().any(char::is_whitespace) {
                bail!("kit = {kit:?}: [sign-in] hosts: {h:?}: give the host alone, like \"login.acme.com\"");
            }
        }
        let pre = compile(&found.wasm).with_context(|| format!("kit {}", found.id))?;
        Ok(SignInKit { file: found.id, pre, hosts: Arc::new(found.hosts.into_iter().map(|h| h.to_lowercase()).collect()), config: Arc::new(found.config) })
    }

    pub async fn start(&self, req: Request) -> Step {
        self.run(req, true).await
    }

    pub async fn finish(&self, req: Request) -> Step {
        self.run(req, false).await
    }

    async fn run(&self, req: Request, start: bool) -> Step {
        let deadline = Instant::now() + TIME;
        // What it prints, such as a panic's message, shows when it fails.
        let output = MemoryOutputPipe::new(MAX_OUTPUT);
        let ctx = Ctx {
            wasi: WasiCtx::builder().stdout(output.clone()).stderr(output.clone()).build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new().memory_size(MEMORY).table_elements(100_000).instances(100).build(),
            hosts: self.hosts.clone(),
            config: self.config.clone(),
            deadline,
            fetches: 0,
        };
        let mut store = Store::new(self.pre.engine(), ctx);
        store.limiter(|c| &mut c.limits);
        let _ = store.set_fuel(FUEL);
        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);
        let ran = timeout_at(deadline, async {
            let kit = self.pre.instantiate_async(&mut store).await?;
            let provider = kit.appos_auth_provider();
            if start { provider.call_start(&mut store, &req).await } else { provider.call_finish(&mut store, &req).await }
        })
        .await;
        let printed = String::from_utf8_lossy(&output.contents()).trim().to_string();
        let printed = if printed.is_empty() { String::new() } else { format!("\n{printed}") };
        match ran {
            Ok(Ok(step)) => self.step(step),
            Ok(Err(e)) => self.failed(&format!("{e:#}{printed}")),
            Err(_) => self.failed(&format!("took longer than {} s{printed}", TIME.as_secs())),
        }
    }

    /// The kit's step, checked.
    fn step(&self, step: types::Step) -> Step {
        match step {
            types::Step::Page(html) if html.len() > MAX_PAGE => self.failed(&format!("a page of {} bytes, more than {MAX_PAGE}", html.len())),
            types::Step::Page(html) => Step::Page(html),
            types::Step::Redirect(to) if is_local(&to) || secure(&to) => Step::Redirect(to),
            types::Step::Redirect(to) => self.failed(&format!("redirect to {to:?}: give an https URL or a path on this site")),
            types::Step::Done(id) => match Identity::new(&id.id, &id.email, &id.name, id.groups) {
                Ok(id) => Step::Done(id),
                Err(e) => {
                    eprintln!("sign-in: {} signed someone in without an email: {e}", self.file);
                    Step::Refuse(format!("Sign-in didn't work: {e}."))
                }
            },
            types::Step::Refuse(why) => Step::Refuse(why),
            types::Step::Unavailable(why) => Step::Unavailable(why),
        }
    }

    fn failed(&self, why: &str) -> Step {
        eprintln!("sign-in: {}: {why}", self.file);
        Step::Unavailable("Sign-in isn't working right now. Try again in a moment.".into())
    }
}

/// The request a sign-in kit gets, for the sign-in `pending`.
pub fn request(pending: &Pending, origin: &str, method: &str, path: &str, query: Vec<(String, String)>, form: Vec<(String, String)>) -> Request {
    let origin = origin.trim_end_matches('/');
    Request {
        method: method.into(),
        path: path.into(),
        query,
        form,
        origin: origin.into(),
        callback: format!("{origin}/_auth/callback"),
        state: pending.state.clone(),
        nonce: pending.nonce.clone(),
        verifier: pending.verifier.clone(),
        challenge: B64.encode(Sha256::digest(pending.verifier.as_bytes())),
    }
}

/// Compiles the component, refuses imports outside the sign-in world and
/// the allowed WASI, and checks it exports the provider.
fn compile(bytes: &[u8]) -> anyhow::Result<SignInPre<Ctx>> {
    let engine = crate::runtime::engine()?;
    let component = Component::new(&engine, bytes).map_err(anyhow::Error::from).context("not a valid WebAssembly component")?;
    let mut problems = Vec::new();
    for (import, _) in imports(&engine, &component) {
        let own = import.strip_prefix(PACKAGE).is_some_and(|i| OWN.contains(&i));
        if !own && !ALLOWED_WASI.iter().any(|w| import.starts_with(w)) {
            problems.push(format!("imports {import}, which sign-in kits can never use"));
        }
    }
    if !problems.is_empty() {
        bail!("{}", problems.join("; "));
    }
    let mut linker: Linker<Ctx> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    types::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    config::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    http::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    let pre = linker.instantiate_pre(&component)?;
    SignInPre::new(pre).map_err(anyhow::Error::from).context("does not export appos:auth/provider (start and finish)")
}

struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    hosts: Arc<Vec<String>>,
    config: Arc<BTreeMap<String, String>>,
    deadline: Instant,
    fetches: u32,
}

impl WasiView for Ctx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl types::Host for Ctx {}

impl config::Host for Ctx {
    fn get(&mut self, key: String) -> wasmtime::Result<Option<String>> {
        Ok(self.config.get(&key).cloned())
    }
}

impl http::Host for Ctx {
    async fn fetch(&mut self, req: http::Request) -> wasmtime::Result<Result<http::Response, String>> {
        self.fetches += 1;
        if self.fetches > MAX_FETCHES {
            return Ok(Err(format!("more than {MAX_FETCHES} fetches in one sign-in step")));
        }
        if let Err(e) = allowed(&req.url, &self.hosts) {
            eprintln!("sign-in: fetch refused: {e}");
            return Ok(Err(e));
        }
        let left = self.deadline.saturating_duration_since(Instant::now());
        let sent = tokio::task::spawn_blocking(move || send(req, left)).await.map_err(|e| wasmtime::format_err!("fetch worker failed: {e}"))?;
        Ok(sent)
    }
}

/// Whether a sign-in kit may fetch `url`: https to one of `hosts`, or http to
/// one of them on this machine.
fn allowed(url: &str, hosts: &[String]) -> Result<(), String> {
    let uri: ureq::http::Uri = url.parse().map_err(|_| format!("{url:?} is no URL"))?;
    let host = uri.host().unwrap_or_default().to_lowercase();
    let listed = hosts.contains(&host);
    let local = ["localhost", "127.0.0.1", "[::1]", "::1"].contains(&host.as_str());
    match uri.scheme_str() {
        _ if !listed => Err(format!("{host:?} is not in kit.toml's hosts ({})", if hosts.is_empty() { "none".to_string() } else { hosts.join(", ") })),
        Some("https") => Ok(()),
        Some("http") if local => Ok(()),
        _ => Err(format!("{url:?}: only https (http only to localhost)")),
    }
}

fn send(req: http::Request, left: Duration) -> Result<http::Response, String> {
    if left.is_zero() {
        return Err("no time left in this sign-in step".into());
    }
    let method = req.method.to_ascii_uppercase();
    if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
        return Err(format!("method {method}: give GET, POST, PUT, PATCH or DELETE"));
    }
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(left)).http_status_as_error(false).max_redirects(0).max_redirects_will_error(false).build().into();
    let mut builder = ureq::http::Request::builder().method(method.as_str()).uri(req.url.as_str());
    for (name, value) in &req.headers {
        if name.eq_ignore_ascii_case("host") {
            return Err("the host header is the URL's".into());
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    let answer = if req.body.is_empty() {
        agent.run(builder.body(()).map_err(|e| e.to_string())?)
    } else {
        agent.run(builder.body(req.body).map_err(|e| e.to_string())?)
    };
    let mut resp = answer.map_err(|e| e.to_string())?;
    let headers = resp.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string()))).collect();
    let status = resp.status().as_u16();
    let body = resp.body_mut().with_config().limit(MAX_BODY).read_to_string().map_err(|e| e.to_string())?;
    Ok(http::Response { status, headers, body })
}

#[cfg(test)]
mod tests {
    use axum::http::{StatusCode, header};

    use super::super::routes::tests::{cookies, get, post, send};
    use super::*;
    use crate::shell::Shell;
    use crate::web::escape;

    const WASM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/auth/testing-plugin.wat");

    /// A sign-in kit's directory: kit.toml, with `rest` after [kit], and
    /// testing-plugin.wat.
    fn kit_dir(rest: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kit.toml"), format!("[kit]\nid = \"acme-sign-in\"\nname = \"Acme\"\nversion = \"1\"\nwasm = \"{WASM}\"\n{rest}"))
            .unwrap();
        dir
    }

    /// A shell signing in with testing-plugin.wat, which fetches `url`.
    fn shell(hosts: &str, url: Option<&str>) -> Arc<Shell> {
        let config = url.map(|u| format!("[config]\nurl = \"{u}\"\n")).unwrap_or_default();
        let dir = kit_dir(&format!("[sign-in]\nhosts = [\"{hosts}\"]\n{config}"));
        let shell = crate::testing::shell_with_auth(&format!("provider = \"kit\"\nkit = \"{}\"\n", dir.path().display()));
        drop(dir);
        shell
    }

    /// An identity service on a free port: /me answers an email, /moved
    /// redirects there.
    async fn service() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let app = axum::Router::new()
            .route("/me", axum::routing::get(|| async { "ann@acme.com" }))
            .route("/moved", axum::routing::get(|| async { axum::response::Redirect::to("/me") }));
        tokio::spawn(async move { axum::serve(listener, app).await });
        url
    }

    /// Starts a sign-in and posts its form.
    async fn sign_in(app: &axum::Router) -> (StatusCode, axum::http::HeaderMap, String) {
        let (status, headers, body) = send(app, get("/_auth/sign-in?to=/elsewhere", "")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("plugin page"), "{body}");
        send(app, post("/_auth/sign-in", &cookies(&headers), "")).await
    }

    #[tokio::test]
    async fn a_sign_in_kit_signs_in_as_what_it_fetched() {
        let me = format!("{}/me", service().await);
        let app = crate::web::router(shell("127.0.0.1", Some(&me)));
        let (status, headers, body) = sign_in(&app).await;
        assert_eq!((status, headers.get(header::LOCATION).map(|l| l.to_str().unwrap())), (StatusCode::SEE_OTHER, Some("/elsewhere")), "{body}");
        let session = cookies(&headers).split("; ").find(|c| c.starts_with("appos-session=")).unwrap().to_string();
        let (_, _, body) = send(&app, get("/", &session)).await;
        assert!(body.contains("ann@acme.com"), "{body}");
    }

    #[tokio::test]
    async fn a_sign_in_kit_reaches_only_its_hosts_and_follows_no_redirect() {
        let service = service().await;
        for (hosts, url, said) in [
            ("login.acme.com", format!("{service}/me"), "\"127.0.0.1\" is not in kit.toml's hosts (login.acme.com)"),
            ("127.0.0.1", format!("{service}/moved"), "\"\" is not an email address"),
        ] {
            let app = crate::web::router(shell(hosts, Some(&url)));
            let (status, _, body) = sign_in(&app).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            assert!(body.contains(&escape(said)), "{said}: {body}");
        }
    }

    #[tokio::test]
    async fn a_sign_in_kit_that_fails_shows_sign_in_unavailable_and_the_site_goes_on() {
        let app = crate::web::router(shell("127.0.0.1", None));
        let (status, _, body) = sign_in(&app).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body.contains("Sign-in isn&#39;t working right now"), "{body}");
        assert_eq!(send(&app, get("/", "")).await.0, StatusCode::OK);
    }

    #[test]
    fn the_example_sign_in_kit_carries_this_contract() {
        let example = include_str!("../../../examples/auth-github/wit/auth.wit");
        assert!(example == include_str!("../../../wit-auth/auth.wit"), "copy wit-auth/auth.wit to examples/auth-github/wit/");
    }

    #[test]
    fn its_redirects_and_pages_are_checked() {
        let dir = kit_dir("[sign-in]\n");
        let p = SignInKit::parse(&mut toml::from_str("kit = \".\"").unwrap(), dir.path(), &|_| None).unwrap();
        let redirect = |to: &str| matches!(p.step(types::Step::Redirect(to.into())), Step::Redirect(_));
        assert!(redirect("https://login.acme.com/authorize?x=1") && redirect("/webinars") && redirect("http://localhost:9000/x"));
        assert!(!redirect("http://login.acme.com/") && !redirect("//evil.example/") && !redirect("javascript:alert(1)"));
        assert!(matches!(p.step(types::Step::Page("x".repeat(MAX_PAGE + 1))), Step::Unavailable(_)));
        let done = |email: &str| p.step(types::Step::Done(types::Identity { id: "".into(), email: email.into(), name: "".into(), groups: vec![] }));
        assert!(matches!(done("Ann@Acme.com"), Step::Done(id) if id.email == "ann@acme.com"));
        assert!(matches!(done("nobody"), Step::Refuse(_)));
    }

    #[test]
    fn sign_in_kits_fetch_only_from_their_hosts() {
        let hosts = vec!["login.acme.com".to_string(), "127.0.0.1".to_string()];
        assert!(allowed("https://login.acme.com/token", &hosts).is_ok());
        assert!(allowed("https://LOGIN.acme.com:8443/x", &hosts).is_ok());
        assert!(allowed("http://127.0.0.1:9000/me", &hosts).is_ok());
        assert!(allowed("http://login.acme.com/token", &hosts).unwrap_err().contains("only https"));
        assert!(allowed("https://evil.example/", &hosts).unwrap_err().contains("not in kit.toml's hosts"));
        assert!(allowed("https://login.acme.com@evil.example/", &hosts).is_err());
        assert!(allowed("https://login.acme.com.evil.example/", &hosts).is_err());
        assert!(allowed("ftp://login.acme.com/", &hosts).is_err());
        assert!(allowed("/relative", &hosts).is_err());
    }

    #[test]
    fn the_kit_says_what_it_may_use() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("p.wat"), include_str!("testing-plugin.wat")).unwrap();
        std::fs::write(dir.path().join("app.wat"), include_str!("../testing.wat")).unwrap();
        let parse = |kit_toml: &str| {
            std::fs::write(dir.path().join("kit.toml"), format!("[kit]\nid = \"acme-sign-in\"\nname = \"Acme\"\nversion = \"1\"\n{kit_toml}")).unwrap();
            SignInKit::parse(&mut toml::from_str("kit = \".\"").unwrap(), dir.path(), &|var| (var == "SET").then(|| "s3cret".into()))
                .map(|p| p.config.as_ref().clone())
                .map_err(|e| format!("{e:#}"))
        };
        let config = parse("wasm = \"p.wat\"\n[sign-in]\nhosts = [\"login.acme.com\"]\n[config]\nurl = \"u\"\n[config-env]\nsecret = \"SET\"").unwrap();
        assert_eq!(config.into_iter().collect::<Vec<_>>(), [("secret".to_string(), "s3cret".to_string()), ("url".into(), "u".into())]);
        assert!(parse("wasm = \"p.wat\"").unwrap_err().contains("no [sign-in]: not a sign-in kit"));
        assert!(parse("wasm = \"missing.wasm\"\n[sign-in]").unwrap_err().contains("missing.wasm"));
        assert!(parse("wasm = \"p.wat\"\n[sign-in]\nhosts = [\"https://login.acme.com\"]").unwrap_err().contains("give the host alone"));
        assert!(parse("wasm = \"p.wat\"\n[sign-in]\n[config-env]\nsecret = \"UNSET\"").unwrap_err().contains("UNSET is not set"));
        assert!(parse("wasm = \"p.wat\"\n[sign-in]\n[config]\nn = 1").unwrap_err().contains("invalid type"));
        assert!(parse("wasm = \"p.wat\"\nwit = [\"x.wit\"]\n[sign-in]").unwrap_err().contains("offers apps nothing"));
        let app = parse("wasm = \"app.wat\"\n[sign-in]").unwrap_err();
        assert!(app.contains("imports appos:framework/registry, which sign-in kits can never use"), "{app}");
        let err = crate::auth::Auth::parse_in("provider = \"kit\"", "http://localhost", [7; 32], "127.0.0.1:8080", &|_| None, dir.path()).err().unwrap();
        assert!(err.to_string().contains("provider \"kit\" needs kit = \"sign-in\""), "{err}");
    }
}
