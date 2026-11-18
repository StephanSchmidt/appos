//! The sandbox. Apps are WebAssembly components run by wasmtime. Each app's
//! linker holds only the interfaces its manifest grants, and WASI has no
//! files, no network and no environment.
//!
//! `init` runs once when the app is loaded and registers routes and menu
//! entries. `call` runs once per request, in a fresh instance, as one kv
//! transaction (tx.rs).
//!
//! A Wasm kit (wasm_kit.rs) loads here too: its imports are checked as an
//! app's, and if it exports the handler it has pages like an app, in its
//! own WASI (its directory, the network, its configuration).
//!
//! A request has a budget: fuel (instructions), memory, I/O units and time.
//! Fuel stops a loop, but not an app waiting in a host call, such as a
//! sleep; so apps run async, and the time limit drops the request wherever
//! it is. CPU-bound code yields every tick of the epoch so the limit can
//! strike there too.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use tokio::time::{Instant, timeout_at};
use wasmtime::component::{Component, HasSelf, InstancePre, Linker, ResourceTable, Val, types::ComponentItem};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::access::Access;
use crate::bundle::Checked;
use crate::kit::{self, About, Context, Kits, Use, User};
use crate::manifest::Manifest;
use crate::platform::Outcome;
use crate::state::Live;
use crate::tx::{Db, Tx};

wasmtime::component::bindgen!({
    world: "app",
    path: "../wit",
    // kv is a kit, linked by kit.rs.
    imports: { default: trappable },
    exports: { default: async },
});

use appos::framework::{log, page, registry, types};
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
/// A kit's http.fetch costs as much as a write: 20 in one request.
pub const FETCH: u32 = 5;
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
/// Calls deep a chain may go: an app calls a Wasm kit (1), which calls
/// another (2), and so on. It bounds the memory of one request.
pub const MAX_DEPTH: u32 = 4;

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
    /// The Wasm kit that trapped, if one did: its id and label. The
    /// failure counts toward its automatic rollback, not the app's.
    pub kit: Option<(String, String)>,
}

impl Failed {
    fn new(kind: Outcome, error: anyhow::Error) -> Self {
        Failed { kind, error, output: vec![], kit: None }
    }
}

/// What a request used, for usage metering: fuel and I/O of the run that
/// counted (they are limited per run), and how many runs it took.
#[derive(Debug, Default, Clone, Copy)]
pub struct Cost {
    pub fuel: u64,
    pub io: u32,
    pub runs: u32,
    /// Commands it sent to kits, once committed.
    pub sent: usize,
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
    denied: Option<String>,
    /// What kits get with a call: set for a request.
    context: Option<Context>,
    /// How deep in a chain of Wasm kit calls this store runs: 0 for an
    /// app, 1 for the kit it calls.
    depth: u32,
    /// A Wasm kit this store called went over its budget, which is this
    /// request's.
    over_kit: bool,
    /// The Wasm kit that trapped in a call from this store: its id and
    /// label.
    kit_failed: Option<(String, String)>,
    /// The hosts a Wasm kit may fetch from; none for an app.
    hosts: Arc<[String]>,
    /// A Wasm kit in a call its caller waits for: it only reads.
    waits: bool,
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
    /// None for a kit without pages.
    pre: Option<AppPre<Ctx>>,
    db: Db,
    /// The kits' functions it imports: what their new versions must keep.
    pub(crate) uses: HashMap<String, Use>,
    /// What it offers, if it is a Wasm kit.
    pub(crate) kit: Option<Arc<crate::wasm_kit::KitVersion>>,
}

impl LoadedApp {
    /// Whether it is a Wasm kit.
    pub fn is_kit(&self) -> bool {
        self.kit.is_some()
    }
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
/// functions it uses from them (not its types: a record is no call).
pub(crate) fn imports(engine: &Engine, component: &Component) -> Vec<(String, Vec<String>)> {
    component
        .component_type()
        .imports(engine)
        .filter_map(|(name, item)| match item.ty {
            ComponentItem::ComponentInstance(i) => {
                let functions = i.exports(engine).filter(|(_, ty)| matches!(ty.ty, ComponentItem::ComponentFunc(_))).map(|(f, _)| f.to_string()).collect();
                Some((name.split('@').next().unwrap_or(name).to_string(), functions))
            }
            _ => None,
        })
        .collect()
}

/// The functions a component imports, with their types, by
/// "acme:crm/leads#count-leads": a Wasm kit checks them (wasm_kit.rs).
fn import_types(engine: &Engine, component: &Component) -> HashMap<String, wasmtime::component::types::ComponentFunc> {
    let mut uses = HashMap::new();
    for (name, item) in component.component_type().imports(engine) {
        if let ComponentItem::ComponentInstance(i) = item.ty {
            for (f, item) in i.exports(engine) {
                if let ComponentItem::ComponentFunc(func) = item.ty {
                    uses.insert(format!("{}#{f}", name.split('@').next().unwrap_or(name)), func);
                }
            }
        }
    }
    uses
}

/// Refuses grants no kit offers, and imports the manifest doesn't grant.
/// A kit may import all of WASI.
fn check_imports(imports: &[(String, Vec<String>)], m: &Manifest, kits: &Kits) -> anyhow::Result<()> {
    let unknown: Vec<&String> = m.capabilities.grant.iter().filter(|g| kits.grant(g).is_none()).collect();
    if let Some(g) = unknown.first() {
        let kit = if m.is_kit() { "" } else { "; a Wasm kit's grants come once it is deployed" };
        bail!("unknown capability {g:?}; known: {}{kit}", kits.for_apps().join(", "));
    }
    let (file, who) = (m.file(), if m.is_kit() { "kits" } else { "apps" });
    let mut problems = Vec::new();
    for (import, functions) in imports {
        let short = import.rsplit('/').next().unwrap_or(import);
        // An interface of a kit's WIT may hold types alone, which other
        // interfaces use.
        if kits.offers(import) || kits.knows(import) {
            for f in functions {
                match kits.grant_for(import, f) {
                    Some(g) if m.grants(&g.name) => {}
                    Some(g) => problems.push(format!("uses {short}::{f}, which needs {} (add it to [capabilities] grant in {file})", g.name)),
                    None => problems.push(format!("uses {short}::{f}, which no grant allows")),
                }
            }
        } else if let Some(cap) = import.strip_prefix(FRAMEWORK) {
            if cap == "http" && !m.is_kit() {
                problems.push("uses http, which only kits can: an app calls a kit that fetches".to_string());
            } else if !ALWAYS.contains(&cap) && cap != "http" {
                problems.push(format!("uses {cap}, which no grant allows"));
            }
        } else if !ALLOWED_WASI.iter().any(|w| import.starts_with(w)) && !(m.is_kit() && import.starts_with("wasi:")) {
            problems.push(format!("imports {import}, which {who} can never use"));
        }
    }
    if problems.is_empty() { Ok(()) } else { bail!("{}", problems.join("; ")) }
}

/// Compiles an app, checks its imports, links only granted functions and
/// runs its `init`, within the app's time limit. A Wasm kit gets its
/// directory below `data` (data/kits/<id>).
pub async fn load(engine: &Engine, db: Db, kits: &Kits, app: &Checked, data: &Path) -> anyhow::Result<LoadedApp> {
    // Compiling takes a while; keep it off the threads serving requests.
    let (e, bundle) = (engine.clone(), app.bundle.clone());
    let component = tokio::task::spawn_blocking(move || Component::new(&e, &bundle.wasm))
        .await?
        .map_err(anyhow::Error::from)
        .context("not a valid WebAssembly component")?;
    let manifest = app.manifest.clone();
    let id = &manifest.app.id;
    let what = if manifest.is_kit() { "kit" } else { "app" };
    // A kit's new version links to the other kits, not to its old one.
    let kits = if manifest.is_kit() { kits.without(id) } else { kits.clone() };
    anyhow::ensure!(!kits.page_names().contains(id.as_str()), "{what} id {id:?}: the platform's own pages are at /{id}");
    anyhow::ensure!(!kits.taken().contains(id.as_str()), "{what} id {id:?} is a kit's name");
    check_imports(&imports(engine, &component), &manifest, &kits)?;

    let mut linker: Linker<Ctx> = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    types::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    registry::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    log::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    page::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    if manifest.is_kit() {
        crate::fetch::http::add_to_linker::<Ctx, HasSelf<Ctx>>(&mut linker, |c| c)?;
    }
    // The import check made sure the app calls only the functions it's granted.
    let types = import_types(engine, &component);
    let linked = kits.link(&mut linker, &manifest.capabilities.grant, &types)?;
    let uses = linked.into_iter().filter_map(|l| Some((l.name.clone(), Use { ty: types.get(&l.name)?.clone(), grant: l.grant, command: l.command }))).collect();
    let instance = linker.instantiate_pre(&component)?;
    let kit = match &manifest.kit {
        Some(offers) => {
            let exports = crate::wasm_kit::check_exports(engine, &component, offers)?;
            let version = crate::wasm_kit::version(&manifest, offers, &app.bundle, &app.label, exports, instance.clone(), db.clone(), data)?;
            Some(Arc::new(version))
        }
        None => None,
    };
    // An app has pages; a kit has them if it exports the handler.
    let handler = component.component_type().exports(engine).any(|(name, _)| name.starts_with("appos:framework/handler"));
    let pre = if kit.is_none() || handler { Some(AppPre::new(instance)?) } else { None };

    let (mut routes, mut menu) = (vec![], vec![]);
    if let Some(pre) = &pre {
        let budget = Budget::of(&manifest, AdminLimits::default());
        let phase = Phase::Init { routes: vec![], menu: vec![] };
        let mut store = new_store(engine, &manifest.app.id, db.clone(), phase, budget, kit.as_deref().map(|k| &k.sandbox));
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
        let Phase::Init { routes: r, menu: m } = &mut store.data_mut().phase else { unreachable!() };
        (routes, menu) = (std::mem::take(r), std::mem::take(m));
        if routes.is_empty() && kit.is_none() {
            bail!("init registered no routes; call registry::add_route");
        }
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
        uses,
        kit,
    })
}

/// A Wasm kit's WASI: its directory ("/") and its configuration. Its
/// directory was made when it loaded; should it be gone, the kit finds no
/// "/". No sockets and no name lookup, as for an app: a kit reaches out only
/// through http::fetch, to its [http] hosts.
fn kit_wasi(wasi: &mut wasmtime_wasi::WasiCtxBuilder, dir: &std::path::Path, env: &[(String, String)]) {
    let _ = wasi.preopened_dir(dir, "/", wasmtime_wasi::FsPerms::ReadWrite);
    for (key, value) in env {
        wasi.env(key, value);
    }
}

/// A store for `id`. An app's WASI has nothing but the output (no files,
/// no network, no environment); a Wasm kit's has `kit_wasi`, and it may
/// fetch from its hosts.
fn new_store(engine: &Engine, id: &str, db: Db, phase: Phase, budget: Budget, kit: Option<&KitSandbox>) -> Store<Ctx> {
    // Writing past MAX_LOG fails, and println! panics on that.
    let output = MemoryOutputPipe::new(MAX_LOG);
    let mut builder = WasiCtx::builder();
    builder.stdout(output.clone()).stderr(output.clone());
    if let Some(kit) = kit {
        kit_wasi(&mut builder, &kit.dir, &kit.env);
    }
    let ctx = Ctx {
        wasi: builder.build(),
        table: ResourceTable::new(),
        over_memory: false,
        memory_limit: budget.memory,
        over_tables: false,
        over_host: false,
        host_bytes: 0,
        memory: Default::default(),
        tables: Default::default(),
        phase,
        app_id: id.to_string(),
        db,
        io_used: 0,
        tx: Tx::default(),
        logs: vec![],
        log_bytes: 0,
        output,
        access: Access::ALL,
        denied: None,
        context: None,
        depth: 0,
        over_kit: false,
        kit_failed: None,
        hosts: kit.map(|k| k.hosts.clone()).unwrap_or_default(),
        waits: false,
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
/// `live` tells kits which apps serve.
pub async fn call(
    app: &LoadedApp,
    handler: &str,
    req: &Request,
    budget: Budget,
    access: Access,
    user: Option<User>,
    live: Live,
) -> (Result<Page, Failed>, Cost) {
    let deadline = Instant::now() + budget.time;
    let Some(pre) = &app.pre else {
        let error = anyhow::anyhow!("{} has no pages", app.manifest.app.id);
        return (Err(Failed::new(Outcome::Crashed, error)), Cost::default());
    };
    let held = &app.manifest.capabilities.grant;
    let cx = Context::new(About {
        app: app.manifest.app.id.clone(),
        user,
        page: Some(kit::Page { method: req.method.clone(), path: req.path.clone(), url: format!("{}{}", req.origin, req.base) }),
        deadline: deadline.into_std(),
        may: held.iter().enumerate().filter(|(bit, _)| access.allows(*bit as u32)).map(|(_, g)| g.clone()).collect(),
        live,
        own: None,
    });
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
        let (result, mut ctx, fuel) = run_once(app, pre, handler, req, deadline, budget, access, &cx).await;
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
                cost.sent = ctx.tx.sent();
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
#[allow(clippy::too_many_arguments)]
async fn run_once(
    app: &LoadedApp,
    pre: &AppPre<Ctx>,
    handler: &str,
    req: &Request,
    deadline: Instant,
    budget: Budget,
    access: Access,
    cx: &Context,
) -> (Result<Page, Failed>, Ctx, u64) {
    let page = Page { status: 200, redirect: None, head: vec![], html: vec![] };
    let phase = Phase::Start { page, done: false };
    let mut store = new_store(pre.engine(), &app.manifest.app.id, app.db.clone(), phase, budget, app.kit.as_deref().map(|k| &k.sandbox));
    store.data_mut().tx = Tx::with_deadline(deadline.into_std());
    store.data_mut().access = access;
    store.data_mut().context = Some(cx.clone());
    let ran = timeout_at(deadline, async {
        let instance = pre.instantiate_async(&mut store).await?;
        instance.appos_framework_handler().call_start(&mut store, handler, req).await
    })
    .await;
    let fuel = budget.fuel - store.get_fuel().unwrap_or(0);
    let mut ctx = store.into_data();
    let Phase::Start { page, done } = std::mem::replace(&mut ctx.phase, Phase::Init { routes: vec![], menu: vec![] }) else { unreachable!() };
    // Denied ends the request, whatever the app did after.
    if let Some(grant) = ctx.denied.take() {
        return (Err(Failed::new(Outcome::Denied, anyhow::anyhow!("denied {grant}"))), ctx, fuel);
    }
    let result = match ran {
        _ if Instant::now() >= deadline => Err(timed_out(budget)),
        Ok(Ok(())) => Ok(page),
        Ok(Err(_)) if done => Ok(page),
        Ok(Err(e)) => {
            let over = ctx.over_memory
                || ctx.over_tables
                || ctx.over_host
                || ctx.over_kit
                || ctx.io_used > IO_BUDGET
                || e.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel);
            let kind = if over { Outcome::OverBudget } else { Outcome::Crashed };
            let error = if ctx.over_memory {
                anyhow::anyhow!("over budget: asked for more than {} MB of memory", ctx.memory_limit >> 20)
            } else if ctx.over_tables {
                anyhow::anyhow!("over budget: asked for more than {MAX_TABLE_ELEMENTS} table elements")
            } else {
                e.into()
            };
            let kit = if kind == Outcome::Crashed { ctx.kit_failed.take() } else { None };
            Err(Failed { kit, ..Failed::new(kind, error) })
        }
        Err(_) => Err(timed_out(budget)),
    };
    (result, ctx, fuel)
}

/// A Wasm kit's sandbox (wasm_kit.rs): its component, linked to what it
/// may call, and what it gets besides.
pub(crate) struct KitSandbox {
    pub pre: InstancePre<Ctx>,
    /// Its name: where policy.csv gives it grants, its log lines' name.
    pub name: String,
    /// Its version's label, to blame it for a trap.
    pub label: String,
    /// The grants it holds, in kit.toml's order: a grant's place is its bit.
    pub held: Vec<String>,
    /// Commands it sends go into the outbox here.
    pub db: Db,
    /// Its own directory, "/" in its WASI.
    pub dir: std::path::PathBuf,
    /// Environment variables: its [config], and [config-env] read.
    pub env: Vec<(String, String)>,
    /// The hosts it may fetch from (kit.toml's [http]).
    pub hosts: Arc<[String]>,
}

/// What a call into a Wasm kit may use: what its caller has left. The kit
/// runs within its caller's deadline (the context's) and gets back what
/// it didn't use.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Left {
    pub fuel: u64,
    /// I/O units used so far: the caller's and the kit's count together.
    pub io: u32,
    /// The kit's depth in the chain: 1 for a call from an app.
    pub depth: u32,
    /// Its caller waits for it, so it only reads (fetch: GET only); a
    /// command doesn't.
    pub waits: bool,
}

/// How a call into a Wasm kit went, for its caller.
pub(crate) struct Called {
    pub result: wasmtime::Result<()>,
    pub left: Left,
    /// A grant the policy denied the kit: it ends the caller's request too.
    pub denied: Option<String>,
    /// It went over the budget, the caller's.
    pub over: bool,
    /// The Wasm kit that trapped, this one or one it called: its id and
    /// label.
    pub blame: Option<(String, String)>,
}

/// Calls `export`, an interface and one of its functions, of a Wasm kit, in a fresh instance with
/// its own memory limit, for `cx` (the caller's context, as the kit sees
/// it: kit.rs `Context::to_kit`). Commands the kit sent go out once the
/// call returns. A command's run is such a call too, with the whole budget.
pub(crate) async fn call_kit(
    kit: &KitSandbox,
    cx: Context,
    access: Access,
    left: Left,
    (interface, function): (&str, &str),
    params: &[Val],
    results: &mut [Val],
) -> Called {
    let deadline = Instant::from_std(cx.deadline());
    let budget = Budget { fuel: left.fuel, memory: MEMORY, time: cx.time_left() };
    let page = Page { status: 200, redirect: None, head: vec![], html: vec![] };
    let mut store = new_store(kit.pre.engine(), &kit.name, kit.db.clone(), Phase::Start { page, done: false }, budget, Some(kit));
    let ctx = store.data_mut();
    ctx.tx = Tx::with_deadline(cx.deadline());
    ctx.access = access;
    ctx.context = Some(cx);
    ctx.io_used = left.io;
    ctx.depth = left.depth;
    ctx.waits = left.waits;
    let what = format!("{}::{function}", interface.split('@').next().unwrap_or(interface).rsplit('/').next().unwrap_or(interface));
    let ran = timeout_at(deadline, async {
        let instance = kit.pre.instantiate_async(&mut store).await?;
        let found = instance.get_export_index(&mut store, None, interface).and_then(|i| instance.get_export_index(&mut store, Some(&i), function));
        let func = found.and_then(|f| instance.get_func(&mut store, f)).ok_or_else(|| wasmtime::format_err!("the kit exports no {what}"))?;
        func.call_async(&mut store, params, results).await
    })
    .await;
    let fuel = store.get_fuel().unwrap_or(0);
    let mut ctx = store.into_data();
    let over = ctx.over_memory || ctx.over_tables || ctx.over_host || ctx.over_kit;
    let left = Left { fuel, io: ctx.io_used, depth: left.depth, waits: left.waits };
    let denied = ctx.denied.take();
    // A trap in the kit's own code is the kit's; an error a host function
    // gave it (a kit it called, the policy) is not.
    let trapped = matches!(&ran, Ok(Err(e)) if e.downcast_ref::<Trap>().is_some_and(|t| *t != Trap::OutOfFuel && *t != Trap::Interrupt));
    let blame = ctx.kit_failed.take().or_else(|| (trapped && !over && denied.is_none()).then(|| (kit.name.clone(), kit.label.clone())));
    let result = match ran {
        Err(_) => Err(wasmtime::format_err!("{what}: the request's time ran out in {}", kit.name)),
        Ok(Err(e)) if e.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel) => Err(e),
        Ok(Err(e)) => Err(e.context(format!("{what} failed in {}", kit.name))),
        Ok(Ok(())) => match timeout_at(deadline, ctx.database(|tx, db, id| tx.commit(db, id))).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e.context(format!("{what}: {}'s commands could not be sent", kit.name))),
            Err(_) => Err(wasmtime::format_err!("{what}: the request's time ran out in {}", kit.name)),
        },
    };
    if result.is_err() {
        print_logs(&ctx);
    }
    let blame = if result.is_err() { blame } else { None };
    Called { result, left, denied, over, blame }
}

/// A Wasm kit's call from this store: how deep it runs, what the caller
/// has left, and its context.
impl Ctx {
    pub(crate) fn kit_call(&self) -> wasmtime::Result<(Context, Left)> {
        let depth = self.depth + 1;
        if depth > MAX_DEPTH {
            return Err(wasmtime::format_err!("a chain of kit calls deeper than {MAX_DEPTH}"));
        }
        Ok((self.context()?, Left { fuel: 0, io: self.io_used, depth, waits: true }))
    }

    /// After a Wasm kit's call: what it used is this request's, and so is a
    /// grant the policy denied it.
    pub(crate) fn kit_called(&mut self, called: &Called) {
        self.io_used = called.left.io;
        self.over_kit |= called.over || self.io_used > IO_BUDGET;
        if self.denied.is_none() {
            self.denied.clone_from(&called.denied);
        }
        if self.kit_failed.is_none() {
            self.kit_failed.clone_from(&called.blame);
        }
    }
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

    /// A kit call in a request that needs `grant`, the app's grant number
    /// `bit`: the policy must let this person use it (the app holds it,
    /// check_imports saw to that).
    pub(crate) fn in_request(&mut self, what: &str, grant: &str, bit: u32, cost: u32) -> wasmtime::Result<()> {
        if matches!(self.phase, Phase::Init { .. }) {
            return Err(wasmtime::format_err!("{what} can only be called while handling a request"));
        }
        if !self.access.allows(bit) {
            self.denied = Some(grant.to_string());
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

/// A Wasm kit's fetch (fetch.rs): to its hosts, in a request, from the
/// request's I/O and time; GET only in a call its caller waits for.
impl crate::fetch::http::Host for Ctx {
    async fn fetch(&mut self, req: crate::fetch::http::Request) -> wasmtime::Result<Result<crate::fetch::http::Response, String>> {
        if matches!(self.phase, Phase::Init { .. }) {
            return Err(wasmtime::format_err!("http::fetch can only be called while handling a request"));
        }
        if self.waits && !req.method.eq_ignore_ascii_case("GET") {
            let e = format!("{} {}: a call an app waits for only reads, so GET only; send a command to change things", req.method, req.url);
            self.log(format!("warning: fetch refused: {e}"));
            return Ok(Err(e));
        }
        if let Err(e) = crate::fetch::allowed(&req.url, &self.hosts) {
            self.log(format!("warning: fetch refused: {e}"));
            return Ok(Err(e));
        }
        self.io_used += FETCH;
        if self.io_used > IO_BUDGET {
            return Err(wasmtime::format_err!("over budget: more than {IO_BUDGET} I/O units in one request (last call: http::fetch)"));
        }
        let left = self.context()?.time_left();
        crate::fetch::send(req, left).await
    }
}

/// kv's functions, for the kv kit (kit.rs), which checked the call.
impl Ctx {
    pub(crate) async fn kv_get(&mut self, key: String) -> wasmtime::Result<Option<String>> {
        check_key("kv::get", &key)?;
        self.database(move |tx, db, id| tx.get(db, id, &key)).await
    }

    /// What kits get with a call; only while handling a request.
    pub(crate) fn context(&self) -> wasmtime::Result<Context> {
        self.context.clone().ok_or_else(|| wasmtime::format_err!("kits can only be called while handling a request"))
    }

    /// A command to a kit (kit.rs), sent when the request commits, with
    /// the user as they are now.
    pub(crate) fn send(&mut self, interface: String, function: String, args: String) -> wasmtime::Result<()> {
        let about = serde_json::to_string(&self.context()?.sent())?;
        self.tx.send(crate::tx::Outgoing { id: crate::auth::session::random(16), interface, function, args, about });
        Ok(())
    }

    pub(crate) fn kv_set(&mut self, key: String, value: String) -> wasmtime::Result<()> {
        check_key("kv::set", &key)?;
        if value.len() > MAX_VALUE {
            return Err(wasmtime::format_err!("kv::set({key:?}): the value is {} bytes, more than {MAX_VALUE}", value.len()));
        }
        self.tx.set(key, value);
        Ok(())
    }

    pub(crate) fn kv_delete(&mut self, key: String) -> wasmtime::Result<()> {
        check_key("kv::delete", &key)?;
        self.tx.delete(key);
        Ok(())
    }

    pub(crate) async fn kv_scan(&mut self, prefix: String, after: Option<String>, limit: u32, reverse: bool) -> wasmtime::Result<Vec<(String, String)>> {
        check_key("kv::scan", &prefix)?;
        self.database(move |tx, db, id| tx.scan(db, id, &prefix, after.as_deref(), limit, reverse)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kit_calls_go_at_most_max_depth_deep() {
        let budget = Budget { fuel: FUEL, memory: MEMORY, time: Duration::from_secs(1) };
        let mut store = new_store(&engine().unwrap(), "test", crate::tx::in_memory(), Phase::Init { routes: vec![], menu: vec![] }, budget, None);
        let ctx = store.data_mut();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        ctx.context = Some(Context::new(About { app: "test".into(), user: None, page: None, deadline, may: vec![], live: Live::default(), own: None }));
        ctx.depth = MAX_DEPTH - 1;
        assert_eq!(ctx.kit_call().unwrap().1.depth, MAX_DEPTH);
        ctx.depth = MAX_DEPTH;
        assert_eq!(ctx.kit_call().err().unwrap().to_string(), "a chain of kit calls deeper than 4");
    }

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
        let result = tokio::time::timeout(Duration::from_millis(20), async { store.data_mut().kv_get("blocked".into()).await }).await;
        other.join().unwrap();
        assert!(result.is_err(), "the host call blocked the timer until the database lock was released");
    }

    #[tokio::test]
    async fn async_database_calls_preserve_staged_writes_and_conflict_detection() {
        let engine = engine().unwrap();
        let mut store = store(&engine);
        store.data_mut().phase = Phase::Start { page: Page { status: 200, redirect: None, head: vec![], html: vec![] }, done: false };
        let ctx = store.data_mut();
        ctx.kv_set("count".into(), "one".into()).unwrap();
        assert_eq!(ctx.kv_get("count".into()).await.unwrap().as_deref(), Some("one"));
        assert_eq!(ctx.kv_scan("".into(), None, 10, false).await.unwrap(), [("count".into(), "one".into())]);
        assert!(ctx.database(|tx, db, id| tx.commit(db, id)).await.unwrap());
        ctx.tx = Tx::default();
        assert_eq!(ctx.kv_get("count".into()).await.unwrap().as_deref(), Some("one"));
        let mut other = Tx::default();
        other.set("count".into(), "two".into());
        assert!(other.commit(&ctx.db, &ctx.app_id).unwrap());
        ctx.kv_set("count".into(), "three".into()).unwrap();
        assert!(!ctx.database(|tx, db, id| tx.commit(db, id)).await.unwrap());
        assert_eq!(Tx::default().get(&ctx.db, &ctx.app_id, "count").unwrap().as_deref(), Some("two"));
    }

    fn store(engine: &Engine) -> Store<Ctx> {
        let manifest = Manifest::parse("[app]\nid = 'test'\nname = 'Test'\nversion = '1'").unwrap();
        let db = crate::tx::in_memory();
        new_store(engine, &manifest.app.id, db, Phase::Init { routes: vec![], menu: vec![] }, Budget::of(&manifest, AdminLimits::default()), None)
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
        let mut small = new_store(&engine, &manifest.app.id, crate::tx::in_memory(), Phase::Init { routes: vec![], menu: vec![] }, b, None);
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

    /// A service on a free port that answers each request with its method.
    async fn service() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let app = axum::Router::new().fallback(|method: axum::http::Method| async move { method.to_string() });
        tokio::spawn(async move { axum::serve(listener, app).await });
        url
    }

    /// A kit's WASI has no TCP, no UDP and no name lookup: those would get
    /// around [http] hosts (a lookup carries data in the name it asks for).
    #[tokio::test]
    async fn a_kit_has_no_sockets() {
        use wasmtime_wasi::p2::bindings::sockets::instance_network::Host as _;
        use wasmtime_wasi::p2::bindings::sockets::ip_name_lookup::Host as _;
        use wasmtime_wasi::p2::bindings::sockets::network::{ErrorCode, IpAddressFamily};
        use wasmtime_wasi::p2::bindings::sockets::tcp_create_socket::Host as _;
        use wasmtime_wasi::p2::bindings::sockets::udp_create_socket::Host as _;
        use wasmtime_wasi::sockets::WasiSocketsView as _;
        struct Kit(WasiCtx, ResourceTable);
        impl WasiView for Kit {
            fn ctx(&mut self) -> WasiCtxView<'_> {
                WasiCtxView { ctx: &mut self.0, table: &mut self.1 }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut builder = WasiCtx::builder();
        kit_wasi(&mut builder, dir.path(), &[]);
        let mut kit = Kit(builder.build(), ResourceTable::new());
        let mut s = kit.sockets();
        let tcp = s.create_tcp_socket(IpAddressFamily::Ipv4).map(|_| ()).map_err(|e| e.downcast().unwrap());
        let udp = s.create_udp_socket(IpAddressFamily::Ipv4).await.map(|_| ()).map_err(|e| e.downcast().unwrap());
        let network = s.instance_network().unwrap();
        let lookup = s.resolve_addresses(network, "secret.example.com".into()).map(|_| ()).map_err(|e| e.downcast().unwrap());
        assert!(matches!((&tcp, &udp), (Err(ErrorCode::AccessDenied), Err(ErrorCode::AccessDenied))), "{tcp:?} {udp:?}");
        assert!(lookup.is_err(), "a lookup goes out");
    }

    #[tokio::test]
    async fn a_kit_fetches_from_its_hosts_within_the_request() {
        use crate::fetch::http::{Host as _, Request as Fetch};
        let url = format!("{}/leads", service().await);
        let fetch = |method: &str, url: &str| Fetch { method: method.into(), url: url.into(), headers: vec![], body: String::new() };
        let mut store = store(&engine().unwrap());
        let ctx = store.data_mut();
        ctx.hosts = vec!["127.0.0.1".to_string()].into();
        assert!(ctx.fetch(fetch("GET", &url)).await.unwrap_err().to_string().contains("only be called while handling a request"));
        ctx.phase = Phase::Start { page: Page { status: 200, redirect: None, head: vec![], html: vec![] }, done: false };
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        ctx.context = Some(Context::new(About { app: "test".into(), user: None, page: None, deadline, may: vec![], live: Live::default(), own: None }));

        let answer = ctx.fetch(fetch("GET", &url)).await.unwrap().unwrap();
        assert_eq!((answer.status, answer.body.as_str(), ctx.io_used), (200, "GET", FETCH));
        let refused = ctx.fetch(fetch("GET", "https://elsewhere.example/")).await.unwrap().unwrap_err();
        assert!(refused.contains("not in kit.toml's [http] hosts (127.0.0.1)"), "{refused}");
        assert_eq!(ctx.io_used, FETCH, "a refused fetch costs nothing");

        // A call its caller waits for only reads; a command or a page may change things.
        ctx.waits = true;
        assert!(ctx.fetch(fetch("POST", &url)).await.unwrap().unwrap_err().contains("GET only"));
        assert_eq!(ctx.fetch(fetch("get", &url)).await.unwrap().unwrap().body, "GET");
        ctx.waits = false;
        assert_eq!(ctx.fetch(fetch("POST", &url)).await.unwrap().unwrap().body, "POST");

        ctx.io_used = IO_BUDGET;
        assert!(ctx.fetch(fetch("GET", &url)).await.unwrap_err().to_string().contains("over budget"));
    }
}
