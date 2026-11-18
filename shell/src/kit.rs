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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wasmtime::StoreContextMut;
use wasmtime::component::Linker;

use crate::runtime::{Ctx, FRAMEWORK, MAX_VALUE, READ, WRITE};
use crate::state::Serving;

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
    pub serving: Serving,
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
        self.0.serving.serves(app)
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

    /// The message and its context, `time` to run.
    pub(crate) fn of(w: crate::tx::Waiting, time: Duration, serving: Serving) -> (Context, Message) {
        let sent: Sent = serde_json::from_str(&w.about).unwrap_or_default();
        let about = About { app: w.app, user: sent.user, page: None, deadline: Instant::now() + time, may: sent.may, serving };
        (Context::new(about), Message { id: w.id, function: w.function, attempts: w.attempts, args: w.args })
    }
}

#[derive(Clone)]
struct Registered {
    kit: Arc<dyn Kit>,
    grants: Vec<Grant>,
    routes: Vec<KitRoute>,
    /// Every interface of its WIT, as apps import it:
    /// "acme:crm/leads@1.0.0". Filled in by `Kits::checked`.
    imports: Vec<String>,
    /// One of the shell's own, which may offer appos:framework.
    builtin: bool,
}

impl Registered {
    fn new(kit: Arc<dyn Kit>, builtin: bool) -> Registered {
        Registered { grants: kit.grants(), routes: kit.routes(), kit, imports: vec![], builtin }
    }

    /// The interfaces its grants' functions are in, like "acme:crm/leads".
    fn interfaces(&self) -> BTreeSet<&str> {
        self.grants.iter().flat_map(|g| &g.functions).map(|f| interface_of(f)).collect()
    }
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
fn read_wit(file: &str, text: &str) -> anyhow::Result<(String, Option<String>, Vec<String>)> {
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

    /// The names the kits' pages are at, like "booking": no app can have
    /// one as its id.
    pub fn page_names(&self) -> BTreeSet<&str> {
        self.0.iter().flat_map(|r| &r.routes).map(KitRoute::name).collect()
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

    /// Every interface and the kit that offers it: where commands go.
    pub(crate) fn interfaces(&self) -> Vec<(String, Arc<dyn Kit>)> {
        self.0.iter().flat_map(|r| r.interfaces().into_iter().map(|i| (i.to_string(), r.kit.clone()))).collect()
    }

    /// Links the functions the app's grants allow, kit by kit. `held` is
    /// the app's grants as app.toml lists them: a grant's place there is
    /// its bit in the request's `Access`.
    pub(crate) fn link(&self, linker: &mut Linker<Ctx>, held: &[String]) -> anyhow::Result<()> {
        for r in self.0.iter() {
            if r.grants.iter().any(|g| held.contains(&g.name)) {
                r.kit.link(&mut KitLinker { linker, grants: &r.grants, held })?;
            }
            // An app imports an interface for its types alone when another
            // uses them (`use flow.{draft}`): it is there, with no
            // functions the app may not call.
            for interface in &r.imports {
                linker.instance(interface)?;
            }
        }
        Ok(())
    }
}

/// How a kit links its functions for one app.
pub struct KitLinker<'a> {
    linker: &'a mut Linker<Ctx>,
    /// The kit's grants.
    grants: &'a [Grant],
    /// The app's.
    held: &'a [String],
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
        };
        runtime::load(&runtime::engine()?, db, kits, &bundle.checked()?).await
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
        runtime::call(app, handler, &req, Budget::of(&app.manifest, AdminLimits::default()), access, user, Serving::default()).await
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

        crate::outbox::start(db.clone(), crate::platform::PlatformDb::in_memory(), &kits, Serving::default()).wake();
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
        let interfaces: Vec<String> = kits.interfaces().into_iter().map(|(i, _)| i).collect();
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
