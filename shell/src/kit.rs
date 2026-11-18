//! Kits: what apps call besides the page. A kit is WIT interfaces and the
//! code behind them; kv and users are the shell's own. A platform adds its
//! own:
//!
//! ```ignore
//! appos_shell::Platform::new()
//!     .kit(Crm::new(crm_db))
//!     .serve(opts)
//!     .await
//! ```
//!
//! A kit names its grants and the functions each allows, and links those
//! functions through `KitLinker`. AppOS checks every call, the app's grant
//! and the policy, and charges the request's I/O budget, before the kit's
//! code runs: kits never check permissions themselves, and they never see
//! the linker they could get around it with.
//!
//! A function answers now (`KitLinker::func`) or is a command
//! (`KitLinker::command`). A function may write the kit's own data, but
//! the request can still fail or run again after the call, so a write
//! there must be harmless to repeat: set a draft's field, don't add a
//! line. A command returns nothing and runs once the request commits: the
//! commit puts it in the outbox, and `Kit::run` gets it from there
//! (outbox.rs). An interface is one kit's, a package may be split across
//! kits: kv and users are two kits of appos:framework, so kv can be
//! swapped as a whole.
//!
//! Kits have pages too, like apps (`Kit::routes`): below names of their
//! own, in the shell's layout. Each names the grant the person needs, and
//! AppOS checks the policy before the page's code runs.
//!
//! Every function, page and `run` gets a `Context`, like Go's
//! context.Context: the app, the signed-in user, the page and the
//! deadline. AppOS builds it, never the caller, so a kit can trust it.
//!
//! A kit is Rust compiled into the platform, or Wasm (wasm_kit.rs), held
//! like an app: deployed, rolled back, turned off and on. A Wasm kit may
//! call other kits, as an app does, with grants of its own
//! (`Kit::caller`); AppOS refuses a cycle. The kits now are `LiveKits`:
//! the platform's own, fixed, and the Wasm kits' live versions. Every
//! Rust kit can bring a fake (`Kit::fake`): a Wasm kit with the same
//! functions and made-up data, which `appos dev` runs where the real one
//! can't.
//!
//! What uses a kit serves only while the kit serves (`Kit::up`, and a
//! Wasm kit loaded and on): else it answers 503 and its commands wait.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wasmtime::StoreContextMut;
use wasmtime::component::types::ComponentFunc;
use wasmtime::component::{Linker, Val};

use crate::access::Access;
use crate::runtime::{Ctx, FRAMEWORK, MAX_VALUE, READ, WRITE};
use crate::state::Live;
use crate::wasm_kit::{KitVersion, Slot, WasmKit};

/// What kit functions take and return: tuples of these, and records that
/// derive them.
pub use wasmtime::component::{ComponentNamedList, ComponentType, Lift, Lower};
pub use wasmtime::{Error, Result};

/// What a kit function returns.
pub type Answer<'a, R> = Pin<Box<dyn Future<Output = Result<R>> + Send + 'a>>;
/// What running a command returns.
pub type Run<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// The grants of the shell's own kit: kv and the user.
pub const KV_READ: &str = "kv-read";
pub const KV_WRITE: &str = "kv-write";
pub const USER_READ: &str = "user-read";

/// What AppOS tells a kit about a call, like Go's context.Context. AppOS
/// builds it, never the caller. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Context(Arc<About>);

#[derive(Debug)]
pub(crate) struct About {
    pub app: String,
    pub user: Option<User>,
    pub page: Option<Page>,
    pub deadline: Instant,
    /// The app's grants the policy lets this user use, at the call (or
    /// for a command, at the commit).
    pub may: Vec<String>,
    pub live: Live,
}

impl Context {
    pub(crate) fn new(about: About) -> Context {
        Context(Arc::new(about))
    }

    /// What a command keeps of it.
    pub(crate) fn sent(&self) -> Sent {
        Sent { user: self.0.user.clone(), may: self.0.may.clone() }
    }

    /// The app that called, or sent the command; for a kit's page, the
    /// name it is at ("booking" for /booking/...).
    pub fn app(&self) -> &str {
        &self.0.app
    }

    /// Who is signed in; none for nobody, and for jobs. For a command, who
    /// was signed in when it was sent.
    pub fn user(&self) -> Option<&User> {
        self.0.user.as_ref()
    }

    /// The page request the call came from; none for a command.
    pub fn page(&self) -> Option<&Page> {
        self.0.page.as_ref()
    }

    /// When the call has to be done: the request's deadline, or a
    /// command's run time. AppOS drops the kit's future then; pass it on,
    /// as a timeout of the kit's own calls.
    pub fn deadline(&self) -> Instant {
        self.0.deadline
    }

    pub fn time_left(&self) -> Duration {
        self.0.deadline.saturating_duration_since(Instant::now())
    }

    /// Whether the policy lets this user use the grant in this app: to
    /// show or choose, never to guard. AppOS checks every call itself.
    pub fn may(&self, grant: &str) -> bool {
        self.0.may.iter().any(|g| g == grant)
    }

    /// Whether an app serves now: deployed and on. To send people to
    /// another page while it doesn't.
    pub fn serves(&self, app: &str) -> bool {
        self.0.live.serves(app)
    }

    /// The context a Wasm kit called with this one gets: the same user,
    /// page and deadline, with `name` as the caller and the grants of
    /// `held` the policy lets this user use there.
    pub(crate) fn to_kit(&self, name: &str, held: &[String]) -> (Context, Access) {
        let access = self.0.live.access(self.user(), name, held);
        let may = held.iter().enumerate().filter(|(bit, _)| access.allows(*bit as u32)).map(|(_, g)| g.clone()).collect();
        let about = About { app: name.into(), user: self.0.user.clone(), page: self.0.page.clone(), deadline: self.0.deadline, may, live: self.0.live.clone() };
        (Context::new(about), access)
    }
}

/// Who is signed in, as the sign-in and the policy know them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ComponentType, Lift, Lower)]
#[component(record)]
pub struct User {
    /// What the sign-in calls them; stable.
    pub id: String,
    pub email: String,
    pub name: String,
    /// The sign-in's groups and those policy.csv puts them in.
    pub groups: Vec<String>,
}

/// The page request a call came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub method: String,
    /// Below the app's base, like "/fireside-2026/desk".
    pub path: String,
    /// Where the app is, like "https://apps.acme.com/events".
    pub url: String,
}

/// What a command carries besides its arguments: `Context` as it was at
/// the commit, kept in the outbox.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Sent {
    pub user: Option<User>,
    pub may: Vec<String>,
}

/// A grant a kit offers: what an app asks for in app.toml, what the policy
/// gives people, what risk.toml rates.
#[derive(Debug, Clone)]
pub struct Grant {
    /// Like "kv-read" or "leads-read".
    pub name: String,
    /// What it lets an app do, in words.
    pub allows: String,
    /// The functions it allows, as "<package>/<interface>#<function>",
    /// like "appos:framework/kv#get", without the package's version.
    pub functions: Vec<String>,
    /// I/O units one call costs, from the request's budget.
    pub cost: u32,
}

impl Grant {
    pub fn new(name: &str, allows: &str, functions: &[&str], cost: u32) -> Grant {
        Grant { name: name.into(), allows: allows.into(), functions: functions.iter().map(|f| f.to_string()).collect(), cost }
    }
}

/// appos.wit: the shell's own interfaces and the app world.
pub const APPOS_WIT: &str = include_str!("../../wit/appos.wit");
/// The line in appos.wit's app world after which the kits' imports go.
const KIT_IMPORTS: &str = "    // The platform's kits: appos new and appos sync add their interfaces.\n";

/// One WIT package of the kits.
struct Package {
    version: Option<String>,
    /// By file name.
    files: Vec<(String, String)>,
    interfaces: BTreeSet<String>,
}

/// A kit that calls other kits, as an app does: its name, where policy.csv
/// gives it grants (`p, anyone, scoring, leads-read`), and the grants it
/// holds, in order: a grant's place is its bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub name: String,
    pub grants: Vec<String>,
}

/// A kit: its WIT, its grants and the code behind its functions.
pub trait Kit: Send + Sync + 'static {
    /// The WIT files apps build against, by file name. kv's WIT is the
    /// shell's own appos.wit.
    fn wit(&self) -> Vec<(String, String)>;
    /// Its grants and the functions each allows.
    fn grants(&self) -> Vec<Grant>;
    /// Links its functions for one app. AppOS checks the grant and the
    /// policy before a call reaches them.
    fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()>;
    /// Runs one command after the sender's commit. An error tries it again
    /// later; the message's id is the same on every try, so a retry can be
    /// harmless.
    fn run(&self, cx: Context, msg: Message) -> Run<'_> {
        let _ = cx;
        Box::pin(async move { anyhow::bail!("{} is no command of this kit", msg.function) })
    }
    /// Its pages, like an app's routes. Each path starts with a name of
    /// the kit's own, which no app can take: "/booking/:draft".
    fn routes(&self) -> Vec<KitRoute> {
        vec![]
    }
    /// Who it is when it calls other kits; none if it calls none. A Rust
    /// kit calls Rust; a Wasm kit has one.
    fn caller(&self) -> Option<Caller> {
        None
    }
    /// Whether it serves now. While it doesn't, what uses it answers 503
    /// and its commands wait in the outbox.
    fn up(&self) -> bool {
        true
    }
    /// The kit as a Wasm component: the same functions with made-up data,
    /// for `appos dev` where the real kit can't run (`--fakes`). It keeps
    /// its data in its own directory, "/" in its WASI. AppOS writes its
    /// kit.toml from this kit's WIT and grants.
    fn fake(&self) -> Option<Vec<u8>> {
        None
    }
}

/// What a path's ":name" segments took, by name.
pub type Params = Vec<(String, String)>;

/// What a kit's page answers.
pub type Respond = Pin<Box<dyn Future<Output = anyhow::Result<Reply>> + Send + 'static>>;
type Handler = Arc<dyn Fn(Context, KitRequest) -> Respond + Send + Sync>;

/// A page of a kit, like an app's route. The first segment of its path is
/// where it is in policy.csv, as an app's id is: `p, anyone, booking,
/// booking-read` lets anyone open the pages at /booking/... that need
/// booking-read.
#[derive(Clone)]
pub struct KitRoute {
    method: String,
    path: String,
    segments: Vec<String>,
    grant: String,
    handler: Handler,
}

impl KitRoute {
    /// GET `path`, like "/booking/:draft", where ":draft" takes any one
    /// segment. The person needs `grant`, one of the kit's.
    pub fn get(path: &str, grant: &str, f: impl Fn(Context, KitRequest) -> Respond + Send + Sync + 'static) -> KitRoute {
        KitRoute::new("GET", path, grant, Arc::new(f))
    }

    /// POST `path`: a form sent to it, as `KitRequest::form`.
    pub fn post(path: &str, grant: &str, f: impl Fn(Context, KitRequest) -> Respond + Send + Sync + 'static) -> KitRoute {
        KitRoute::new("POST", path, grant, Arc::new(f))
    }

    fn new(method: &str, path: &str, grant: &str, handler: Handler) -> KitRoute {
        let segments = path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
        KitRoute { method: method.into(), path: path.into(), segments, grant: grant.into(), handler }
    }

    /// The grant the person needs.
    pub fn grant(&self) -> &str {
        &self.grant
    }

    /// Like "GET /booking/:draft".
    pub fn pattern(&self) -> String {
        format!("{} {}", self.method, self.path)
    }

    /// The name it is at: the path's first segment.
    pub(crate) fn name(&self) -> &str {
        self.segments.first().map_or("", String::as_str)
    }

    pub(crate) fn respond(&self, cx: Context, req: KitRequest) -> Respond {
        (self.handler)(cx, req)
    }

    /// Its params, if it takes this path.
    fn matches(&self, method: &str, path: &[&str]) -> Option<Params> {
        if self.method != method || self.segments.len() != path.len() {
            return None;
        }
        let mut params = Vec::new();
        for (pattern, actual) in self.segments.iter().zip(path) {
            match pattern.strip_prefix(':') {
                Some(name) => params.push((name.to_string(), actual.to_string())),
                None if pattern == actual => {}
                None => return None,
            }
        }
        Some(params)
    }
}

/// A request to a kit's page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitRequest {
    pub method: String,
    /// The whole path, like "/booking/k3x9".
    pub path: String,
    /// What the path's ":name" segments took.
    pub params: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
    pub form: Vec<(String, String)>,
    /// Where the site is, like "https://apps.acme.com".
    pub origin: String,
    /// Sent by htmx: answer with what it swaps.
    pub htmx: bool,
}

impl KitRequest {
    /// A ":name" segment of the path.
    pub fn param(&self, name: &str) -> Option<&str> {
        first(&self.params, name)
    }

    /// A value of the query string.
    pub fn query_value(&self, name: &str) -> Option<&str> {
        first(&self.query, name)
    }

    /// A field of the form; for several of one name, `form`.
    pub fn form_value(&self, name: &str) -> Option<&str> {
        first(&self.form, name)
    }
}

fn first<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

/// What a kit's page answers: a page in the shell's layout, or where the
/// browser goes next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub(crate) status: u16,
    pub(crate) title: String,
    pub(crate) head: Vec<String>,
    /// (name, html); "page" is the whole page.
    pub(crate) html: Vec<(String, String)>,
    pub(crate) redirect: Option<String>,
}

impl Reply {
    /// `html` in the shell's layout, titled `title`. HTML from an Askama
    /// template, as apps make theirs: it escapes what people typed.
    pub fn page(title: &str, html: impl Into<String>) -> Reply {
        Reply { status: 200, title: title.into(), head: vec![], html: vec![("page".into(), html.into())], redirect: None }
    }

    /// Sends the browser to `to`, a path on this site: a 303, which htmx
    /// follows too.
    pub fn redirect(to: &str) -> Reply {
        Reply { status: 303, title: String::new(), head: vec![], html: vec![], redirect: Some(to.into()) }
    }

    /// Another status than 200: 422 for a form with mistakes, which htmx
    /// swaps like 200.
    pub fn status(mut self, status: u16) -> Reply {
        self.status = status;
        self
    }

    /// HTML for <head>, like a <style>.
    pub fn head(mut self, html: impl Into<String>) -> Reply {
        self.head.push(html.into());
        self
    }

    /// For htmx requests: HTML swapped into the element with this id.
    pub fn swap(mut self, id: &str, html: impl Into<String>) -> Reply {
        self.html.push((id.into(), html.into()));
        self
    }
}

/// A command, delivered to its kit after the sender's commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The same on every try: a kit that did it already can skip it.
    pub id: String,
    /// Like "acme:crm/leads#add-lead".
    pub function: String,
    /// Tries that failed before this one.
    pub attempts: u32,
    args: String,
}

impl Message {
    /// The arguments, as the function takes them: a tuple, like
    /// `(String,)`.
    pub fn args<T: DeserializeOwned>(&self) -> anyhow::Result<T> {
        Ok(serde_json::from_str(&self.args)?)
    }

    /// The arguments as JSON.
    pub(crate) fn raw_args(&self) -> &str {
        &self.args
    }

    /// The message and its context, `time` to run.
    pub(crate) fn of(w: crate::tx::Waiting, time: Duration, live: Live) -> (Context, Message) {
        let sent: Sent = serde_json::from_str(&w.about).unwrap_or_default();
        let about = About { app: w.app, user: sent.user, page: None, deadline: Instant::now() + time, may: sent.may, live };
        (Context::new(about), Message { id: w.id, function: w.function, attempts: w.attempts, args: w.args })
    }
}

#[derive(Clone)]
struct Registered {
    kit: Arc<dyn Kit>,
    grants: Vec<Grant>,
    routes: Vec<KitRoute>,
    caller: Option<Caller>,
    /// Every interface of its WIT, as apps import it:
    /// "acme:crm/leads@1.0.0". Filled in by `Kits::checked`.
    imports: Vec<String>,
    /// One of the shell's own, which may offer appos:framework.
    builtin: bool,
    /// A Wasm kit's id.
    wasm: Option<String>,
}

impl Registered {
    fn new(kit: Arc<dyn Kit>, builtin: bool) -> Registered {
        Registered { grants: kit.grants(), routes: kit.routes(), caller: kit.caller(), kit, imports: vec![], builtin, wasm: None }
    }

    /// Its name, for messages: a Wasm kit's id, or its WIT's package.
    fn name(&self) -> String {
        if let Some(id) = &self.wasm {
            return id.clone();
        }
        let package = self.kit.wit().first().and_then(|(file, text)| read_wit(file, text).ok()).map(|(p, ..)| p);
        package.unwrap_or_else(|| "a kit".into())
    }

    /// The interfaces its grants' functions are in, like "acme:crm/leads".
    fn interfaces(&self) -> BTreeSet<&str> {
        self.grants.iter().flat_map(|g| &g.functions).map(|f| interface_of(f)).collect()
    }

    /// Its name for people: a Wasm kit's id, else its interface, or their
    /// package if they share one: "appos:framework/kv", "acme:crm".
    fn label(&self) -> String {
        if let Some(id) = &self.wasm {
            return id.clone();
        }
        let interfaces = self.interfaces();
        let packages: BTreeSet<&str> = interfaces.iter().map(|i| i.split_once('/').map_or(*i, |(p, _)| p)).collect();
        match (interfaces.len(), packages.len()) {
            (0, _) => self.name(),
            (1, _) => interfaces.into_iter().next().unwrap_or_default().to_string(),
            (_, 1) => packages.into_iter().next().unwrap_or_default().to_string(),
            _ => interfaces.into_iter().collect::<Vec<_>>().join(", "),
        }
    }
}

/// A kit as the admin screen shows it: what an app uses through a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Offer {
    /// A Wasm kit's id, else its interfaces: `Registered::label`.
    pub name: String,
    /// A Wasm kit's id.
    pub wasm: Option<String>,
    /// One of AppOS's own, like kv.
    pub builtin: bool,
    /// Loaded: for a Wasm kit, a version of it is.
    pub up: bool,
    /// The grants it holds: the kits it calls.
    pub calls: Vec<String>,
}

/// The kits of a platform: the shell's own first, then the platform's.
#[derive(Clone)]
pub struct Kits(Arc<Vec<Registered>>);

impl Default for Kits {
    fn default() -> Self {
        Kits::builtin()
    }
}

/// A kit's WIT file, read: its package ("acme:crm"), version and
/// interfaces.
pub(crate) fn read_wit(file: &str, text: &str) -> anyhow::Result<(String, Option<String>, Vec<String>)> {
    let read = wit_parser::UnresolvedPackageGroup::parse(file, text).map_err(|(map, e)| anyhow::anyhow!("kit WIT {file}: {}", e.render(&map)))?;
    let name = read.main.name;
    let interfaces = read.main.interfaces.iter().filter_map(|(_, i)| i.name.clone()).collect();
    Ok((format!("{}:{}", name.namespace, name.name), name.version.map(|v| v.to_string()), interfaces))
}

/// The kits' WIT packages ("acme:crm"), read and checked. A package may be
/// split across kits, with one version.
fn packages(all: &[Registered]) -> anyhow::Result<BTreeMap<String, Package>> {
    let mut packages: BTreeMap<String, Package> = BTreeMap::new();
    for r in all.iter().filter(|r| !r.builtin) {
        for (file, text) in r.kit.wit() {
            let (package, version, interfaces) = read_wit(&file, &text)?;
            let p = packages.entry(package.clone()).or_insert_with(|| Package { version: version.clone(), files: vec![], interfaces: BTreeSet::new() });
            anyhow::ensure!(p.version == version, "kit WIT {file}: {package} in two versions");
            p.interfaces.extend(interfaces);
            p.files.push((file, text));
        }
    }
    for r in all.iter().filter(|r| !r.builtin) {
        for interface in r.interfaces() {
            let (package, name) = interface.split_once('/').unwrap_or((interface, ""));
            anyhow::ensure!(packages.get(package).is_some_and(|p| p.interfaces.contains(name)), "a grant names {interface}, which no kit's WIT has");
        }
    }
    Ok(packages)
}

/// The kits' pages: each path starts with a name, the kit's own; no two
/// pages of one kit are the same; each needs one of the kit's grants.
fn check_routes(all: &[Registered]) -> anyhow::Result<()> {
    let mut names: HashMap<&str, usize> = HashMap::new();
    for (n, r) in all.iter().enumerate() {
        let mut seen = HashSet::new();
        for route in &r.routes {
            let pattern = route.pattern();
            let name = route.name();
            anyhow::ensure!(route.path.starts_with('/'), "page {pattern}: the path starts with /");
            crate::manifest::validate_app_id(name).map_err(|e| anyhow::anyhow!("page {pattern}: it must start with a name: {e}"))?;
            anyhow::ensure!(r.grants.iter().any(|g| g.name == route.grant), "page {pattern}: {} is none of the kit's grants", route.grant);
            anyhow::ensure!(
                seen.insert((route.method.as_str(), route.segments.iter().map(|s| if s.starts_with(':') { ":" } else { s }).collect::<Vec<_>>())),
                "page {pattern} twice"
            );
            anyhow::ensure!(*names.entry(name).or_insert(n) == n, "two kits have pages at /{name}");
        }
    }
    Ok(())
}

/// Who calls whom, by the grants: a kit calls the kits offering its grants.
fn calls(all: &[Registered]) -> Vec<Vec<usize>> {
    let offers = |grant: &str| all.iter().position(|r| r.grants.iter().any(|g| g.name == grant));
    all.iter().map(|r| r.caller.iter().flat_map(|c| &c.grants).filter_map(|g| offers(g)).collect()).collect()
}

/// The kits that call other kits: each name is its own, an app's id can't
/// be one, and no kit gets kv (a kit keeps its own data). Calls go one
/// way: a kit can't reach itself through others.
fn check_callers(all: &[Registered]) -> anyhow::Result<()> {
    let pages: HashMap<&str, usize> = all.iter().enumerate().flat_map(|(n, r)| r.routes.iter().map(move |p| (p.name(), n))).collect();
    let mut names = HashSet::new();
    for (n, r) in all.iter().enumerate() {
        let Some(c) = &r.caller else { continue };
        crate::manifest::validate_app_id(&c.name).map_err(|e| anyhow::anyhow!("kit {:?}: {e}", c.name))?;
        anyhow::ensure!(names.insert(c.name.as_str()), "two kits are called {}", c.name);
        anyhow::ensure!(pages.get(c.name.as_str()).is_none_or(|p| *p == n), "kit {}: another kit's pages are at /{}", c.name, c.name);
        anyhow::ensure!(c.grants.len() <= crate::manifest::MAX_GRANTS, "kit {}: more than {} grants", c.name, crate::manifest::MAX_GRANTS);
        for g in &c.grants {
            anyhow::ensure!(g != KV_READ && g != KV_WRITE, "kit {}: kits get no kv ({g}); a kit keeps its own data, in its directory", c.name);
        }
    }
    let calls = calls(all);
    fn reaches(calls: &[Vec<usize>], from: usize, to: usize, seen: &mut HashSet<usize>) -> bool {
        calls[from].iter().any(|&next| next == to || (seen.insert(next) && reaches(calls, next, to, seen)))
    }
    for (n, r) in all.iter().enumerate() {
        if let Some(c) = &r.caller
            && reaches(&calls, n, n, &mut HashSet::new())
        {
            anyhow::bail!("kit {} calls itself, through its grants: kits call one way", c.name);
        }
    }
    Ok(())
}

impl Kits {
    /// The kits every platform has: the shell's own, kv and the user.
    pub fn builtin() -> Kits {
        Kits::checked(vec![Registered::new(Arc::new(Kv), true), Registered::new(Arc::new(Users), true)]).expect("the built-in kits are valid")
    }

    /// With one kit more. Grant names, functions, interfaces and the names
    /// pages are at are each one kit's.
    pub fn with(self, kit: impl Kit) -> anyhow::Result<Kits> {
        let mut all = Arc::try_unwrap(self.0).unwrap_or_else(|shared| shared.to_vec());
        all.push(Registered::new(Arc::new(kit), false));
        Kits::checked(all)
    }

    /// With a Wasm kit, in place of its earlier version.
    pub(crate) fn with_wasm(&self, kit: WasmKit) -> anyhow::Result<Kits> {
        let id = kit.id().to_string();
        let mut all: Vec<Registered> = self.0.iter().filter(|r| r.wasm.as_deref() != Some(&id)).cloned().collect();
        all.push(Registered { wasm: Some(id), ..Registered::new(Arc::new(kit), false) });
        Kits::checked(all)
    }

    /// Without the Wasm kit `id`: what its next version links to.
    pub(crate) fn without(&self, id: &str) -> Kits {
        Kits(Arc::new(self.0.iter().filter(|r| r.wasm.as_deref() != Some(id)).cloned().collect()))
    }

    /// The WIT apps build against, by path below an app's wit/: appos.wit,
    /// whose app world imports every interface the kits offer, and each
    /// kit's package under deps/. An app imports only what it calls, so it
    /// needs only those grants.
    pub fn contract(&self) -> BTreeMap<String, String> {
        let packages = packages(&self.0).expect("checked when the kits were added");
        let mut imports = String::new();
        for r in self.0.iter().filter(|r| !r.builtin) {
            for interface in r.interfaces() {
                let package = interface.split('/').next().unwrap_or(interface);
                let version = packages.get(package).and_then(|p| p.version.as_ref()).map(|v| format!("@{v}")).unwrap_or_default();
                imports.push_str(&format!("    import {interface}{version};\n"));
            }
        }
        let appos = APPOS_WIT.replacen(KIT_IMPORTS, &format!("{KIT_IMPORTS}{imports}"), 1);
        let mut wit = BTreeMap::from([("appos.wit".to_string(), appos)]);
        for (package, p) in packages {
            for (file, text) in p.files {
                wit.insert(format!("deps/{}/{file}", package.replace(':', "-")), text);
            }
        }
        wit
    }

    fn checked(mut all: Vec<Registered>) -> anyhow::Result<Kits> {
        let (mut names, mut functions, mut files) = (HashSet::new(), HashSet::new(), HashSet::from(["appos.wit".to_string()]));
        let mut interfaces = HashSet::new();
        for r in all.iter() {
            for (file, _) in r.kit.wit() {
                anyhow::ensure!(
                    file.ends_with(".wit") && !file.contains(['/', '\\']) && files.insert(file.clone()),
                    "kit WIT file {file:?}: one name.wit per file, each its own"
                );
            }
            for g in &r.grants {
                crate::manifest::validate_grant_name(&g.name).map_err(anyhow::Error::msg)?;
                anyhow::ensure!(names.insert(g.name.as_str()), "two kits offer the grant {}", g.name);
                for f in &g.functions {
                    let Some((interface, function)) = f.split_once('#') else {
                        anyhow::bail!("grant {}: {f:?} must be <package>/<interface>#<function>", g.name);
                    };
                    anyhow::ensure!(
                        interface.contains(':') && interface.contains('/') && !interface.contains('@') && !function.is_empty(),
                        "grant {}: {f:?} must be <package>/<interface>#<function>, without a version",
                        g.name
                    );
                    anyhow::ensure!(r.builtin || !interface.starts_with(FRAMEWORK), "grant {}: {interface} is the shell's own", g.name);
                    anyhow::ensure!(functions.insert(f.as_str()), "two grants allow {f}");
                }
            }
            for i in r.interfaces() {
                anyhow::ensure!(interfaces.insert(i), "two kits offer {i}");
            }
        }
        packages(&all)?;
        check_routes(&all)?;
        check_callers(&all)?;
        for r in all.iter_mut().filter(|r| !r.builtin) {
            let mut imports = vec![];
            for (file, text) in r.kit.wit() {
                let (package, version, names) = read_wit(&file, &text)?;
                let version = version.map(|v| format!("@{v}")).unwrap_or_default();
                imports.extend(names.iter().map(|i| format!("{package}/{i}{version}")));
            }
            r.imports = imports;
        }
        Ok(Kits(Arc::new(all)))
    }

    /// The names the kits' pages are at, like "booking".
    pub fn page_names(&self) -> BTreeSet<&str> {
        self.0.iter().flat_map(|r| &r.routes).map(KitRoute::name).collect()
    }

    /// The kits' names, those of their pages and of those that call other
    /// kits: no app can have one as its id.
    pub fn taken(&self) -> BTreeSet<&str> {
        let mut taken = self.page_names();
        taken.extend(self.0.iter().filter_map(|r| r.caller.as_ref()).map(|c| c.name.as_str()));
        taken
    }

    /// What waits for: the first kit behind `grants`, and theirs, that
    /// doesn't serve: one that isn't up (a Wasm kit with no version
    /// loaded), or a Wasm kit `serves` says no to (off).
    pub(crate) fn waits(&self, grants: &[String], serves: &dyn Fn(&str) -> bool) -> Option<String> {
        let mut todo: Vec<&str> = grants.iter().map(String::as_str).collect();
        let mut seen = HashSet::new();
        while let Some(grant) = todo.pop() {
            let Some(n) = self.0.iter().position(|r| r.grants.iter().any(|g| g.name == grant)) else { continue };
            if !seen.insert(n) {
                continue;
            }
            let r = &self.0[n];
            if !r.kit.up() || r.wasm.as_deref().is_some_and(|id| !serves(id)) {
                return Some(r.name());
            }
            todo.extend(r.caller.iter().flat_map(|c| &c.grants).map(String::as_str));
        }
        None
    }

    /// The kit that offers `grant`.
    pub(crate) fn offer(&self, grant: &str) -> Option<Offer> {
        let r = self.0.iter().find(|r| r.grants.iter().any(|g| g.name == grant))?;
        let calls = r.caller.iter().flat_map(|c| c.grants.iter().cloned()).collect();
        Some(Offer { name: r.label(), wasm: r.wasm.clone(), builtin: r.builtin, up: r.kit.up(), calls })
    }

    /// The kit that offers an interface, and its grants.
    pub(crate) fn kit_of(&self, interface: &str) -> Option<(Arc<dyn Kit>, Vec<String>)> {
        let r = self.0.iter().find(|r| r.interfaces().contains(interface))?;
        Some((r.kit.clone(), r.grants.iter().map(|g| g.name.clone()).collect()))
    }

    /// Writes the kits' fakes into `dir`, replacing what it held, as
    /// `--fakes` does: one directory per kit, with its kit.toml.
    pub fn write_fakes(&self, dir: &Path) -> anyhow::Result<usize> {
        crate::wasm_kit::write_fakes(self, &crate::runtime::engine()?, dir)
    }

    /// Starts each kit's fake as `appos dev` does, alone with the built-in
    /// kits, and answers their names: for a platform's tests.
    pub fn check_fakes(&self) -> anyhow::Result<Vec<String>> {
        crate::wasm_kit::check_fakes(self)
    }

    /// The kits that bring a fake, each with a name of its own, its WIT,
    /// its grants and its commands: what `--fakes` writes.
    pub(crate) fn fakes(&self, engine: &wasmtime::Engine) -> anyhow::Result<Vec<Fake>> {
        let mut fakes: Vec<Fake> = vec![];
        for r in self.0.iter().filter(|r| !r.builtin) {
            let Some(wasm) = r.kit.fake() else { continue };
            let package = r.kit.wit().first().and_then(|(file, text)| read_wit(file, text).ok()).map(|(p, ..)| p.replace(':', "-"));
            let mut name = r.wasm.clone().or(package).unwrap_or_else(|| "kit".into());
            if fakes.iter().any(|f| f.name == name) {
                name = format!("{name}-{}", fakes.len() + 1);
            }
            let mut linker = Linker::new(engine);
            let held: Vec<String> = r.grants.iter().map(|g| g.name.clone()).collect();
            let mut linked = vec![];
            r.kit.link(&mut KitLinker { linker: &mut linker, grants: &r.grants, held: &held, uses: &HashMap::new(), linked: &mut linked })?;
            let commands = linked.into_iter().filter(|l| l.command).map(|l| l.name).collect();
            fakes.push(Fake { name, wit: r.kit.wit(), grants: r.grants.clone(), commands, wasm });
        }
        Ok(fakes)
    }

    /// The kit's page for a request: the route, what its params took and
    /// the kit's grants. `path` is split at '/', decoded.
    pub(crate) fn page(&self, method: &str, path: &[&str]) -> Option<(&KitRoute, Params, &[Grant])> {
        self.0.iter().find_map(|r| r.routes.iter().find_map(|route| Some((route, route.matches(method, path)?, r.grants.as_slice()))))
    }

    /// Every grant, in the order the kits offer them.
    pub fn grants(&self) -> impl Iterator<Item = &Grant> {
        self.0.iter().flat_map(|r| &r.grants)
    }

    /// Every grant's name.
    pub fn names(&self) -> Vec<String> {
        self.grants().map(|g| g.name.clone()).collect()
    }

    pub fn grant(&self, name: &str) -> Option<&Grant> {
        self.grants().find(|g| g.name == name)
    }

    /// What a grant lets an app do, in words.
    pub fn allows(&self, name: &str) -> &str {
        self.grant(name).map_or("Unknown grant", |g| g.allows.as_str())
    }

    /// Whether a kit offers this interface ("acme:crm/leads", no version).
    pub fn offers(&self, interface: &str) -> bool {
        let prefix = format!("{interface}#");
        self.grants().any(|g| g.functions.iter().any(|f| f.starts_with(&prefix)))
    }

    /// Whether a kit's WIT has this interface ("acme:crm/types", no
    /// version), with functions or types alone.
    pub fn knows(&self, interface: &str) -> bool {
        self.0.iter().flat_map(|r| &r.imports).any(|i| i.split('@').next() == Some(interface))
    }

    /// The grant that allows a function of an interface.
    pub fn grant_for(&self, interface: &str, function: &str) -> Option<&Grant> {
        let f = format!("{interface}#{function}");
        self.grants().find(|g| g.functions.contains(&f))
    }

    /// Every interface a kit offers: where commands go.
    pub(crate) fn interfaces(&self) -> Vec<String> {
        self.0.iter().flat_map(|r| r.interfaces().into_iter().map(str::to_string)).collect()
    }

    /// Links the functions the app's grants allow, kit by kit. `held` is
    /// the app's grants as app.toml lists them: a grant's place there is
    /// its bit in the request's `Access`. `uses` is the functions the app
    /// imports, with their types, by "acme:crm/leads#count-leads". Answers
    /// what it linked.
    pub(crate) fn link(&self, linker: &mut Linker<Ctx>, held: &[String], uses: &HashMap<String, ComponentFunc>) -> anyhow::Result<Vec<Linked>> {
        let mut linked = vec![];
        for r in self.0.iter() {
            if r.grants.iter().any(|g| held.contains(&g.name)) {
                r.kit.link(&mut KitLinker { linker, grants: &r.grants, held, uses, linked: &mut linked })?;
            }
            // An app imports an interface for its types alone when another
            // uses them (`use flow.{draft}`): it is there, with no
            // functions the app may not call.
            for interface in &r.imports {
                linker.instance(interface)?;
            }
        }
        Ok(linked)
    }
}

/// A function a kit linked for an app.
pub(crate) struct Linked {
    /// Like "acme:crm/leads#count-leads".
    pub name: String,
    pub grant: String,
    pub command: bool,
}

/// A kit's function an app (or a kit) imports: what a new version of the
/// kit must keep (wasm_kit.rs `KitVersion::fits`).
#[derive(Clone)]
pub(crate) struct Use {
    pub ty: ComponentFunc,
    pub grant: String,
    pub command: bool,
}

/// The kits now: the platform's own, fixed, and the Wasm kits' live
/// versions. A Wasm kit's version goes live under State's change lock
/// (state.rs): `with` checks the kits with it, `put` makes it live.
pub struct LiveKits {
    fixed: Kits,
    now: RwLock<Kits>,
    /// Each Wasm kit's live version, by id: what everything linked to it
    /// calls.
    slots: Mutex<HashMap<String, Slot>>,
}

impl LiveKits {
    pub fn new(fixed: Kits) -> LiveKits {
        LiveKits { now: RwLock::new(fixed.clone()), fixed, slots: Mutex::default() }
    }

    /// The kits now.
    pub fn now(&self) -> Kits {
        self.now.read().clone()
    }

    /// The platform's own.
    pub fn fixed(&self) -> &Kits {
        &self.fixed
    }

    fn slot(&self, id: &str) -> Slot {
        self.slots.lock().entry(id.to_string()).or_default().clone()
    }

    /// A Wasm kit's live version.
    pub(crate) fn live(&self, id: &str) -> Option<Arc<KitVersion>> {
        self.slots.lock().get(id)?.read().clone()
    }

    /// The kits with `v` live, checked: no cycle, each name and grant one
    /// kit's.
    pub(crate) fn with(&self, v: &KitVersion) -> anyhow::Result<Kits> {
        self.now().with_wasm(WasmKit::of(v, self.slot(&v.id)))
    }

    /// Makes `v` live, and `kits`, which `with` checked.
    pub(crate) fn put(&self, v: Arc<KitVersion>, kits: Kits) {
        let slot = self.slot(&v.id);
        *slot.write() = Some(v);
        *self.now.write() = kits;
    }
}

/// A kit's fake, as `--fakes` writes it (wasm_kit.rs).
pub(crate) struct Fake {
    /// Like "acme-crm": its package's.
    pub name: String,
    pub wit: Vec<(String, String)>,
    pub grants: Vec<Grant>,
    /// Its functions that are commands, like "acme:crm/customers#add-note".
    pub commands: Vec<String>,
    pub wasm: Vec<u8>,
}

/// How a kit links its functions for one app.
pub struct KitLinker<'a> {
    linker: &'a mut Linker<Ctx>,
    /// The kit's grants.
    grants: &'a [Grant],
    /// The app's.
    held: &'a [String],
    /// The functions the app imports, with their types.
    uses: &'a HashMap<String, ComponentFunc>,
    /// What it linked.
    linked: &'a mut Vec<Linked>,
}

impl KitLinker<'_> {
    /// What AppOS checks on a call of the function; none if the app may
    /// not call it at all.
    fn checked(&self, interface: &str, function: &str) -> anyhow::Result<Option<Checked>> {
        let unversioned = interface.split('@').next().unwrap_or(interface);
        let name = format!("{unversioned}#{function}");
        let Some(grant) = self.grants.iter().find(|g| g.functions.contains(&name)) else {
            anyhow::bail!("{name}: none of the kit's grants allows it");
        };
        // The app doesn't hold the grant: it may not import the function
        // (runtime::check_imports), so there is nothing to link.
        let Some(bit) = self.held.iter().position(|g| *g == grant.name) else { return Ok(None) };
        let short = unversioned.rsplit('/').next().unwrap_or(unversioned);
        Ok(Some(Checked { what: format!("{short}::{function}"), grant: grant.name.clone(), bit: bit as u32, cost: grant.cost, name }))
    }

    /// Links a function that answers now. `interface` is as apps import
    /// it, with its version: "acme:crm/leads@1.0.0". AppOS checks the
    /// grant, the policy and the budget, then calls `f` with the call's
    /// context. It may write the kit's data; the request can fail or run
    /// again after it, so make such a write harmless to repeat. What must
    /// happen once is a command.
    pub fn func<P, R, F>(&mut self, interface: &str, function: &str, f: F) -> anyhow::Result<()>
    where
        P: ComponentNamedList + Lift + Send + 'static,
        R: ComponentNamedList + Lower + Send + 'static,
        F: Fn(Context, P) -> Answer<'static, R> + Send + Sync + 'static,
    {
        self.func_in_store(interface, function, move |mut store, params| {
            let answer = store.data_mut().context().map(|cx| f(cx, params));
            Box::pin(async move { answer?.await })
        })
    }

    /// A query on the request's own store, for the shell's own kit.
    fn func_in_store<P, R, F>(&mut self, interface: &str, function: &str, f: F) -> anyhow::Result<()>
    where
        P: ComponentNamedList + Lift + Send + 'static,
        R: ComponentNamedList + Lower + Send + 'static,
        F: for<'b> Fn(StoreContextMut<'b, Ctx>, P) -> Answer<'b, R> + Send + Sync + 'static,
    {
        let Some(checked) = self.checked(interface, function)? else { return Ok(()) };
        self.linked.push(Linked { name: checked.name.clone(), grant: checked.grant.clone(), command: false });
        let checked = Arc::new(checked);
        let f = Arc::new(f);
        self.linker.instance(interface)?.func_wrap_async(function, move |mut store: StoreContextMut<'_, Ctx>, params: P| {
            let (checked, f) = (checked.clone(), f.clone());
            Box::new(async move {
                store.data_mut().in_request(&checked.what, &checked.grant, checked.bit, checked.cost)?;
                f(store, params).await
            })
        })?;
        Ok(())
    }

    /// Links a command: it returns nothing and runs after the request
    /// commits. AppOS checks the grant, the policy and the budget, and
    /// keeps the arguments (as JSON) in the request's transaction; the
    /// kit's `run` gets them after the commit. A request that fails, or
    /// runs again, sends nothing.
    pub fn command<P>(&mut self, interface: &str, function: &str) -> anyhow::Result<()>
    where
        P: ComponentNamedList + Lift + Serialize + Send + Sync + 'static,
    {
        let Some(checked) = self.checked(interface, function)? else { return Ok(()) };
        self.linked.push(Linked { name: checked.name.clone(), grant: checked.grant.clone(), command: true });
        let to = interface_of(&checked.name).to_string();
        self.linker.instance(interface)?.func_wrap(function, move |mut store: StoreContextMut<'_, Ctx>, params: P| {
            let ctx = store.data_mut();
            ctx.in_request(&checked.what, &checked.grant, checked.bit, checked.cost)?;
            let args = serde_json::to_string(&params).map_err(|e| wasmtime::format_err!("{}: {e}", checked.what))?;
            if args.len() > MAX_VALUE {
                return Err(wasmtime::format_err!("{}: the arguments are {} bytes, more than {MAX_VALUE}", checked.what, args.len()));
            }
            ctx.send(to.clone(), checked.name.clone(), args)
        })?;
        Ok(())
    }
}

/// For Wasm kits (wasm_kit.rs): functions whose types the kit's component
/// has, not Rust.
impl KitLinker<'_> {
    /// The type the app imports a function with, if it does:
    /// "acme:crm/leads", "count-leads".
    pub(crate) fn uses(&self, interface: &str, function: &str) -> Option<&ComponentFunc> {
        self.uses.get(&format!("{}#{function}", interface.split('@').next().unwrap_or(interface)))
    }

    /// Links a function that answers now, as `func` does.
    pub(crate) fn func_dynamic<F>(&mut self, interface: &str, function: &str, f: F) -> anyhow::Result<()>
    where
        F: for<'b> Fn(StoreContextMut<'b, Ctx>, &'b [Val], &'b mut [Val]) -> Answer<'b, ()> + Send + Sync + 'static,
    {
        let Some(checked) = self.checked(interface, function)? else { return Ok(()) };
        self.linked.push(Linked { name: checked.name.clone(), grant: checked.grant.clone(), command: false });
        let (checked, f) = (Arc::new(checked), Arc::new(f));
        self.linker.instance(interface)?.func_new_async(function, move |mut store: StoreContextMut<'_, Ctx>, _, params, results| {
            let (checked, f) = (checked.clone(), f.clone());
            Box::new(async move {
                store.data_mut().in_request(&checked.what, &checked.grant, checked.bit, checked.cost)?;
                f(store, params, results).await
            })
        })?;
        Ok(())
    }

    /// Links a command, as `command` does: its arguments are kept as JSON
    /// (wasm_kit.rs `to_json`).
    pub(crate) fn command_dynamic(&mut self, interface: &str, function: &str) -> anyhow::Result<()> {
        let Some(checked) = self.checked(interface, function)? else { return Ok(()) };
        self.linked.push(Linked { name: checked.name.clone(), grant: checked.grant.clone(), command: true });
        let to = interface_of(&checked.name).to_string();
        self.linker.instance(interface)?.func_new(function, move |mut store: StoreContextMut<'_, Ctx>, _, params, _| {
            let ctx = store.data_mut();
            ctx.in_request(&checked.what, &checked.grant, checked.bit, checked.cost)?;
            let args = serde_json::Value::Array(params.iter().map(crate::wasm_kit::to_json).collect::<wasmtime::Result<_>>()?).to_string();
            if args.len() > MAX_VALUE {
                return Err(wasmtime::format_err!("{}: the arguments are {} bytes, more than {MAX_VALUE}", checked.what, args.len()));
            }
            ctx.send(to.clone(), checked.name.clone(), args)
        })?;
        Ok(())
    }
}

/// The interface of a function: "acme:crm/leads" of "acme:crm/leads#count-leads".
fn interface_of(function: &str) -> &str {
    function.split('#').next().unwrap_or(function)
}

/// What AppOS checks before a call reaches the kit.
struct Checked {
    /// Like "kv::get", for messages.
    what: String,
    grant: String,
    bit: u32,
    cost: u32,
    /// Like "acme:crm/leads#count-leads".
    name: String,
}

/// kv: the app's own data, in the request's transaction (tx.rs). Its own
/// kit, so a platform can replace it.
struct Kv;

const KV: &str = "appos:framework/kv@0.1.0";

impl Kit for Kv {
    /// Its WIT is the shell's own appos.wit.
    fn wit(&self) -> Vec<(String, String)> {
        vec![]
    }

    fn grants(&self) -> Vec<Grant> {
        vec![
            Grant::new(KV_READ, "Read its data: kv::get, kv::scan", &["appos:framework/kv#get", "appos:framework/kv#scan"], READ),
            Grant::new(KV_WRITE, "Change its data: kv::set, kv::delete", &["appos:framework/kv#set", "appos:framework/kv#delete"], WRITE),
        ]
    }

    fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
        linker.func_in_store(KV, "get", |mut s, (key,): (String,)| Box::pin(async move { Ok((s.data_mut().kv_get(key).await?,)) }))?;
        linker.func_in_store(KV, "set", |mut s, (key, value): (String, String)| Box::pin(async move { s.data_mut().kv_set(key, value) }))?;
        linker.func_in_store(KV, "delete", |mut s, (key,): (String,)| Box::pin(async move { s.data_mut().kv_delete(key) }))?;
        linker.func_in_store(KV, "scan", |mut s, (prefix, after, limit, reverse): (String, Option<String>, u32, bool)| {
            Box::pin(async move { Ok((s.data_mut().kv_scan(prefix, after, limit, reverse).await?,)) })
        })
    }
}

/// The user: who is signed in, for apps.
struct Users;

const USER: &str = "appos:framework/users@0.1.0";

impl Kit for Users {
    /// Its WIT is the shell's own appos.wit.
    fn wit(&self) -> Vec<(String, String)> {
        vec![]
    }

    fn grants(&self) -> Vec<Grant> {
        vec![Grant::new(USER_READ, "See who is signed in: users::current", &["appos:framework/users#current"], READ)]
    }

    fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
        linker.func(USER, "current", |cx, ()| Box::pin(async move { Ok((cx.user().cloned(),)) }))
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;

    use super::*;
    use crate::access::Access;
    use crate::bundle::Bundle;
    use crate::platform::Outcome;
    use crate::runtime::{self, AdminLimits, Budget, Request};

    /// acme:crm with one query, count-leads, that answers 7 and keeps who
    /// asked what; and one command, add-lead, that keeps who added what.
    #[derive(Clone, Default)]
    struct Crm {
        asked: Arc<Mutex<Vec<Asked>>>,
        added: Arc<Mutex<Vec<Asked>>>,
    }

    /// The app, the user's email, the page's path and the argument.
    type Asked = (String, Option<String>, Option<String>, String);

    fn asked(cx: &Context, source: String) -> Asked {
        (cx.app().into(), cx.user().map(|u| u.email.clone()), cx.page().map(|p| format!("{} {}", p.method, p.path)), source)
    }

    fn ann() -> User {
        User { id: "ann".into(), email: "ann@acme.com".into(), name: "Ann".into(), groups: vec!["sales".into()] }
    }

    impl Kit for Crm {
        fn wit(&self) -> Vec<(String, String)> {
            vec![(
                "crm.wit".into(),
                "package acme:crm@1.0.0;\ninterface types { record lead { source: string } }\ninterface leads { count-leads: func(source: string) -> u32; add-lead: func(source: string); }\n".into(),
            )]
        }

        fn grants(&self) -> Vec<Grant> {
            vec![
                Grant::new("leads-read", "Count leads: leads::count-leads", &["acme:crm/leads#count-leads"], 2),
                Grant::new("leads-write", "Add leads", &["acme:crm/leads#add-lead"], 5),
            ]
        }

        fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
            let log = self.asked.clone();
            linker.func("acme:crm/leads@1.0.0", "count-leads", move |cx, (source,): (String,)| {
                log.lock().push(asked(&cx, source));
                Box::pin(async move { Ok((7u32,)) })
            })?;
            linker.command::<(String,)>("acme:crm/leads@1.0.0", "add-lead")
        }

        fn run(&self, cx: Context, msg: Message) -> Run<'_> {
            Box::pin(async move {
                anyhow::ensure!(msg.function == "acme:crm/leads#add-lead", "no command {}", msg.function);
                anyhow::ensure!(cx.time_left() > std::time::Duration::ZERO, "a command has time to run");
                let (source,): (String,) = msg.args()?;
                self.added.lock().push(asked(&cx, source));
                Ok(())
            })
        }
    }

    fn kits(crm: &Crm) -> Kits {
        Kits::builtin().with(crm.clone()).unwrap()
    }

    async fn load(kits: &Kits, grants: &str) -> anyhow::Result<runtime::LoadedApp> {
        load_wat(kits, grants, include_bytes!("testing-kit.wat"), crate::tx::in_memory()).await
    }

    async fn load_wat(kits: &Kits, grants: &str, wat: &[u8], db: crate::tx::Db) -> anyhow::Result<runtime::LoadedApp> {
        let bundle = Bundle {
            manifest: format!("[app]\nid = 'test'\nname = 'Test'\nversion = '1'\nowner = 'Tests'\ngroup = 'Tests'\n[capabilities]\ngrant = [{grants}]\n"),
            wasm: wat.to_vec(),
            icon: None,
            deps: None,
            kit: None,
        };
        runtime::load(&runtime::engine()?, db, kits, &bundle.checked()?, &std::env::temp_dir()).await
    }

    async fn get(app: &runtime::LoadedApp, access: Access) -> Result<runtime::Page, runtime::Failed> {
        run(app, "read", access, None).await.0
    }

    async fn run(app: &runtime::LoadedApp, handler: &str, access: Access, user: Option<User>) -> (Result<runtime::Page, runtime::Failed>, runtime::Cost) {
        let req = Request {
            method: "GET".into(),
            path: "/".into(),
            params: vec![],
            query: vec![],
            form: vec![],
            base: "/test".into(),
            origin: "http://localhost".into(),
            htmx: false,
            now: "2026-11-18T12:00:00Z".into(),
        };
        runtime::call(app, handler, &req, Budget::of(&app.manifest, AdminLimits::default()), access, user, Live::default()).await
    }

    #[tokio::test]
    async fn an_app_calls_a_platform_kit() {
        let crm = Crm::default();
        let app = load(&kits(&crm), "'leads-read'").await.unwrap();
        let page = get(&app, Access::ALL).await.unwrap_or_else(|f| panic!("{:#}", f.error));
        assert_eq!(page.html, [("page".to_string(), "seven".to_string())]);
        assert_eq!(*crm.asked.lock(), [("test".to_string(), None, Some("GET /".to_string()), "web".to_string())]);
    }

    #[tokio::test]
    async fn appos_checks_the_policy_before_the_kit_runs() {
        let crm = Crm::default();
        // kv-read first: leads-read is the app's grant number 1.
        let app = load(&kits(&crm), "'kv-read', 'leads-read'").await.unwrap();
        for access in [Access::NONE, Access::ALL.without(1)] {
            let Err(failed) = get(&app, access).await else { panic!("the policy denies leads-read") };
            assert!(failed.kind == Outcome::Denied, "{:#}", failed.error);
            assert_eq!(failed.error.to_string(), "denied leads-read");
        }
        assert!(crm.asked.lock().is_empty(), "the kit never saw the call");
    }

    #[tokio::test]
    async fn commands_run_after_the_commit() {
        let crm = Crm::default();
        let kits = kits(&crm);
        let db = crate::tx::in_memory();
        let app = load_wat(&kits, "'leads-write'", include_bytes!("testing-command.wat"), db.clone()).await.unwrap();
        let (page, cost) = run(&app, "send", Access::ALL, Some(ann())).await;
        assert_eq!(page.unwrap_or_else(|f| panic!("{:#}", f.error)).html, [("page".to_string(), "sent".to_string())]);
        assert_eq!(cost.sent, 2);
        assert!(crm.added.lock().is_empty(), "nothing runs before delivery");
        let sent = crate::tx::outbox(&db).unwrap();
        assert_eq!(
            sent.iter().map(|m| (m.app.as_str(), m.function.as_str(), m.args.as_str())).collect::<Vec<_>>(),
            [("test", "acme:crm/leads#add-lead", r#"["web"]"#), ("test", "acme:crm/leads#add-lead", r#"["ads"]"#)]
        );

        crate::outbox::start(db.clone(), crate::platform::PlatformDb::in_memory(), Live::of_kits(kits.clone()));
        for _ in 0..200 {
            if crate::tx::outbox(&db).unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        // For whom it runs: who was signed in at the commit; no page.
        let test = |s: &str| ("test".to_string(), Some("ann@acme.com".to_string()), None, s.to_string());
        assert_eq!(*crm.added.lock(), [test("web"), test("ads")]);
        assert!(crate::tx::outbox(&db).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_request_that_fails_or_is_denied_sends_nothing() {
        let crm = Crm::default();
        let db = crate::tx::in_memory();
        let app = load_wat(&kits(&crm), "'leads-write'", include_bytes!("testing-command.wat"), db.clone()).await.unwrap();
        let (page, cost) = run(&app, "crash", Access::ALL, None).await;
        assert!(page.is_err() && cost.sent == 0);
        let (page, _) = run(&app, "send", Access::NONE, None).await;
        assert_eq!(page.err().unwrap().error.to_string(), "denied leads-write");
        assert!(crate::tx::outbox(&db).unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_app_sees_who_is_signed_in_with_user_read() {
        let kits = Kits::builtin();
        let wat = include_bytes!("testing-user.wat");
        let app = load_wat(&kits, "'user-read'", wat, crate::tx::in_memory()).await.unwrap();
        let shown = |page: Result<runtime::Page, runtime::Failed>| page.unwrap_or_else(|f| panic!("{:#}", f.error)).html[0].1.clone();
        assert_eq!(shown(run(&app, "who", Access::ALL, Some(ann())).await.0), "ann@acme.com");
        assert_eq!(shown(run(&app, "who", Access::ALL, None).await.0), "nobody");
        let (page, _) = run(&app, "who", Access::NONE, Some(ann())).await;
        assert_eq!(page.err().unwrap().error.to_string(), "denied user-read");
        let err = load_wat(&kits, "", wat, crate::tx::in_memory()).await.err().unwrap().to_string();
        assert!(err.contains("uses users::current, which needs user-read"), "{err}");
    }

    /// A kit called `name` that offers `offers` and calls with `holds`, in
    /// the package acme:<offers>.
    struct Calls {
        name: &'static str,
        offers: &'static str,
        holds: Vec<String>,
    }

    impl Kit for Calls {
        fn wit(&self) -> Vec<(String, String)> {
            vec![(format!("{}.wit", self.offers), format!("package acme:{}@1.0.0;\ninterface i {{ f: func(); }}\n", self.offers))]
        }
        fn grants(&self) -> Vec<Grant> {
            vec![Grant::new(self.offers, "f", &[&format!("acme:{}/i#f", self.offers)], 1)]
        }
        fn caller(&self) -> Option<Caller> {
            Some(Caller { name: self.name.into(), grants: self.holds.clone() })
        }
        fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn kits_call_one_way() {
        let kit = |name, offers, holds: &[&str]| Calls { name, offers, holds: holds.iter().map(|g| g.to_string()).collect() };
        let a = || kit("a", "a-call", &["b-call"]);
        assert!(Kits::builtin().with(a()).unwrap().with(kit("b", "b-call", &[])).is_ok());
        let err = Kits::builtin().with(a()).unwrap().with(kit("b", "b-call", &["c-call"])).unwrap().with(kit("c", "c-call", &["a-call"])).err().unwrap();
        assert_eq!(err.to_string(), "kit a calls itself, through its grants: kits call one way");
        let err = Kits::builtin().with(kit("a", "a-call", &["a-call"])).err().unwrap();
        assert!(err.to_string().contains("kit a calls itself"));
        let err = Kits::builtin().with(a()).unwrap().with(kit("a", "b-call", &[])).err().unwrap();
        assert_eq!(err.to_string(), "two kits are called a");
        let err = Kits::builtin().with(kit("a", "a-call", &["kv-write"])).err().unwrap();
        assert!(err.to_string().contains("kits get no kv (kv-write)"), "{err}");
    }

    #[tokio::test]
    async fn an_app_needs_the_grant_and_a_kit_that_offers_it() {
        let crm = Crm::default();
        let err = load(&kits(&crm), "").await.err().unwrap().to_string();
        assert!(err.contains("uses leads::count-leads, which needs leads-read (add it to [capabilities] grant in app.toml)"), "{err}");
        let err = load(&kits(&crm), "'leads-write'").await.err().unwrap().to_string();
        assert!(err.contains("needs leads-read"), "{err}");
        let err = load(&Kits::builtin(), "'leads-read'").await.err().unwrap().to_string();
        assert!(err.contains(r#"unknown capability "leads-read"; known: kv-read, kv-write, user-read"#), "{err}");
    }

    #[test]
    fn grants_and_functions_are_each_one_kits() {
        let crm = Crm::default();
        let kits = kits(&crm);
        assert_eq!(kits.names(), ["kv-read", "kv-write", "user-read", "leads-read", "leads-write"]);
        assert_eq!(kits.grant_for("acme:crm/leads", "count-leads").map(|g| g.name.as_str()), Some("leads-read"));
        assert!(kits.offers("acme:crm/leads") && kits.offers("appos:framework/kv") && !kits.offers("acme:crm/deals"));
        struct Wit(&'static str);
        impl Kit for Wit {
            fn wit(&self) -> Vec<(String, String)> {
                vec![(self.0.into(), String::new())]
            }
            fn grants(&self) -> Vec<Grant> {
                vec![]
            }
            fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
                Ok(())
            }
        }
        for file in ["appos.wit", "crm.wit", "../x.wit", "crm.txt"] {
            assert!(kits.clone().with(Wit(file)).is_err(), "{file}");
        }

        struct Bad(Vec<Grant>);
        impl Kit for Bad {
            fn wit(&self) -> Vec<(String, String)> {
                vec![]
            }
            fn grants(&self) -> Vec<Grant> {
                self.0.clone()
            }
            fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
                Ok(())
            }
        }
        for (grant, why) in [
            (Grant::new("leads-read", "", &["acme:crm/deals#list"], 1), "two kits offer the grant leads-read"),
            (Grant::new("deals-read", "", &["acme:crm/leads#count-leads"], 1), "two grants allow acme:crm/leads#count-leads"),
            (Grant::new("own-kv", "", &["appos:framework/kv#get"], 1), "the shell's own"),
            (Grant::new("deals-read", "", &["acme:crm/deals@1.0.0#list"], 1), "without a version"),
            (Grant::new("admin", "", &["acme:crm/deals#list"], 1), "admin screen"),
            (Grant::new("Deals", "", &["acme:crm/deals#list"], 1), "lowercase"),
            (Grant::new("deals-read", "", &["acme:crm/leads#list-deals"], 1), "two kits offer acme:crm/leads"),
        ] {
            let err = kits.clone().with(Bad(vec![grant])).err().unwrap().to_string();
            assert!(err.contains(why), "{err}");
        }
        let err = kits.clone().with(Bad(vec![Grant::new("deals-read", "", &["acme:crm/deals#list"], 1)])).err().unwrap().to_string();
        assert!(err.contains("a grant names acme:crm/deals, which no kit's WIT has"), "{err}");

        // A kit may offer interfaces of several packages, and a package may
        // be split across kits, as the shell's own is (kv, users).
        let kits = kits.with(Two).unwrap();
        let interfaces = kits.interfaces();
        assert_eq!(interfaces, ["appos:framework/kv", "appos:framework/users", "acme:crm/leads", "acme:a/x", "acme:b/y"]);
    }

    /// Offers acme:a/x and acme:b/y.
    struct Two;

    impl Kit for Two {
        fn wit(&self) -> Vec<(String, String)> {
            vec![
                ("a.wit".into(), "package acme:a;\ninterface x { f: func(); }\n".into()),
                ("b.wit".into(), "package acme:b@2.1.0;\ninterface y { record r { s: string } g: func() -> r; }\n".into()),
            ]
        }
        fn grants(&self) -> Vec<Grant> {
            vec![Grant::new("a-read", "", &["acme:a/x#f"], 1), Grant::new("b-read", "", &["acme:b/y#g"], 1)]
        }
        fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn apps_build_against_appos_wit_and_the_kits_packages() {
        let builtin = Kits::builtin().contract();
        assert_eq!(builtin.keys().collect::<Vec<_>>(), ["appos.wit"]);
        assert_eq!(builtin["appos.wit"], APPOS_WIT);

        let contract = kits(&Crm::default()).with(Two).unwrap().contract();
        assert_eq!(contract.keys().collect::<Vec<_>>(), ["appos.wit", "deps/acme-a/a.wit", "deps/acme-b/b.wit", "deps/acme-crm/crm.wit"]);
        let imports = "    import acme:crm/leads@1.0.0;\n    import acme:a/x;\n    import acme:b/y@2.1.0;\n";
        assert!(contract["appos.wit"].contains(&format!("{KIT_IMPORTS}{imports}")), "{}", contract["appos.wit"]);

        // It is WIT that resolves, with the app world importing the kits.
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in &contract {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let mut resolve = wit_parser::Resolve::new();
        let (package, _) = resolve.push_dir(dir.path()).unwrap();
        let world = resolve.select_world(&[package], Some("app")).unwrap();
        let imported: Vec<String> = resolve.worlds[world].imports.keys().map(|k| resolve.name_world_key(k)).collect();
        assert!(imported.contains(&"acme:b/y@2.1.0".to_string()), "{imported:?}");
    }

    /// A kit with the pages given, and the grant desk-read.
    struct Pages(Vec<KitRoute>);

    impl Kit for Pages {
        fn wit(&self) -> Vec<(String, String)> {
            vec![("desk.wit".into(), "package acme:desk;\ninterface d { f: func(); }\n".into())]
        }
        fn grants(&self) -> Vec<Grant> {
            vec![Grant::new("desk-read", "", &["acme:desk/d#f"], 1)]
        }
        fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
            Ok(())
        }
        fn routes(&self) -> Vec<KitRoute> {
            self.0.clone()
        }
    }

    fn page(path: &str, grant: &str) -> KitRoute {
        KitRoute::get(path, grant, |_, _| Box::pin(async move { Ok(Reply::page("Desk", "")) }))
    }

    #[test]
    fn a_kits_pages_are_at_names_of_its_own() {
        let kits = Kits::builtin().with(Pages(vec![page("/desk", "desk-read"), page("/desk/:id", "desk-read"), page("/front", "desk-read")])).unwrap();
        assert_eq!(kits.page_names().into_iter().collect::<Vec<_>>(), ["desk", "front"]);
        let (route, params, _) = kits.page("GET", &["desk", "7"]).unwrap();
        assert_eq!((route.pattern(), params), ("GET /desk/:id".to_string(), vec![("id".to_string(), "7".to_string())]));
        assert!(kits.page("POST", &["desk", "7"]).is_none() && kits.page("GET", &["desk", "7", "x"]).is_none());

        for (path, grant, why) in [
            ("desk", "desk-read", "starts with /"),
            ("/:id", "desk-read", "must start with a name"),
            ("/", "desk-read", "must start with a name"),
            ("/admin/x", "desk-read", "reserved"),
            ("/desk", "kv-read", "kv-read is none of the kit's grants"),
        ] {
            let err = Kits::builtin().with(Pages(vec![page(path, grant)])).err().unwrap().to_string();
            assert!(err.contains(why), "{path}: {err}");
        }
        let err = Kits::builtin().with(Pages(vec![page("/desk/:id", "desk-read"), page("/desk/:other", "desk-read")])).err().unwrap().to_string();
        assert!(err.contains("page GET /desk/:other twice"), "{err}");
        struct Other;
        impl Kit for Other {
            fn wit(&self) -> Vec<(String, String)> {
                vec![("front.wit".into(), "package acme:front;\ninterface f { g: func(); }\n".into())]
            }
            fn grants(&self) -> Vec<Grant> {
                vec![Grant::new("front-read", "", &["acme:front/f#g"], 1)]
            }
            fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
                Ok(())
            }
            fn routes(&self) -> Vec<KitRoute> {
                vec![page("/front/x", "front-read")]
            }
        }
        let err = kits.with(Other).err().unwrap().to_string();
        assert!(err.contains("two kits have pages at /front"), "{err}");
    }

    #[test]
    fn a_kits_wit_must_parse() {
        struct Broken;
        impl Kit for Broken {
            fn wit(&self) -> Vec<(String, String)> {
                vec![("broken.wit".into(), "package acme:broken;\ninterface x { f: func(; }\n".into())]
            }
            fn grants(&self) -> Vec<Grant> {
                vec![]
            }
            fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
                Ok(())
            }
        }
        let err = Kits::builtin().with(Broken).err().unwrap().to_string();
        assert!(err.starts_with("kit WIT broken.wit: "), "{err}");
    }

    #[test]
    fn a_platform_with_a_mistake_does_not_start() {
        let crm = Crm::default();
        let platform = crate::Platform::new().kit(crm.clone()).kit(crm);
        assert!(platform.kits.is_err());
    }
}
