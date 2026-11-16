//! The sandbox. Apps are WebAssembly components run by wasmtime. Each app's
//! linker holds only the interfaces its manifest grants, and WASI has no
//! files, no network and no environment.
//!
//! `init` runs once when the app is loaded and registers routes and menu
//! entries. `call` runs once per request, in a fresh instance, as one kv
//! transaction (tx.rs).
//!
//! A request has a budget: fuel (instructions), memory, I/O units and time.
//! Fuel stops a loop, but not an app waiting in a host call, such as a
//! sleep; so apps run async, and the time limit drops the request wherever
//! it is. CPU-bound code yields every tick of the epoch so the limit can
//! strike there too.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use tokio::time::{Instant, timeout_at};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable, types::ComponentItem};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::access::Access;
use crate::bundle::Checked;
use crate::manifest::{Grant, Manifest};
use crate::platform::Outcome;
use crate::tx::{Db, Tx};

wasmtime::component::bindgen!({
    world: "app",
    path: "../wit",
    imports: {
        "appos:framework/kv.get": async | trappable,
        "appos:framework/kv.scan": async | trappable,
        default: trappable,
    },
    exports: { default: async },
});

use appos::framework::{kv, log, page, registry, types};
pub use types::Request;

/// Instructions one request may execute.
pub const FUEL: u64 = 1_000_000_000;
pub const MEMORY: usize = 64 * 1024 * 1024;

/// What an admin set for one app's requests on the console, kept in
/// platform.db across deploys; none is the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdminLimits {
    pub memory_mb: Option<u32>,
    pub seconds: Option<u32>,
    /// Millions of instructions.
    pub fuel_m: Option<u32>,
}

impl AdminLimits {
    /// What an admin may set: memory in MB, seconds (as app.toml may ask),
    /// millions of instructions.
    pub const MEMORY_MB: std::ops::RangeInclusive<u32> = 16..=512;
    pub const SECONDS: std::ops::RangeInclusive<u32> = 1..=crate::manifest::MAX_SECONDS as u32;
    pub const FUEL_M: std::ops::RangeInclusive<u32> = 250..=8000;

    pub fn check(&self) -> Result<(), String> {
        let out = |what: &str, v: Option<u32>, r: &std::ops::RangeInclusive<u32>, unit: &str| match v {
            Some(v) if !r.contains(&v) => Err(format!("{what} {v} {unit}: must be {} to {} {unit}", r.start(), r.end())),
            _ => Ok(()),
        };
        out("memory", self.memory_mb, &Self::MEMORY_MB, "MB")?;
        out("time", self.seconds, &Self::SECONDS, "s")?;
        out("fuel", self.fuel_m, &Self::FUEL_M, "million instructions")
    }
}

/// What one request may use: the platform's defaults, app.toml's time and
/// what an admin set over both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub fuel: u64,
    pub memory: usize,
    pub time: Duration,
}

impl Budget {
    pub fn of(m: &Manifest, set: AdminLimits) -> Budget {
        Budget::with(m.time_limit(), set)
    }

    /// With `time` as app.toml asks for it.
    pub fn with(time: Duration, set: AdminLimits) -> Budget {
        Budget {
            fuel: set.fuel_m.map_or(FUEL, |f| u64::from(f) * 1_000_000),
            memory: set.memory_mb.map_or(MEMORY, |mb| mb as usize * 1024 * 1024),
            time: set.seconds.map_or(time, |s| Duration::from_secs(s.into())),
        }
    }
}

/// How long a request waits in line for a busy app before it gets 503.
pub const QUEUE_WAIT: Duration = Duration::from_secs(10);
/// Host calls one request may make: a read costs READ, a write WRITE.
pub const IO_BUDGET: u32 = 100;
pub const READ: u32 = 1;
pub const WRITE: u32 = 5;
/// Entries one page may add to <head>.
const MAX_HEAD: usize = 20;
/// Retained page output or init's route/menu metadata, outside Wasm memory.
const MAX_HOST_DATA: usize = 4 * 1024 * 1024;
/// Bytes of a kv key, and of a value. Writes wait in memory until the
/// request ends, so they are kept small.
pub const MAX_KEY: usize = 1024;
pub const MAX_VALUE: usize = 1024 * 1024;
/// Bytes of log lines one request may print.
const MAX_LOG: usize = 64 * 1024;
/// How often one request runs at most when other requests keep changing
/// what it read.
const MAX_RUNS: u32 = 10;
/// Elements of all tables of one request.
const MAX_TABLE_ELEMENTS: usize = 100_000;
/// How often running apps yield, so a time limit can stop them.
const TICK: Duration = Duration::from_millis(10);

pub(crate) const FRAMEWORK: &str = "appos:framework/";
/// Interfaces every app may import without a grant.
pub(crate) const ALWAYS: &[&str] = &["types", "registry", "log", "page", "handler"];
/// WASI interfaces apps may import. They are backed by an empty context.
pub(crate) const ALLOWED_WASI: &[&str] = &["wasi:io/", "wasi:clocks/", "wasi:random/", "wasi:cli/"];

#[derive(Debug, Clone)]
pub struct Route {
    pub method: String,
    pub segments: Vec<String>,
    pub handler: String,
}

impl Route {
    /// As written in the app, like "GET /items/:id".
    pub fn pattern(&self) -> String {
        format!("{} /{}", self.method, self.segments.join("/"))
    }
}

#[derive(Debug, Clone)]
pub struct MenuEntry {
    pub label: String,
    pub path: String,
}

/// What an app built for one request; the shell turns it into HTTP.
#[derive(Debug)]
pub struct Page {
    pub status: u16,
    pub redirect: Option<String>,
    /// HTML for <head>.
    pub head: Vec<String>,
    /// (name, html) in the order set; "page" is the whole page.
    pub html: Vec<(String, String)>,
}

/// Why a request failed.
pub struct Failed {
    /// Timeout, OverBudget, Conflict, Crashed or Denied.
    pub kind: Outcome,
    pub error: anyhow::Error,
    /// The last lines the app logged or printed (a panic message).
    pub output: Vec<String>,
}

impl Failed {
    fn new(kind: Outcome, error: anyhow::Error) -> Self {
        Failed { kind, error, output: vec![] }
    }
}

/// What a request used, for usage metering: fuel and I/O of the run that
/// counted (they are limited per run), and how many runs it took.
#[derive(Debug, Default, Clone, Copy)]
pub struct Cost {
    pub fuel: u64,
    pub io: u32,
    pub runs: u32,
}

enum Phase {
    Init {
        routes: Vec<Route>,
        menu: Vec<MenuEntry>,
    },
    /// `done` is set by page.done, which stops the app.
    Start {
        page: Page,
        done: bool,
    },
}

pub struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
    /// Set when the app asked for more than `memory_limit`.
    over_memory: bool,
    memory_limit: usize,
    over_tables: bool,
    over_host: bool,
    host_bytes: usize,
    memory: Allocations,
    tables: Allocations,
    phase: Phase,
    app_id: String,
    db: Db,
    io_used: u32,
    tx: Tx,
    /// Log lines, printed once the request is over (see `call`).
    logs: Vec<String>,
    log_bytes: usize,
    /// What the app prints to stdout and stderr, such as the message of a
    /// panic; printed with the log lines.
    output: MemoryOutputPipe,
    /// The grants this request may use, by the policy (access.rs).
    access: Access,
    /// The grant a host call needed and the policy denied; it ended the
    /// request.
    denied: Option<Grant>,
}

/// Track allocations across all memories/tables in the store. Wasmtime
/// calls the limiter for each object, including its initial allocation.
#[derive(Default)]
struct Allocations {
    used: usize,
    pending: usize,
}

impl Allocations {
    fn grow(&mut self, current: usize, desired: usize, maximum: Option<usize>, limit: usize) -> bool {
        self.pending = 0;
        let growth = desired.saturating_sub(current);
        if growth > limit.saturating_sub(self.used) || maximum.is_some_and(|m| desired > m) {
            return false;
        }
        self.used += growth;
        self.pending = growth;
        true
    }

    fn failed(&mut self) {
        self.used -= self.pending;
        self.pending = 0;
    }
}

impl WasiView for Ctx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl ResourceLimiter for Ctx {
    fn memory_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> wasmtime::Result<bool> {
        self.over_memory |= desired.saturating_sub(current) > self.memory_limit.saturating_sub(self.memory.used);
        Ok(self.memory.grow(current, desired, maximum, self.memory_limit))
    }

    fn memory_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        self.memory.failed();
        Ok(())
    }

    fn table_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> wasmtime::Result<bool> {
        self.over_tables |= desired.saturating_sub(current) > MAX_TABLE_ELEMENTS.saturating_sub(self.tables.used);
        Ok(self.tables.grow(current, desired, maximum, MAX_TABLE_ELEMENTS))
    }

    fn table_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        self.tables.failed();
        Ok(())
    }
}

pub struct LoadedApp {
    pub manifest: Manifest,
    /// The version and a 128-bit SHA-256 prefix covering every bundle file.
    pub label: String,
    /// The app's icon.svg, if it has one.
    pub icon: Option<String>,
    pub routes: Vec<Route>,
    pub menu: Vec<MenuEntry>,
    pub concurrency: Arc<tokio::sync::Semaphore>,
    /// Requests run side by side (read). One that runs again after a
    /// conflict runs alone (write), so it can't conflict again.
    turns: tokio::sync::RwLock<()>,
    pre: AppPre<Ctx>,
    db: Db,
}

/// Requests one app may run at once: one per core of this machine. More
/// wait in line (see web.rs).
fn max_concurrent() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// The engine, and a thread that ticks its epoch until it is dropped.
pub fn engine() -> anyhow::Result<Engine> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    let engine = Engine::new(&config)?;
    let weak = engine.weak();
    std::thread::spawn(move || {
        while let Some(engine) = weak.upgrade() {
            engine.increment_epoch();
            drop(engine);
            std::thread::sleep(TICK);
        }
    });
    Ok(engine)
}

/// The interfaces a component imports, without versions, each with the
/// functions it uses from them.
pub(crate) fn imports(engine: &Engine, component: &Component) -> Vec<(String, Vec<String>)> {
    component
        .component_type()
        .imports(engine)
        .filter_map(|(name, item)| match item.ty {
            ComponentItem::ComponentInstance(i) => {
                Some((name.split('@').next().unwrap_or(name).to_string(), i.exports(engine).map(|(f, _)| f.to_string()).collect()))
            }
            _ => None,
        })
        .collect()
}

/// Refuses imports the manifest doesn't grant.
fn check_imports(imports: &[(String, Vec<String>)], m: &Manifest) -> anyhow::Result<()> {
    let mut problems = Vec::new();
    for (import, functions) in imports {
        if let Some(cap) = import.strip_prefix(FRAMEWORK) {
            if cap == "kv" {
                for f in functions {
                    let grant = Grant::of_kv(f);
                    if !m.grants(grant) {
                        problems.push(format!("uses kv::{f}, which needs {grant} (add it to [capabilities] grant in app.toml)"));
                    }
                }
            } else if !ALWAYS.contains(&cap) {
                problems.push(format!("uses {cap}, which no grant allows"));
            }
        } else if !ALLOWED_WASI.iter().any(|w| import.starts_with(w)) {
            problems.push(format!("imports {import}, which apps can never use"));
        }
    }
    if problems.is_empty() { Ok(()) } else { bail!("{}", problems.join("; ")) }
}

/// Compiles an app, checks its imports, links only granted interfaces and
/// runs its `init`, within the app's time limit.
pub async fn load(engine: &Engine, db: Db, app: &Checked) -> anyhow::Result<LoadedApp> {
    // Compiling takes a while; keep it off the threads serving requests.
    let (e, bundle) = (engine.clone(), app.bundle.clone());
    let component = tokio::task::spawn_blocking(move || Component::new(&e, &bundle.wasm))
        .await?
        .map_err(anyhow::Error::from)
        .context("not a valid WebAssembly component")?;
    let manifest = app.manifest.clone();
    check_imports(&imports(engine, &component), &manifest)?;

    let mut linker: Linker<Ctx> = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    types::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    registry::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    log::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    page::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    // The import check made sure the app calls only the kv functions it's granted.
    if manifest.grants(Grant::KvRead) || manifest.grants(Grant::KvWrite) {
        kv::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    }
    let pre = AppPre::new(linker.instantiate_pre(&component)?)?;

    let budget = Budget::of(&manifest, AdminLimits::default());
    let mut store = new_store(engine, &manifest, db.clone(), Phase::Init { routes: vec![], menu: vec![] }, budget);
    let deadline = Instant::now() + manifest.time_limit();
    let init = timeout_at(deadline, async {
        let instance = pre.instantiate_async(&mut store).await?;
        instance.appos_framework_handler().call_init(&mut store).await
    })
    .await;
    match init {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(anyhow::Error::from(e).context("init failed")),
        Err(_) => bail!("init took longer than {} s", manifest.time_limit().as_secs()),
    }
    let Phase::Init { routes, menu } = &mut store.data_mut().phase else { unreachable!() };
    let (routes, menu) = (std::mem::take(routes), std::mem::take(menu));
    if routes.is_empty() {
        bail!("init registered no routes; call registry::add_route");
    }
    Ok(LoadedApp {
        manifest,
        label: app.label.clone(),
        icon: app.bundle.icon.clone(),
        routes,
        menu,
        concurrency: Arc::new(tokio::sync::Semaphore::new(max_concurrent())),
        turns: tokio::sync::RwLock::new(()),
        pre,
        db,
    })
}

fn new_store(engine: &Engine, m: &Manifest, db: Db, phase: Phase, budget: Budget) -> Store<Ctx> {
    // Writing past MAX_LOG fails, and println! panics on that.
    let output = MemoryOutputPipe::new(MAX_LOG);
    let ctx = Ctx {
        wasi: WasiCtx::builder().stdout(output.clone()).stderr(output.clone()).build(),
        table: ResourceTable::new(),
        over_memory: false,
        memory_limit: budget.memory,
        over_tables: false,
        over_host: false,
        host_bytes: 0,
        memory: Default::default(),
        tables: Default::default(),
        phase,
        app_id: m.app.id.clone(),
        db,
        io_used: 0,
        tx: Tx::default(),
        logs: vec![],
        log_bytes: 0,
        output,
        access: Access::ALL,
        denied: None,
    };
    let mut store = Store::new(engine, ctx);
    store.limiter(|c| c);
    let _ = store.set_fuel(budget.fuel);
    store.set_epoch_deadline(1);
    store.epoch_deadline_async_yield_and_update(1);
    store
}

/// Runs one request and returns the page it built, then commits its kv
/// writes. If another request changed a key this one read with kv::get, the
/// writes are dropped and the request runs again from the start, this time
/// alone. All runs together get the budget's time. Only the run that
/// counts prints its log lines.
pub async fn call(app: &LoadedApp, handler: &str, req: &Request, budget: Budget, access: Access) -> (Result<Page, Failed>, Cost) {
    let deadline = Instant::now() + budget.time;
    let timeout = || timed_out(budget);
    let mut cost = Cost::default();
    for run in 1..=MAX_RUNS {
        cost.runs = run;
        let (_side_by_side, _alone) = if run == 1 {
            match timeout_at(deadline, app.turns.read()).await {
                Ok(turn) => (Some(turn), None),
                Err(_) => return (Err(timeout()), cost),
            }
        } else {
            match timeout_at(deadline, app.turns.write()).await {
                Ok(turn) => (None, Some(turn)),
                Err(_) => return (Err(timeout()), cost),
            }
        };
        let (result, mut ctx, fuel) = run_once(app, handler, req, deadline, budget, access).await;
        cost.fuel = fuel;
        cost.io = ctx.io_used;
        let page = match result {
            Ok(page) => page,
            Err(f) => {
                let lines = print_logs(&ctx);
                let mut f = f;
                f.output = lines[lines.len().saturating_sub(10)..].to_vec();
                return (Err(f), cost);
            }
        };
        match timeout_at(deadline, ctx.database(|tx, db, id| tx.commit(db, id))).await {
            Ok(Ok(true)) => {
                print_logs(&ctx);
                return (Ok(page), cost);
            }
            Ok(Ok(false)) => {}
            Err(_) => return (Err(timeout()), cost),
            Ok(Err(_)) if Instant::now() >= deadline => return (Err(timeout()), cost),
            Ok(Err(e)) => return (Err(Failed::new(Outcome::Crashed, e.into())), cost),
        }
    }
    let error = anyhow::anyhow!("ran {MAX_RUNS} times, and each time another request changed the kv data it had read");
    (Err(Failed::new(Outcome::Conflict, error)), cost)
}

fn timed_out(budget: Budget) -> Failed {
    Failed::new(Outcome::Timeout, anyhow::anyhow!("took longer than {} s", budget.time.as_secs()))
}

/// One run in a fresh instance. page.done stops the app right there (as a
/// trap the shell expects); returning from `start` without it ends the app
/// the same way. The instance is dropped either way. Returns the fuel used.
async fn run_once(app: &LoadedApp, handler: &str, req: &Request, deadline: Instant, budget: Budget, access: Access) -> (Result<Page, Failed>, Ctx, u64) {
    let page = Page { status: 200, redirect: None, head: vec![], html: vec![] };
    let mut store = new_store(app.pre.engine(), &app.manifest, app.db.clone(), Phase::Start { page, done: false }, budget);
    store.data_mut().tx = Tx::with_deadline(deadline.into_std());
    store.data_mut().access = access;
    let ran = timeout_at(deadline, async {
        let instance = app.pre.instantiate_async(&mut store).await?;
        instance.appos_framework_handler().call_start(&mut store, handler, req).await
    })
    .await;
    let fuel = budget.fuel - store.get_fuel().unwrap_or(0);
    let mut ctx = store.into_data();
    let Phase::Start { page, done } = std::mem::replace(&mut ctx.phase, Phase::Init { routes: vec![], menu: vec![] }) else { unreachable!() };
    // Denied ends the request, whatever the app did after.
    if let Some(grant) = ctx.denied {
        return (Err(Failed::new(Outcome::Denied, anyhow::anyhow!("denied {grant}"))), ctx, fuel);
    }
    let result = match ran {
        _ if Instant::now() >= deadline => Err(timed_out(budget)),
        Ok(Ok(())) => Ok(page),
        Ok(Err(_)) if done => Ok(page),
        Ok(Err(e)) => {
            let over = ctx.over_memory || ctx.over_tables || ctx.over_host || ctx.io_used > IO_BUDGET || e.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel);
            let kind = if over { Outcome::OverBudget } else { Outcome::Crashed };
            let error = if ctx.over_memory {
                anyhow::anyhow!("over budget: asked for more than {} MB of memory", ctx.memory_limit >> 20)
            } else if ctx.over_tables {
                anyhow::anyhow!("over budget: asked for more than {MAX_TABLE_ELEMENTS} table elements")
            } else {
                e.into()
            };
            Err(Failed::new(kind, error))
        }
        Err(_) => Err(timed_out(budget)),
    };
    (result, ctx, fuel)
}

/// Prints the app's log lines and output, and returns them.
fn print_logs(ctx: &Ctx) -> Vec<String> {
    let output = ctx.output.contents();
    let output = String::from_utf8_lossy(&output);
    // The sandbox has no environment, so RUST_BACKTRACE can't be set.
    let printed = output.lines().filter(|l| !l.is_empty() && !l.starts_with("note: run with `RUST_BACKTRACE=1`"));
    let lines: Vec<String> = ctx.logs.iter().map(String::as_str).chain(printed).map(str::to_string).collect();
    // Logging is off the async workers: even an open but unread pipe must
    // not hold up a committed request or prevent other requests from running.
    crate::terminal::app_logs(&ctx.app_id, &lines);
    lines
}

// ---- host functions --------------------------------------------------------

/// A path on this site, not `//elsewhere` or `https://elsewhere`.
pub(crate) fn is_local(url: &str) -> bool {
    url.starts_with('/') && !url.starts_with("//") && !url.starts_with("/\\") && !url.chars().any(char::is_control)
}

impl Ctx {
    /// SQLite and its mutex can wait. Keep those waits off Tokio's worker
    /// threads, so the request deadline and other apps can still run.
    async fn database<T: Send + 'static>(&mut self, op: impl FnOnce(&mut Tx, &Db, &str) -> rusqlite::Result<T> + Send + 'static) -> wasmtime::Result<T> {
        let (mut tx, db, id) = (std::mem::take(&mut self.tx), self.db.clone(), self.app_id.clone());
        let (tx, result) = tokio::task::spawn_blocking(move || {
            let result = op(&mut tx, &db, &id);
            (tx, result)
        })
        .await
        .map_err(|e| wasmtime::format_err!("database worker failed: {e}"))?;
        self.tx = tx;
        result.map_err(|e| {
            self.over_host |= e.sqlite_error_code() == Some(rusqlite::ErrorCode::TooBig);
            e.into()
        })
    }

    fn host_allocation(&mut self, added: usize, removed: usize) -> wasmtime::Result<()> {
        let bytes = self.host_bytes - removed + added;
        if bytes > MAX_HOST_DATA {
            self.over_host = true;
            return Err(wasmtime::format_err!("over budget: more than {} MB of page or registry data", MAX_HOST_DATA >> 20));
        }
        self.host_bytes = bytes;
        Ok(())
    }

    fn in_init(&self, what: &str) -> wasmtime::Result<()> {
        match self.phase {
            Phase::Init { .. } => Ok(()),
            Phase::Start { .. } => Err(wasmtime::format_err!("{what} can only be called in init")),
        }
    }

    /// A host call in a request that needs `grant`: the policy must let
    /// this person use it (the app holds it, check_imports saw to that).
    fn in_request(&mut self, what: &str, grant: Grant, cost: u32) -> wasmtime::Result<()> {
        if matches!(self.phase, Phase::Init { .. }) {
            return Err(wasmtime::format_err!("{what} can only be called while handling a request"));
        }
        if !self.access.allows(grant) {
            self.denied = Some(grant);
            return Err(wasmtime::format_err!("denied {grant}: {what}"));
        }
        self.io_used += cost;
        if self.io_used > IO_BUDGET {
            return Err(wasmtime::format_err!("over budget: more than {IO_BUDGET} I/O units in one request (last call: {what})"));
        }
        Ok(())
    }

    fn log(&mut self, line: String) {
        if self.log_bytes > MAX_LOG {
            return;
        }
        // Empty messages still retain a String in the log vector.
        self.log_bytes += line.len() + size_of::<String>();
        self.logs.push(if self.log_bytes > MAX_LOG { format!("warning: more than {MAX_LOG} bytes logged; the rest is dropped") } else { line });
    }
}

fn check_key(what: &str, key: &str) -> wasmtime::Result<()> {
    if key.len() > MAX_KEY {
        return Err(wasmtime::format_err!("{what}: the key is {} bytes, more than {MAX_KEY}", key.len()));
    }
    Ok(())
}

impl types::Host for Ctx {}

fn valid_path(path: &str) -> wasmtime::Result<Vec<String>> {
    if !path.starts_with('/') {
        return Err(wasmtime::format_err!("path {path:?} must start with /"));
    }
    Ok(path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect())
}

impl registry::Host for Ctx {
    fn add_route(&mut self, method: String, path: String, handler: String) -> wasmtime::Result<()> {
        self.in_init("registry::add_route")?;
        let method = method.to_ascii_uppercase();
        if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
            return Err(wasmtime::format_err!("unsupported method {method}"));
        }
        self.host_allocation(method.len() + path.len() + handler.len() + size_of::<Route>() + path.split('/').count() * size_of::<String>(), 0)?;
        let segments = valid_path(&path)?;
        if let Phase::Init { routes, .. } = &mut self.phase {
            routes.push(Route { method, segments, handler });
        }
        Ok(())
    }

    fn add_menu(&mut self, label: String, path: String) -> wasmtime::Result<()> {
        self.in_init("registry::add_menu")?;
        self.host_allocation(label.len() + path.len() + size_of::<MenuEntry>(), 0)?;
        valid_path(&path)?;
        if let Phase::Init { menu, .. } = &mut self.phase {
            menu.push(MenuEntry { label, path });
        }
        Ok(())
    }
}

impl Ctx {
    /// The page being built, until page.done.
    fn page(&mut self, what: &str) -> wasmtime::Result<&mut Page> {
        match &mut self.phase {
            Phase::Init { .. } => Err(wasmtime::format_err!("{what} can only be called while handling a request")),
            Phase::Start { page, .. } => Ok(page),
        }
    }
}

impl page::Host for Ctx {
    fn add_header(&mut self, html: String) -> wasmtime::Result<()> {
        let page = self.page("page::add-header")?;
        if page.head.contains(&html) {
            return Ok(());
        }
        if page.head.len() >= MAX_HEAD {
            return Err(wasmtime::format_err!("more than {MAX_HEAD} add-header calls for one page"));
        }
        self.host_allocation(html.len() + size_of::<String>(), 0)?;
        self.page("page::add-header")?.head.push(html);
        Ok(())
    }

    fn set_html(&mut self, name: String, html: String) -> wasmtime::Result<()> {
        let existing = self.page("page::set-html")?.html.iter().find(|(n, _)| *n == name).map(|(_, h)| h.len());
        let overhead = if existing.is_some() { 0 } else { name.len() + size_of::<(String, String)>() };
        self.host_allocation(html.len() + overhead, existing.unwrap_or(0))?;
        let page = self.page("page::set-html")?;
        match page.html.iter_mut().find(|(n, _)| *n == name) {
            Some(existing) => existing.1 = html,
            None => page.html.push((name, html)),
        }
        Ok(())
    }

    fn set_status(&mut self, code: u16) -> wasmtime::Result<()> {
        self.page("page::set-status")?.status = code;
        Ok(())
    }

    fn redirect(&mut self, path: String) -> wasmtime::Result<()> {
        if !is_local(&path) {
            return Err(wasmtime::format_err!("page::redirect({path:?}): must be a path on this site, like req.url(\"/\")"));
        }
        let old = self.page("page::redirect")?.redirect.as_ref().map_or(0, String::len);
        self.host_allocation(path.len(), old)?;
        let page = self.page("page::redirect")?;
        page.redirect = Some(path);
        page.status = 303;
        Ok(())
    }

    /// Stops the app: the trap unwinds it, and `call` sees `done`.
    fn done(&mut self) -> wasmtime::Result<()> {
        self.page("page::done")?;
        if let Phase::Start { done, .. } = &mut self.phase {
            *done = true;
        }
        Err(wasmtime::format_err!("page::done"))
    }
}

impl log::Host for Ctx {
    fn info(&mut self, message: String) -> wasmtime::Result<()> {
        self.log(message);
        Ok(())
    }

    fn warn(&mut self, message: String) -> wasmtime::Result<()> {
        self.log(format!("warning: {message}"));
        Ok(())
    }
}

impl kv::Host for Ctx {
    async fn get(&mut self, key: String) -> wasmtime::Result<Option<String>> {
        self.in_request("kv::get", Grant::KvRead, READ)?;
        check_key("kv::get", &key)?;
        self.database(move |tx, db, id| tx.get(db, id, &key)).await
    }

    fn set(&mut self, key: String, value: String) -> wasmtime::Result<()> {
        self.in_request("kv::set", Grant::KvWrite, WRITE)?;
        check_key("kv::set", &key)?;
        if value.len() > MAX_VALUE {
            return Err(wasmtime::format_err!("kv::set({key:?}): the value is {} bytes, more than {MAX_VALUE}", value.len()));
        }
        self.tx.set(key, value);
        Ok(())
    }

    fn delete(&mut self, key: String) -> wasmtime::Result<()> {
        self.in_request("kv::delete", Grant::KvWrite, WRITE)?;
        check_key("kv::delete", &key)?;
        self.tx.delete(key);
        Ok(())
    }

    async fn scan(&mut self, prefix: String, after: Option<String>, limit: u32, reverse: bool) -> wasmtime::Result<Vec<(String, String)>> {
        self.in_request("kv::scan", Grant::KvRead, READ)?;
        check_key("kv::scan", &prefix)?;
        self.database(move |tx, db, id| tx.scan(db, id, &prefix, after.as_deref(), limit, reverse)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn a_blocked_kv_read_does_not_block_the_request_timer() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        store.data_mut().phase = Phase::Start { page: Page { status: 200, redirect: None, head: vec![], html: vec![] }, done: false };
        let db = store.data().db.clone();
        let (ready, entered) = std::sync::mpsc::channel();
        let other = std::thread::spawn(move || {
            let _guard = db.lock();
            ready.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(150));
        });
        entered.recv().unwrap();
        let result = tokio::time::timeout(Duration::from_millis(20), async { kv::Host::get(store.data_mut(), "blocked".into()).await }).await;
        other.join().unwrap();
        assert!(result.is_err(), "the host call blocked the timer until the database lock was released");
    }

    #[tokio::test]
    async fn async_database_calls_preserve_staged_writes_and_conflict_detection() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        store.data_mut().phase = Phase::Start { page: Page { status: 200, redirect: None, head: vec![], html: vec![] }, done: false };
        let ctx = store.data_mut();
        kv::Host::set(ctx, "count".into(), "one".into()).unwrap();
        assert_eq!(kv::Host::get(ctx, "count".into()).await.unwrap().as_deref(), Some("one"));
        assert_eq!(kv::Host::scan(ctx, "".into(), None, 10, false).await.unwrap(), [("count".into(), "one".into())]);
        assert!(ctx.database(|tx, db, id| tx.commit(db, id)).await.unwrap());
        ctx.tx = Tx::default();
        assert_eq!(kv::Host::get(ctx, "count".into()).await.unwrap().as_deref(), Some("one"));
        let mut other = Tx::default();
        other.set("count".into(), "two".into());
        assert!(other.commit(&ctx.db, &ctx.app_id).unwrap());
        kv::Host::set(ctx, "count".into(), "three".into()).unwrap();
        assert!(!ctx.database(|tx, db, id| tx.commit(db, id)).await.unwrap());
        assert_eq!(Tx::default().get(&ctx.db, &ctx.app_id, "count").unwrap().as_deref(), Some("two"));
    }

    fn store(engine: &Engine) -> Store<Ctx> {
        let manifest = Manifest::parse("[app]\nid = 'test'\nname = 'Test'\nversion = '1'").unwrap();
        let db = crate::tx::in_memory();
        new_store(engine, &manifest, db, Phase::Init { routes: vec![], menu: vec![] }, Budget::of(&manifest, AdminLimits::default()))
    }

    #[tokio::test]
    async fn the_memory_budget_covers_all_memories_in_a_store() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        let module = wasmtime::Module::new(&engine, "(module (memory 600) (memory 600))").unwrap();
        assert!(wasmtime::Instance::new_async(&mut store, &module, &[]).await.is_err(), "two 37.5 MiB memories exceeded the 64 MiB request budget");
        assert!(store.data().over_memory);
    }

    #[tokio::test]
    async fn an_admin_limit_replaces_the_default_budget() {
        let default = Budget::with(Duration::from_secs(5), AdminLimits::default());
        assert_eq!((default.fuel, default.memory, default.time), (FUEL, MEMORY, Duration::from_secs(5)));
        let set = AdminLimits { memory_mb: Some(16), seconds: Some(10), fuel_m: Some(250) };
        let b = Budget::with(Duration::from_secs(5), set);
        assert_eq!((b.fuel, b.memory, b.time), (250_000_000, 16 << 20, Duration::from_secs(10)));
        for bad in [AdminLimits { memory_mb: Some(8), ..set }, AdminLimits { seconds: Some(31), ..set }, AdminLimits { fuel_m: Some(100), ..set }] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
        assert!(set.check().is_ok());
        // 18.75 MiB fits the default 64 MB, not an admin's 16.
        let engine = engine().unwrap();
        let manifest = Manifest::parse("[app]\nid = 'test'\nname = 'Test'\nversion = '1'").unwrap();
        let module = wasmtime::Module::new(&engine, "(module (memory 300))").unwrap();
        let mut small = new_store(&engine, &manifest, crate::tx::in_memory(), Phase::Init { routes: vec![], menu: vec![] }, b);
        assert!(wasmtime::Instance::new_async(&mut small, &module, &[]).await.is_err());
        assert!(small.data().over_memory);
        assert!(wasmtime::Instance::new_async(&mut store(&engine), &module, &[]).await.is_ok());
    }

    #[tokio::test]
    async fn the_table_budget_covers_all_tables_in_a_store() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        let module = wasmtime::Module::new(&engine, "(module (table 60000 funcref) (table 60000 funcref))").unwrap();
        assert!(wasmtime::Instance::new_async(&mut store, &module, &[]).await.is_err());
    }

    #[test]
    fn memory_growth_only_charges_new_bytes_and_failed_growth_releases_its_reservation() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        let first = wasmtime::Memory::new(&mut store, wasmtime::MemoryType::new(256, Some(1024))).unwrap();
        first.grow(&mut store, 256).unwrap();
        assert_eq!(store.data().memory.used, 32 * 1024 * 1024);
        assert!(wasmtime::Memory::new(&mut store, wasmtime::MemoryType::new(256, None)).is_ok());
        // A real Wasmtime allocation can fail after the limiter approved it.
        // Simulate that callback, then verify the released room can be used.
        assert!(store.data_mut().memory_growing(0, 16 * 1024 * 1024, None).unwrap());
        store.data_mut().memory_grow_failed(wasmtime::format_err!("allocation failed")).unwrap();
        assert!(wasmtime::Memory::new(&mut store, wasmtime::MemoryType::new(256, None)).is_ok());
        assert_eq!(store.data().memory.used, MEMORY);
        assert!(wasmtime::Memory::new(&mut store, wasmtime::MemoryType::new(1, None)).is_err());
    }

    #[test]
    fn empty_log_lines_cannot_bypass_the_log_budget() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        for _ in 0..=MAX_LOG {
            store.data_mut().log(String::new());
        }
        assert!(store.data().logs.len() <= MAX_LOG / size_of::<String>() + 1);
    }

    #[test]
    fn page_output_cannot_accumulate_unbounded_host_memory() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        store.data_mut().phase = Phase::Start { page: Page { status: 200, redirect: None, head: vec![], html: vec![] }, done: false };
        let ctx = store.data_mut();
        page::Host::set_html(ctx, "page".into(), "x".repeat(2 * 1024 * 1024)).unwrap();
        page::Host::set_html(ctx, "page".into(), "x".repeat(1024 * 1024)).unwrap();
        page::Host::add_header(ctx, "x".repeat(2 * 1024 * 1024)).unwrap();
        assert!(page::Host::set_html(ctx, "extra".into(), "x".repeat(2 * 1024 * 1024)).is_err());
    }

    #[test]
    fn redirects_reject_browser_url_normalization_bypasses() {
        for url in ["//evil.example", "/\\evil.example", "/\t/evil.example", "/\n/evil.example", "/\r\\evil.example"] {
            assert!(!is_local(url), "accepted external redirect {url:?}");
        }
        for url in ["/", "/app/items?next=//example.com", "/app/a%20b", "/app/#section"] {
            assert!(is_local(url), "refused local redirect {url:?}");
        }
    }

    #[test]
    fn registry_metadata_cannot_accumulate_unbounded_host_memory() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        let refused = (0..17).any(|_| registry::Host::add_menu(store.data_mut(), "x".repeat(256 * 1024), "/".into()).is_err());
        assert!(refused);
    }
}
