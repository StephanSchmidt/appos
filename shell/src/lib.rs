//! The AppOS shell as a library, so both `appos-shell` and `appos dev` run
//! the very same shell.

#[macro_use]
mod names;

mod access;
mod admin;
pub mod api;
mod auth;
pub mod bundle;
mod deploy;
pub mod deps;
mod log_filter;
mod manifest;
mod platform;
pub mod review;
mod runtime;
mod shell;
mod state;
pub mod terminal;
#[cfg(test)]
mod testing;
mod tx;
mod usage;
mod web;

pub use manifest::{Manifest, Risk, validate_app_id};

use std::path::PathBuf;
use std::sync::Arc;

use wasmtime::Engine;
// Native status messages are logs; closed output pipes must not stop the server.
use crate::{terminal_errorln as eprintln, terminal_logln as println};

pub struct Options {
    pub addr: String,
    /// Where the platform API (`/_platform/...`) and the admin screen
    /// (`/admin`) listen, e.g. an internal IP. Without it, they are served
    /// on `addr`.
    pub platform_addr: Option<String>,
    /// Serve the platform API and the admin screen at all. `appos dev`
    /// doesn't.
    pub platform_api: bool,
    /// The platform's name, shown by `appos platforms`.
    pub name: String,
    /// Where app repositories live, e.g. "https://gitlab.acme.com/marketing-apps".
    /// `appos new` tells the user to push an app to `<git>/<app-id>.git`.
    pub git: Option<String>,
    /// The public origin; defaults to `http://<addr>`.
    pub origin: Option<String>,
    /// Where the shell keeps its data: kv, the platform database and the
    /// apps deployed to it.
    pub data: PathBuf,
    /// App directories, or their `app.toml` files. A deployed app of the
    /// same id replaces one of these.
    pub apps: Vec<PathBuf>,
    /// Each message reloads every app from disk (used by `appos dev`).
    pub reload: Option<tokio::sync::mpsc::Receiver<()>>,
    /// The grants' risk levels, as the company rates them.
    pub risk: manifest::Risk,
    /// Keep each route's last request that worked, as the example
    /// `appos review` starts from (review.rs). `appos dev` does.
    pub keep_examples: bool,
    /// auth.toml: how people sign in; it goes with `policy`. Without
    /// either (and without `no_auth`), the token sign-in: a link printed at
    /// start signs in as an admin who may do everything (auth/token.rs).
    pub auth: Option<PathBuf>,
    /// policy.csv: who may call what (access.rs).
    pub policy: Option<PathBuf>,
    /// No sign-in and no policy: anyone may use every grant an app holds,
    /// and the admin screen. Only when asked for (`--no-auth`, `appos dev`).
    pub no_auth: bool,
}

/// Loads the apps from directories, with the checks a deployed bundle gets
/// (bundle.rs). An app that fails to load is refused with a message; the
/// others still run.
async fn load_all(engine: &Engine, db: &tx::Db, paths: &[PathBuf]) -> Vec<Arc<runtime::LoadedApp>> {
    let mut apps = Vec::new();
    for path in paths {
        let loaded = async {
            let path = path.clone();
            let checked = tokio::task::spawn_blocking(move || bundle::Bundle::from_dir(&path)?.checked()).await??;
            runtime::load(engine, db.clone(), &checked).await
        }
        .await;
        match loaded {
            Ok(app) => {
                let m = &app.manifest.app;
                println!("loaded {} {} at /{} ({} routes)", m.id, app.label, m.id, app.routes.len());
                apps.push(Arc::new(app));
            }
            Err(e) => eprintln!("refused {}: {e:#}", path.display()),
        }
    }
    apps
}

/// Requests carry the address they come from: the header provider
/// believes only its proxies (auth/proxy.rs).
fn with_peer(router: axum::Router) -> axum::extract::connect_info::IntoMakeServiceWithConnectInfo<axum::Router, std::net::SocketAddr> {
    router.into_make_service_with_connect_info::<std::net::SocketAddr>()
}

/// Loads the apps and serves them until the process ends.
pub async fn serve(mut opts: Options) -> anyhow::Result<()> {
    check_auth(&opts)?;
    if opts.no_auth {
        println!("--no-auth: no sign-in and no policy; anyone may do anything, on the admin screen and the platform API too");
    }
    let engine = runtime::engine()?;
    let db = tx::open_db(&opts.data.join("appos.db"))?;
    // No platform.db yet: the first start, with no last one to compare with.
    let first = !opts.data.join("platform.db").exists();
    let platform = platform::PlatformDb::open(&opts.data.join("platform.db"))?;
    match platform.risk_levels(opts.risk.levels()) {
        Ok(0) => {}
        Ok(_) if first => {}
        Ok(n) => println!("risk.toml: {n} grants rated differently than at the last start"),
        Err(e) => eprintln!("could not keep the risk levels: {e:#}"),
    }
    let apps = load_all(&engine, &db, &opts.apps).await;

    let origin = opts.origin.clone().unwrap_or_else(|| format!("http://{}", opts.addr));
    // Neither --auth nor --no-auth: the token sign-in (auth/token.rs).
    let token = (opts.auth.is_none() && !opts.no_auth).then(|| auth::session::random(24));
    let auth = match (&opts.auth, &token) {
        (Some(path), _) => Some(auth::Auth::load(path, &origin, &opts.data, &opts.addr)?),
        (None, Some(secret)) => Some(auth::Auth::token(secret.clone(), &origin, &opts.data)?),
        (None, None) => None,
    };
    let policy = match &opts.policy {
        Some(path) => {
            let policies = access::Policies::load(path).await?;
            println!("{}: {} rules", policies.file_name(), policies.current().rules());
            Some(policies)
        }
        None if token.is_some() => Some(access::Policies::built_in(&auth::token::policy()).await?),
        None => None,
    };
    let shell = shell::Shell::new(shell::Parts {
        origin: origin.clone(),
        name: opts.name.clone(),
        git: opts.git.clone(),
        engine: engine.clone(),
        db: db.clone(),
        data: opts.data.clone(),
        platform,
        risk: opts.risk.clone(),
        keep_examples: opts.keep_examples,
        auth,
        policy,
    })?;
    access::watch(shell.clone());
    shell.state.insert_loaded(apps);
    deploy::load_live(&shell).await?;

    if let Some(mut reload) = opts.reload.take() {
        let shell = shell.clone();
        let paths = opts.apps.clone();
        tokio::spawn(async move {
            while reload.recv().await.is_some() {
                shell.state.insert_loaded(load_all(&engine, &db, &paths).await);
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(&opts.addr).await?;
    println!("AppOS shell on {origin}");
    let connect = |url: &str| {
        println!("platform API on {url}/_platform (connect: appos login {url}), admin screen on {url}/admin");
        if let Some(secret) = &token {
            println!("sign in as {}, who may do everything: {url}/_auth/sign-in?token={secret}&to=/admin", auth::token::EMAIL);
            println!("  (on this machine only, new at every start; without it nobody may use the admin screen, the platform API or an app's grants)");
        }
    };
    if let (Some(secret), false) = (&token, opts.platform_api) {
        println!("sign in as {}, who may do everything: {origin}/_auth/sign-in?token={secret}", auth::token::EMAIL);
    }
    match (&opts.platform_addr, opts.platform_api) {
        (_, false) => axum::serve(listener, with_peer(web::router(shell))).await?,
        (None, true) => {
            connect(&origin);
            axum::serve(listener, with_peer(web::router(shell.clone()).merge(web::platform_router(shell, None)))).await?;
        }
        (Some(addr), true) => {
            let platform = tokio::net::TcpListener::bind(addr).await?;
            connect(&format!("http://{addr}"));
            let site = axum::serve(listener, with_peer(web::router(shell.clone())));
            let api = axum::serve(platform, with_peer(web::platform_router(shell, Some(&format!("http://{addr}")))));
            tokio::try_join!(async { site.await }, async { api.await })?;
        }
    }
    Ok(())
}

/// Safe unless told otherwise: sign-in and a policy, the token sign-in
/// (neither given), or `--no-auth` and neither.
fn check_auth(opts: &Options) -> anyhow::Result<()> {
    match (opts.no_auth, opts.auth.is_some(), opts.policy.is_some()) {
        (false, true, true) | (true, false, false) => Ok(()),
        (false, false, false) => only_here(opts),
        (true, ..) => anyhow::bail!("--no-auth means no sign-in and no policy: drop --auth and --policy, or --no-auth"),
        (false, true, false) => {
            anyhow::bail!("--auth needs --policy policy.csv: without one, anyone may do anything, on the admin screen and the platform API too")
        }
        (false, false, true) => anyhow::bail!("--policy needs --auth auth.toml, to know who is who"),
    }
}

/// The token sign-in prints its link to whoever starts the shell: only for
/// a shell on this machine, never one other machines reach, by its address
/// or through a proxy (`--origin`).
fn only_here(opts: &Options) -> anyhow::Result<()> {
    let addrs = std::iter::once(&opts.addr).chain(&opts.platform_addr);
    let reached = addrs.map(|a| (a.as_str(), auth::is_loopback(a))).chain(opts.origin.as_deref().map(|o| (o, local_origin(o))));
    match reached.into_iter().find(|(_, local)| !local) {
        Some((place, _)) => anyhow::bail!(
            "{place} is reached from other machines: give --auth auth.toml --policy policy.csv (how people sign in, who may do what), or --no-auth to run it open. Without them the shell runs on this machine only, signed in with a link it prints"
        ),
        None => Ok(()),
    }
}

fn local_origin(origin: &str) -> bool {
    let host = origin.parse::<axum::http::Uri>().ok().and_then(|u| u.host().map(|h| h.trim_matches(['[', ']']).to_ascii_lowercase()));
    host.is_some_and(|h| h == "localhost" || h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()))
}

#[cfg(test)]
mod check_auth_tests {
    use super::*;

    #[test]
    fn the_shell_needs_sign_in_and_a_policy_or_no_auth() {
        let opts = |auth: bool, policy: bool, no_auth: bool| Options {
            addr: "127.0.0.1:0".into(),
            platform_addr: None,
            platform_api: true,
            name: "test".into(),
            git: None,
            origin: None,
            data: PathBuf::new(),
            apps: vec![],
            reload: None,
            risk: Default::default(),
            keep_examples: false,
            auth: auth.then(|| "auth.toml".into()),
            policy: policy.then(|| "policy.csv".into()),
            no_auth,
        };
        assert!(check_auth(&opts(true, true, false)).is_ok());
        assert!(check_auth(&opts(false, false, true)).is_ok());
        assert!(check_auth(&opts(false, false, false)).is_ok(), "the token sign-in");
        for (auth, policy, no_auth) in [(true, false, false), (false, true, false), (true, true, true), (true, false, true)] {
            assert!(check_auth(&opts(auth, policy, no_auth)).is_err(), "auth {auth}, policy {policy}, --no-auth {no_auth}");
        }
    }

    #[test]
    fn the_token_sign_in_is_for_this_machine_only() {
        let opts = |addr: &str, platform: Option<&str>, origin: Option<&str>| Options {
            addr: addr.into(),
            platform_addr: platform.map(Into::into),
            platform_api: true,
            name: "test".into(),
            git: None,
            origin: origin.map(Into::into),
            data: PathBuf::new(),
            apps: vec![],
            reload: None,
            risk: Default::default(),
            keep_examples: false,
            auth: None,
            policy: None,
            no_auth: false,
        };
        for ok in [opts("127.0.0.1:8080", None, None), opts("localhost:8080", Some("[::1]:9090"), Some("http://localhost:8080"))] {
            assert!(check_auth(&ok).is_ok());
        }
        for (bad, place) in [
            (opts("0.0.0.0:8080", None, None), "0.0.0.0:8080"),
            (opts("127.0.0.1:8080", Some("10.0.0.5:9090"), None), "10.0.0.5:9090"),
            (opts("127.0.0.1:8080", None, Some("https://apps.acme.com")), "https://apps.acme.com"),
        ] {
            let err = check_auth(&bad).unwrap_err().to_string();
            assert!(err.starts_with(place) && err.contains("--no-auth"), "{err}");
        }
        assert!(check_auth(&Options { no_auth: true, ..opts("0.0.0.0:8080", None, None) }).is_ok(), "open, when asked for");
    }
}
