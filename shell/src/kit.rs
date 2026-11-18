//! Kits: what apps call besides the page. A kit is a WIT package and the
//! code behind it; kv is the first. A platform adds its own:
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
//! A function is a query or a command. A query answers now and only reads
//! (`KitLinker::func`). A command returns nothing and runs after the
//! request commits (`KitLinker::command`): the commit puts it in the
//! outbox, and `Kit::run` gets it from there (outbox.rs). One kit is one
//! WIT package.

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasmtime::StoreContextMut;
use wasmtime::component::Linker;

use crate::runtime::{Ctx, FRAMEWORK, MAX_VALUE, READ, WRITE};

/// What kit functions take and return: tuples of these, and records that
/// derive them.
pub use wasmtime::component::{ComponentNamedList, ComponentType, Lift, Lower};
pub use wasmtime::{Error, Result};

/// What a kit function returns: a future borrowing the call.
pub type Answer<'a, R> = Pin<Box<dyn Future<Output = Result<R>> + Send + 'a>>;
/// What running a command returns.
pub type Run<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// The grants kv offers.
pub const KV_READ: &str = "kv-read";
pub const KV_WRITE: &str = "kv-write";

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
    fn run(&self, msg: Message) -> Run<'_> {
        Box::pin(async move { anyhow::bail!("{} is no command of this kit", msg.function) })
    }
}

/// A command, delivered to its kit after the sender's commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The same on every try: a kit that did it already can skip it.
    pub id: String,
    /// The app that sent it.
    pub app: String,
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

    pub(crate) fn of(w: crate::tx::Waiting) -> Message {
        Message { id: w.id, app: w.app, function: w.function, attempts: w.attempts, args: w.args }
    }
}

struct Registered {
    kit: Arc<dyn Kit>,
    grants: Vec<Grant>,
}

/// The kits of a platform: kv first, then the platform's own.
#[derive(Clone)]
pub struct Kits(Arc<Vec<Registered>>);

impl Default for Kits {
    fn default() -> Self {
        Kits::builtin()
    }
}

impl Kits {
    /// The kits every platform has: kv.
    pub fn builtin() -> Kits {
        Kits(Arc::new(vec![Registered { grants: Kv.grants(), kit: Arc::new(Kv) }])).checked().expect("the built-in kits are valid")
    }

    /// With one kit more. Grant names and functions are each one kit's.
    pub fn with(self, kit: impl Kit) -> anyhow::Result<Kits> {
        let mut all =
            Arc::try_unwrap(self.0).unwrap_or_else(|shared| shared.iter().map(|r| Registered { kit: r.kit.clone(), grants: r.grants.clone() }).collect());
        all.push(Registered { grants: kit.grants(), kit: Arc::new(kit) });
        Kits(Arc::new(all)).checked()
    }

    fn checked(self) -> anyhow::Result<Kits> {
        let (mut names, mut functions, mut files) = (HashSet::new(), HashSet::new(), HashSet::from(["appos.wit".to_string()]));
        let mut packages = HashSet::new();
        for (i, r) in self.0.iter().enumerate() {
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
                    anyhow::ensure!(i == 0 || !interface.starts_with(FRAMEWORK), "grant {}: {interface} is the shell's own", g.name);
                    anyhow::ensure!(functions.insert(f.as_str()), "two grants allow {f}");
                }
            }
            let mine: BTreeSet<&str> = r.grants.iter().flat_map(|g| &g.functions).map(|f| package(f)).collect();
            anyhow::ensure!(mine.len() <= 1, "a kit is one WIT package, not {}", mine.into_iter().collect::<Vec<_>>().join(" and "));
            if let Some(p) = mine.into_iter().next() {
                anyhow::ensure!(packages.insert(p), "two kits offer {p}");
            }
        }
        Ok(self)
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

    /// The grant that allows a function of an interface.
    pub fn grant_for(&self, interface: &str, function: &str) -> Option<&Grant> {
        let f = format!("{interface}#{function}");
        self.grants().find(|g| g.functions.contains(&f))
    }

    /// The kits' WIT files, apart from appos.wit.
    pub fn wit(&self) -> Vec<(String, String)> {
        self.0.iter().flat_map(|r| r.kit.wit()).collect()
    }

    /// Each kit with functions, by its WIT package: where commands go.
    pub(crate) fn packages(&self) -> Vec<(String, Arc<dyn Kit>)> {
        let first = |r: &Registered| r.grants.iter().flat_map(|g| &g.functions).next().map(|f| package(f).to_string());
        self.0.iter().filter_map(|r| Some((first(r)?, r.kit.clone()))).collect()
    }

    /// Links the functions the app's grants allow, kit by kit. `held` is
    /// the app's grants as app.toml lists them: a grant's place there is
    /// its bit in the request's `Access`.
    pub(crate) fn link(&self, linker: &mut Linker<Ctx>, held: &[String]) -> anyhow::Result<()> {
        for r in self.0.iter() {
            if r.grants.iter().any(|g| held.contains(&g.name)) {
                r.kit.link(&mut KitLinker { linker, grants: &r.grants, held })?;
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

    /// Links a query: it answers now and only reads. `interface` is as
    /// apps import it, with its version: "acme:crm/leads@1.0.0". AppOS
    /// checks the grant, the policy and the budget, then calls `f`.
    pub fn func<P, R, F>(&mut self, interface: &str, function: &str, f: F) -> anyhow::Result<()>
    where
        P: ComponentNamedList + Lift + Send + 'static,
        R: ComponentNamedList + Lower + Send + 'static,
        F: for<'b> Fn(Call<'b>, P) -> Answer<'b, R> + Send + Sync + 'static,
    {
        let Some(checked) = self.checked(interface, function)? else { return Ok(()) };
        let checked = Arc::new(checked);
        let f = Arc::new(f);
        self.linker.instance(interface)?.func_wrap_async(function, move |mut store: StoreContextMut<'_, Ctx>, params: P| {
            let (checked, f) = (checked.clone(), f.clone());
            Box::new(async move {
                store.data_mut().in_request(&checked.what, &checked.grant, checked.bit, checked.cost)?;
                f(Call { store }, params).await
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
        let kit = package(&checked.name).to_string();
        self.linker.instance(interface)?.func_wrap(function, move |mut store: StoreContextMut<'_, Ctx>, params: P| {
            let ctx = store.data_mut();
            ctx.in_request(&checked.what, &checked.grant, checked.bit, checked.cost)?;
            let args = serde_json::to_string(&params).map_err(|e| wasmtime::format_err!("{}: {e}", checked.what))?;
            if args.len() > MAX_VALUE {
                return Err(wasmtime::format_err!("{}: the arguments are {} bytes, more than {MAX_VALUE}", checked.what, args.len()));
            }
            ctx.send(kit.clone(), checked.name.clone(), args);
            Ok(())
        })?;
        Ok(())
    }
}

/// The WIT package of a function: "acme:crm" of "acme:crm/leads#count-leads".
fn package(function: &str) -> &str {
    function.split('/').next().unwrap_or(function)
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

/// One call of a kit function, by an app handling a request.
pub struct Call<'a> {
    store: StoreContextMut<'a, Ctx>,
}

impl Call<'_> {
    /// The calling app's id.
    pub fn app(&mut self) -> &str {
        self.store.data().app_id()
    }

    pub(crate) fn ctx(&mut self) -> &mut Ctx {
        self.store.data_mut()
    }
}

/// kv: the app's own data, in the request's transaction (tx.rs).
struct Kv;

const KV: &str = "appos:framework/kv@0.1.0";

impl Kit for Kv {
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
        linker.func(KV, "get", |mut call, (key,): (String,)| Box::pin(async move { Ok((call.ctx().kv_get(key).await?,)) }))?;
        linker.func(KV, "set", |mut call, (key, value): (String, String)| Box::pin(async move { call.ctx().kv_set(key, value) }))?;
        linker.func(KV, "delete", |mut call, (key,): (String,)| Box::pin(async move { call.ctx().kv_delete(key) }))?;
        linker.func(KV, "scan", |mut call, (prefix, after, limit, reverse): (String, Option<String>, u32, bool)| {
            Box::pin(async move { Ok((call.ctx().kv_scan(prefix, after, limit, reverse).await?,)) })
        })
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
        asked: Arc<Mutex<Vec<(String, String)>>>,
        added: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl Kit for Crm {
        fn wit(&self) -> Vec<(String, String)> {
            vec![(
                "crm.wit".into(),
                "package acme:crm@1.0.0;\ninterface leads { count-leads: func(source: string) -> u32; add-lead: func(source: string); }\n".into(),
            )]
        }

        fn grants(&self) -> Vec<Grant> {
            vec![
                Grant::new("leads-read", "Count leads: leads::count-leads", &["acme:crm/leads#count-leads"], 2),
                Grant::new("leads-write", "Add leads", &["acme:crm/leads#add-lead"], 5),
            ]
        }

        fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
            let asked = self.asked.clone();
            linker.func("acme:crm/leads@1.0.0", "count-leads", move |mut call, (source,): (String,)| {
                let asked = asked.clone();
                Box::pin(async move {
                    asked.lock().push((call.app().to_string(), source));
                    Ok((7u32,))
                })
            })?;
            linker.command::<(String,)>("acme:crm/leads@1.0.0", "add-lead")
        }

        fn run(&self, msg: Message) -> Run<'_> {
            Box::pin(async move {
                anyhow::ensure!(msg.function == "acme:crm/leads#add-lead", "no command {}", msg.function);
                let (source,): (String,) = msg.args()?;
                self.added.lock().push((msg.app, source));
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
        run(app, "read", access).await.0
    }

    async fn run(app: &runtime::LoadedApp, handler: &str, access: Access) -> (Result<runtime::Page, runtime::Failed>, runtime::Cost) {
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
        runtime::call(app, handler, &req, Budget::of(&app.manifest, AdminLimits::default()), access).await
    }

    #[tokio::test]
    async fn an_app_calls_a_platform_kit() {
        let crm = Crm::default();
        let app = load(&kits(&crm), "'leads-read'").await.unwrap();
        let page = get(&app, Access::ALL).await.unwrap_or_else(|f| panic!("{:#}", f.error));
        assert_eq!(page.html, [("page".to_string(), "seven".to_string())]);
        assert_eq!(*crm.asked.lock(), [("test".to_string(), "web".to_string())]);
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
        let (page, cost) = run(&app, "send", Access::ALL).await;
        assert_eq!(page.unwrap_or_else(|f| panic!("{:#}", f.error)).html, [("page".to_string(), "sent".to_string())]);
        assert_eq!(cost.sent, 2);
        assert!(crm.added.lock().is_empty(), "nothing runs before delivery");
        let sent = crate::tx::outbox(&db).unwrap();
        assert_eq!(
            sent.iter().map(|m| (m.app.as_str(), m.function.as_str(), m.args.as_str())).collect::<Vec<_>>(),
            [("test", "acme:crm/leads#add-lead", r#"["web"]"#), ("test", "acme:crm/leads#add-lead", r#"["ads"]"#)]
        );

        crate::outbox::start(db.clone(), crate::platform::PlatformDb::in_memory(), &kits).wake();
        for _ in 0..200 {
            if crate::tx::outbox(&db).unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let test = |s: &str| ("test".to_string(), s.to_string());
        assert_eq!(*crm.added.lock(), [test("web"), test("ads")]);
        assert!(crate::tx::outbox(&db).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_request_that_fails_or_is_denied_sends_nothing() {
        let crm = Crm::default();
        let db = crate::tx::in_memory();
        let app = load_wat(&kits(&crm), "'leads-write'", include_bytes!("testing-command.wat"), db.clone()).await.unwrap();
        let (page, cost) = run(&app, "crash", Access::ALL).await;
        assert!(page.is_err() && cost.sent == 0);
        let (page, _) = run(&app, "send", Access::NONE).await;
        assert_eq!(page.err().unwrap().error.to_string(), "denied leads-write");
        assert!(crate::tx::outbox(&db).unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_app_needs_the_grant_and_a_kit_that_offers_it() {
        let crm = Crm::default();
        let err = load(&kits(&crm), "").await.err().unwrap().to_string();
        assert!(err.contains("uses leads::count-leads, which needs leads-read (add it to [capabilities] grant in app.toml)"), "{err}");
        let err = load(&kits(&crm), "'leads-write'").await.err().unwrap().to_string();
        assert!(err.contains("needs leads-read"), "{err}");
        let err = load(&Kits::builtin(), "'leads-read'").await.err().unwrap().to_string();
        assert!(err.contains(r#"unknown capability "leads-read"; known: kv-read, kv-write"#), "{err}");
    }

    #[test]
    fn grants_and_functions_are_each_one_kits() {
        let crm = Crm::default();
        let kits = kits(&crm);
        assert_eq!(kits.names(), ["kv-read", "kv-write", "leads-read", "leads-write"]);
        assert_eq!(kits.grant_for("acme:crm/leads", "count-leads").map(|g| g.name.as_str()), Some("leads-read"));
        assert!(kits.offers("acme:crm/leads") && kits.offers("appos:framework/kv") && !kits.offers("acme:crm/deals"));
        assert_eq!(kits.wit().len(), 1);
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
            (Grant::new("deals-read", "", &["acme:crm/deals#list"], 1), "two kits offer acme:crm"),
        ] {
            let err = kits.clone().with(Bad(vec![grant])).err().unwrap().to_string();
            assert!(err.contains(why), "{err}");
        }
        let two = Bad(vec![Grant::new("a-read", "", &["acme:a/x#f"], 1), Grant::new("b-read", "", &["acme:b/x#f"], 1)]);
        let err = kits.clone().with(two).err().unwrap().to_string();
        assert!(err.contains("a kit is one WIT package, not acme:a and acme:b"), "{err}");
    }

    #[test]
    fn a_platform_with_a_mistake_does_not_start() {
        let crm = Crm::default();
        let platform = crate::Platform::new().kit(crm.clone()).kit(crm);
        assert!(platform.kits.is_err());
    }
}
