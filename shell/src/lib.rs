//! The AppOS shell as a library, so both `appos-shell` and `appos dev` run
//! the very same shell.

#[macro_use]
mod names;

mod access;
mod admin;
pub mod api;
mod args;
mod auth;
pub mod bundle;
mod deploy;
pub mod deps;
mod fetch;
pub mod kit;
mod log_filter;
mod manifest;
mod outbox;
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
mod wasm_kit;
mod web;

pub use manifest::{APP_TOML, KIT_TOML, Manifest, Risk, validate_app_id, validate_grant_name};

use std::path::PathBuf;
use std::sync::Arc;

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
    /// Directories of Wasm kits, each with its kit.toml (wasm_kit.rs):
    /// loaded with the platform's own.
    pub kits: Vec<PathBuf>,
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
    /// `appos dev`: no policy, as with `no_auth`, but the dev sign-in
    /// (auth/dev.rs) with a test user filled in, so pages can be tried
    /// signed in and anonymous.
    pub dev_sign_in: bool,
}

/// A platform: the shell and its kits, what apps call besides the page
/// (kit.rs).
///
/// ```ignore
/// appos_shell::Platform::new()
///     .kit(Crm::new(crm_db))
///     .serve(opts)
///     .await
/// ```
pub struct Platform {
    /// A mistake, like two kits offering the same grant, stops `serve`.
    kits: anyhow::Result<kit::Kits>,
}

impl Default for Platform {
    fn default() -> Self {
        Platform { kits: Ok(kit::Kits::builtin()) }
    }
}

impl Platform {
    /// A platform with kv, which every platform has.
    pub fn new() -> Platform {
        Platform::default()
    }

    /// With one kit more.
    pub fn kit(self, kit: impl kit::Kit) -> Platform {
        Platform { kits: self.kits.and_then(|k| k.with(kit)) }
    }

    /// Loads the apps and serves them until the process ends.
    pub async fn serve(self, opts: Options) -> anyhow::Result<()> {
        serve_with(opts, self.kits?).await
    }

    /// The platform's binary: `appos-shell`'s command line with this
    /// platform's kits. `--contract` prints the WIT apps build against
    /// (`appos sync --shell`), `--dev APP...` serves apps as `appos dev`
    /// does (`appos dev --shell`). Exits on an error.
    pub async fn main(self) {
        let result = async {
            let kits = self.kits?;
            match args::parse(std::env::args().skip(1))? {
                args::Run::Serve(opts) => serve_with(*opts, kits).await,
                args::Run::Dev(mut opts) => {
                    opts.reload = Some(args::stdin_reloads());
                    serve_with(*opts, kits).await
                }
                args::Run::Contract(dirs) => {
                    let kits = wasm_kit::add_dirs(kits, &wasm_kit::kit_dirs(&dirs)?)?;
                    crate::terminal_println!("{}", serde_json::to_string_pretty(&kits.contract())?);
                    Ok(())
                }
                args::Run::Fakes(dir) => {
                    let n = kits.write_fakes(&dir)?;
                    crate::terminal_println!("{n} fakes in {}", dir.display());
                    Ok(())
                }
                args::Run::Nothing => Ok(()),
            }
        }
        .await;
        if let Err(error) = result {
            // Synchronous: the log queue's thread would not get to it
            // before exit.
            crate::terminal_eprintln!("error: {error:#}");
            std::process::exit(1);
        }
    }
}

impl Options {
    /// What `appos dev` serves: the app in `dir`, with its data in
    /// dir/appos-data, open (no sign-in), keeping the examples
    /// `appos review` starts from, reloading on each message of `reload`.
    pub fn dev(dir: PathBuf, addr: String, reload: tokio::sync::mpsc::Receiver<()>) -> Options {
        Options {
            addr,
            platform_addr: None,
            platform_api: false,
            name: "dev".into(),
            git: None,
            origin: None,
            data: dir.join("appos-data"),
            kits: fakes(&dir),
            apps: vec![dir],
            reload: Some(reload),
            risk: Default::default(),
            keep_examples: true,
            auth: None,
            policy: None,
            no_auth: true,
            dev_sign_in: true,
        }
    }
}

/// The kits `appos dev` runs the app at `dir` with: the built-in ones and
/// the fakes `appos sync` put into its fakes/.
pub fn dev_kits(dir: &std::path::Path) -> anyhow::Result<kit::Kits> {
    wasm_kit::add_dirs(kit::Kits::builtin(), &fakes(dir))
}

/// The fakes `appos sync` put into the app at `dir` (fakes/<kit>/kit.toml):
/// Wasm kits that stand in for the platform's.
fn fakes(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir.join("fakes")) else { return vec![] };
    let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("kit.toml").is_file()).collect();
    dirs.sort();
    dirs
}

/// Loads the apps from directories, with the checks a deployed bundle gets
/// (bundle.rs). An app that fails to load is refused with a message; the
/// others still run.
async fn load_all(shell: &shell::Shell, paths: &[PathBuf]) {
    for path in paths {
        let loaded = async {
            let path = path.clone();
            let checked = tokio::task::spawn_blocking(move || bundle::Bundle::from_dir(&path)?.checked()).await??;
            let app = Arc::new(runtime::load(&shell.engine, shell.db.clone(), &shell.kits.now(), &checked, &shell.data).await?);
            shell.state.insert_loaded([app.clone()]).await?;
            anyhow::Ok(app)
        }
        .await;
        match loaded {
            Ok(app) => {
                let m = &app.manifest.app;
                println!("loaded {} {} at /{} ({} routes)", m.id, app.label, m.id, app.routes.len());
            }
            Err(e) => eprintln!("refused {}: {e:#}", path.display()),
        }
    }
}

/// Requests carry the address they come from: the header provider
/// believes only its proxies (auth/proxy.rs).
fn with_peer(router: axum::Router) -> axum::extract::connect_info::IntoMakeServiceWithConnectInfo<axum::Router, std::net::SocketAddr> {
    router.into_make_service_with_connect_info::<std::net::SocketAddr>()
}

/// Loads the apps and serves them until the process ends: a platform with
/// kv only.
pub async fn serve(opts: Options) -> anyhow::Result<()> {
    Platform::new().serve(opts).await
}

async fn serve_with(mut opts: Options, kits: kit::Kits) -> anyhow::Result<()> {
    check_auth(&opts)?;
    opts.risk.check(&kits)?;
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

    let origin = opts.origin.clone().unwrap_or_else(|| format!("http://{}", opts.addr));
    match (opts.no_auth, opts.dev_sign_in) {
        (true, true) => println!("no policy: anyone may do anything. Anonymous until you sign in, as a test user or anyone: {origin}/_auth/sign-in"),
        (true, false) => println!("--no-auth: no sign-in and no policy; anyone may do anything, on the admin screen and the platform API too"),
        _ => {}
    }
    // Neither --auth nor --no-auth: the token sign-in (auth/token.rs).
    let token = (opts.auth.is_none() && !opts.no_auth).then(|| auth::session::random(24));
    let auth = match (&opts.auth, &token) {
        (Some(path), _) => Some(auth::Auth::load(path, &origin, &opts.data, &opts.addr)?),
        (None, Some(secret)) => Some(auth::Auth::token(secret.clone(), &origin, &opts.data)?),
        (None, None) if opts.dev_sign_in => Some(auth::Auth::dev(&origin, &opts.data)?),
        (None, None) => None,
    };
    let policy = match &opts.policy {
        Some(path) => {
            let policies = access::Policies::load(path, kits.names()).await?;
            println!("{}: {} rules", policies.file_name(), policies.current().rules());
            Some(Arc::new(policies))
        }
        None if token.is_some() => Some(Arc::new(access::Policies::built_in(&auth::token::policy(), kits.names()).await?)),
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
        kits: kits.clone(),
        keep_examples: opts.keep_examples,
        auth,
        policy,
    })?;
    access::watch(shell.clone());
    // The kits first, then the apps that use them.
    let deployed = deploy::load_kits(&shell, &opts.kits).await?;
    load_all(&shell, &opts.apps).await;
    deploy::load_live(&shell, deployed).await;
    shell.outbox.wake();

    if let Some(mut reload) = opts.reload.take() {
        let shell = shell.clone();
        let paths = opts.apps.clone();
        tokio::spawn(async move {
            while reload.recv().await.is_some() {
                load_all(&shell, &paths).await;
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
            kits: vec![],
            reload: None,
            risk: Default::default(),
            keep_examples: false,
            auth: auth.then(|| "auth.toml".into()),
            policy: policy.then(|| "policy.csv".into()),
            no_auth,
            dev_sign_in: false,
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
            kits: vec![],
            reload: None,
            risk: Default::default(),
            keep_examples: false,
            auth: None,
            policy: None,
            no_auth: false,
            dev_sign_in: false,
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
