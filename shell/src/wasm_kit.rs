//! Kits in Wasm (kit.rs): a component and its kit.toml. A Wasm kit is held
//! like an app (state.rs): deployed with `appos deploy` in its directory,
//! with versions, rollback, off and on, or loaded when the platform starts
//! (`--kit DIR`), as `APP` folders are.
//!
//! ```toml
//! [kit]
//! id = "scoring"                      # its name: in policy.csv, for its directory
//! name = "Lead scoring"
//! version = "1.0.0"
//! owner = "Jane Doe <jane@acme.com>"  # a deploy needs an owner and a group
//! group = "Sales"
//! wasm = "kit.wasm"                   # next to kit.toml
//! wit = ["api/scoring.wit"]           # what it offers
//! commands = ["acme:scoring/leads#add-lead"]   # its functions that are commands
//! fake = "fake.wasm"                  # what appos dev runs in its place, if it has one
//!
//! [grants.scoring-read]
//! allows = "Score leads: leads::score"
//! functions = ["acme:scoring/leads#score"]
//! cost = 1                            # I/O units a call costs; 1 when left out
//!
//! [capabilities]
//! grant = ["crm-read"]                # other kits' grants it calls with
//!
//! [limits]
//! seconds = 5                         # for its pages, as in app.toml
//!
//! [config]                            # environment variables
//! REGION = "eu"
//! [config-env]                        # read from the shell's environment
//! API_KEY = "SCORING_API_KEY"
//! ```
//!
//! A call runs like an app's request: a fresh instance for each call, with
//! what its caller has left of the request's fuel, I/O and time, and memory
//! of its own. A chain of calls goes at most `MAX_DEPTH` deep. Unlike an app
//! it has a directory of its own (data/kits/<id>, "/" in its WASI), the
//! network and its configuration as environment variables. It gets no kv:
//! it keeps its own data. What it takes and gives are values: no
//! resources, streams or futures. A command to it runs once the sender
//! committed, with a whole budget of its own.
//!
//! If its component exports appos:framework/handler, it has pages too: its
//! init and requests run like an app's, at /<id>, on a budget of their own,
//! in its WASI.
//!
//! Everything linked to a kit calls its live version, through its `Slot`:
//! a new version replaces the old one in place, once everything that uses
//! the kit still fits it (`fits`).
//!
//! A sign-in kit (auth/sign_in_kit.rs) has a kit.toml too, with
//! `[sign-in] hosts = [...]`, the hosts it may fetch from, and no WIT or
//! grants: auth.toml names it, and `config.get` gives it its [config].

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::{Value as Json, json};
use wasmtime::StoreContextMut;
use wasmtime::component::types::{ComponentFunc, ComponentItem, Type};
use wasmtime::component::{Component, InstancePre, Val};

use crate::bundle::{Bundle, KitFiles};
use crate::kit::{Caller, Context, Fake, Grant, KV_READ, KV_WRITE, Kit, KitLinker, Kits, LiveKits, Message, Run, Use};
use crate::manifest::{AppInfo, Limits, Manifest};
use crate::runtime::{Called, Ctx, FUEL, KitSandbox, Left, call_kit};

/// Bytes of a kit.toml and of one of its WIT files.
const MAX_TEXT: u64 = 1 << 20;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KitToml {
    kit: About,
    #[serde(default)]
    grants: BTreeMap<String, GrantToml>,
    #[serde(default)]
    capabilities: crate::manifest::Capabilities,
    #[serde(default)]
    limits: Limits,
    #[serde(default)]
    config: BTreeMap<String, String>,
    #[serde(default, rename = "config-env")]
    config_env: BTreeMap<String, String>,
    #[serde(default, rename = "sign-in")]
    sign_in: Option<SignInToml>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignInToml {
    #[serde(default)]
    hosts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct About {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default = "default_wasm")]
    wasm: PathBuf,
    #[serde(default)]
    wit: Vec<PathBuf>,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    fake: Option<PathBuf>,
}

fn default_wasm() -> PathBuf {
    PathBuf::from("kit.wasm")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantToml {
    allows: String,
    functions: Vec<String>,
    #[serde(default = "one")]
    cost: u32,
}

fn one() -> u32 {
    1
}

/// What a kit.toml says a kit offers, beyond what an app.toml says.
#[derive(Debug, Clone)]
pub struct Offers {
    pub grants: Vec<Grant>,
    /// Its functions that are commands, like "acme:crm/customers#add-note".
    pub commands: Vec<String>,
    /// Its WIT files, as kit.toml names them: below its directory.
    pub wit: Vec<PathBuf>,
    /// Its fake, below its directory.
    pub fake: Option<PathBuf>,
    config: BTreeMap<String, String>,
    config_env: BTreeMap<String, String>,
}

/// A kit.toml read into a manifest, as an app.toml would be, with what the
/// kit offers.
pub(crate) fn manifest(text: &str) -> anyhow::Result<Manifest> {
    let toml: KitToml = toml::from_str(text)?;
    anyhow::ensure!(toml.sign_in.is_none(), "a sign-in kit: auth.toml names it (provider = \"kit\")");
    anyhow::ensure!(!toml.kit.wit.is_empty(), "wit = [...]: the WIT of what it offers");
    let mut grants = vec![];
    for (name, g) in &toml.grants {
        crate::manifest::validate_grant_name(name).map_err(|e| anyhow::anyhow!("[grants.{name}]: {e}"))?;
        let functions: Vec<&str> = g.functions.iter().map(String::as_str).collect();
        grants.push(Grant::new(name, &g.allows, &functions, g.cost));
    }
    for c in &toml.kit.commands {
        anyhow::ensure!(grants.iter().any(|g| g.functions.contains(c)), "commands: no grant allows {c}");
    }
    for g in &toml.capabilities.grant {
        anyhow::ensure!(g != KV_READ && g != KV_WRITE, "[capabilities] {g}: kits get no kv; a kit keeps its own data, in its directory");
    }
    for key in toml.config_env.keys() {
        anyhow::ensure!(!toml.config.contains_key(key), "{key} is in [config] and [config-env]: give it once");
    }
    let About { id, name, version, owner, group, wasm, wit, commands, fake } = toml.kit;
    let offers = Offers { grants, commands, wit, fake, config: toml.config, config_env: toml.config_env };
    Ok(Manifest { app: AppInfo { id, name, version, owner, group, wasm }, capabilities: toml.capabilities, limits: toml.limits, kit: Some(offers) })
}

/// A version of a Wasm kit, loaded (runtime::load): compiled, linked to
/// what it may call.
pub(crate) struct KitVersion {
    pub id: String,
    pub wit: Vec<(String, String)>,
    pub grants: Vec<Grant>,
    /// The grants it calls other kits with.
    pub held: Vec<String>,
    pub commands: Vec<String>,
    /// Interfaces with their package's version: "acme:crm/customers" →
    /// "acme:crm/customers@0.1.0".
    versions: HashMap<String, String>,
    /// The functions it exports, with their types, by
    /// "acme:crm/customers#get".
    pub exports: HashMap<String, ComponentFunc>,
    pub sandbox: KitSandbox,
    pub fake: Option<Vec<u8>>,
}

impl KitVersion {
    /// "acme:crm/customers@0.1.0" of "acme:crm/customers".
    fn versioned(&self, interface: &str) -> String {
        self.versions.get(interface).cloned().unwrap_or_else(|| interface.to_string())
    }

    /// Whether it offers this interface ("acme:crm/leads", no version).
    fn offers(&self, interface: &str) -> bool {
        self.grants.iter().flat_map(|g| &g.functions).any(|f| f.split('#').next() == Some(interface))
    }

    fn grant_of(&self, function: &str) -> Option<&Grant> {
        self.grants.iter().find(|g| g.functions.iter().any(|f| f == function))
    }

    /// Whether what `uses` (an app's or a kit's imports) still fits this
    /// version, which replaces `old`: each function of its interfaces
    /// that it uses is there, with the same types, under the same grant,
    /// a command or not as before.
    pub fn fits(&self, old: Option<&KitVersion>, uses: &HashMap<String, Use>) -> Result<(), String> {
        for (function, used) in uses {
            let interface = function.split('#').next().unwrap_or(function);
            if !self.offers(interface) && !old.is_some_and(|o| o.offers(interface)) {
                continue;
            }
            let short = function.rsplit('/').next().unwrap_or(function).replace('#', "::");
            let (Some(ty), Some(grant)) = (self.exports.get(function), self.grant_of(function)) else {
                return Err(format!("{short} is gone"));
            };
            if !same(&used.ty, ty) {
                return Err(format!("{short} has other types"));
            }
            if grant.name != used.grant {
                return Err(format!("{short} needs {} now, not {}", grant.name, used.grant));
            }
            match (self.commands.contains(function), used.command) {
                (true, false) => return Err(format!("{short} is a command now")),
                (false, true) => return Err(format!("{short} is no command now")),
                _ => {}
            }
        }
        Ok(())
    }
}

/// The live version of a kit, for everything linked to it: none while no
/// version is loaded.
pub(crate) type Slot = Arc<RwLock<Option<Arc<KitVersion>>>>;

/// A kit in Wasm, as the kits know it: what it offers, and the slot with
/// its live version.
pub struct WasmKit {
    id: String,
    wit: Vec<(String, String)>,
    grants: Vec<Grant>,
    held: Vec<String>,
    commands: Vec<String>,
    fake: Option<Vec<u8>>,
    slot: Slot,
}

impl WasmKit {
    /// `version`, which goes live in `slot`.
    pub(crate) fn of(version: &KitVersion, slot: Slot) -> WasmKit {
        WasmKit {
            id: version.id.clone(),
            wit: version.wit.clone(),
            grants: version.grants.clone(),
            held: version.held.clone(),
            commands: version.commands.clone(),
            fake: version.fake.clone(),
            slot,
        }
    }

    /// The kit in `dir` (its kit.toml and the files it names), not loaded:
    /// what it offers, for the contract apps build against.
    pub fn read(dir: &Path) -> anyhow::Result<WasmKit> {
        let file = dir.join(crate::manifest::KIT_TOML);
        let bundle = Bundle::from_dir(dir).with_context(|| file.display().to_string())?;
        let manifest = bundle.manifest().with_context(|| file.display().to_string())?;
        let offers = manifest.kit.context("no kit")?;
        let KitFiles { wit, fake } = bundle.kit.unwrap_or_default();
        Ok(WasmKit {
            id: manifest.app.id,
            wit,
            grants: offers.grants,
            held: manifest.capabilities.grant,
            commands: offers.commands,
            fake,
            slot: Slot::default(),
        })
    }

    /// Its id: its name in policy.csv.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Checks what a kit's component exports: every function its grants
/// allow, taking and giving values only. Returns them with their types.
pub(crate) fn check_exports(engine: &wasmtime::Engine, component: &Component, offers: &Offers) -> anyhow::Result<HashMap<String, ComponentFunc>> {
    let mut exports = HashMap::new();
    for (name, item) in component.component_type().exports(engine) {
        let ComponentItem::ComponentInstance(i) = item.ty else { continue };
        let interface = name.split('@').next().unwrap_or(name);
        for (f, item) in i.exports(engine) {
            if let ComponentItem::ComponentFunc(func) = item.ty {
                exports.insert(format!("{interface}#{f}"), func);
            }
        }
    }
    let mut problems = vec![];
    for function in offers.grants.iter().flat_map(|g| &g.functions) {
        let short = function.rsplit('/').next().unwrap_or(function).replace('#', "::");
        match exports.get(function) {
            None => problems.push(format!("exports no {short}, which a grant allows")),
            Some(func) if !func.params().all(|(_, t)| value(&t)) || !func.results().all(|t| value(&t)) => {
                problems.push(format!("{short} takes or gives more than values: no resources, streams or futures"));
            }
            Some(_) => {}
        }
    }
    if !problems.is_empty() {
        bail!("{}", problems.join("; "));
    }
    Ok(exports)
}

/// A loaded kit's version: `pre` is its component, linked; its directory
/// is data/kits/<id>, made here; its [config-env] comes from the shell's
/// environment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn version(
    manifest: &Manifest,
    offers: &Offers,
    bundle: &Bundle,
    label: &str,
    exports: HashMap<String, ComponentFunc>,
    pre: InstancePre<Ctx>,
    db: crate::tx::Db,
    data: &Path,
) -> anyhow::Result<KitVersion> {
    let id = manifest.app.id.clone();
    let files = bundle.kit.clone().unwrap_or_default();
    let mut versions = HashMap::new();
    for (name, text) in &files.wit {
        let (package, version, interfaces) = crate::kit::read_wit(name, text)?;
        for i in interfaces {
            let versioned = version.as_ref().map_or_else(|| format!("{package}/{i}"), |v| format!("{package}/{i}@{v}"));
            versions.insert(format!("{package}/{i}"), versioned);
        }
    }
    let env = config(&offers.config, &offers.config_env, &|var| std::env::var(var).ok())?;
    let dir = data.join("kits").join(&id);
    std::fs::create_dir_all(&dir).with_context(|| format!("its directory, {}", dir.display()))?;
    let held = manifest.capabilities.grant.clone();
    let sandbox = KitSandbox { pre, name: id.clone(), label: label.to_string(), held: held.clone(), db, dir, env: env.into_iter().collect() };
    Ok(KitVersion { id, wit: files.wit, grants: offers.grants.clone(), held, commands: offers.commands.clone(), versions, exports, sandbox, fake: files.fake })
}

/// A sign-in kit: its component, the hosts it may fetch from and its
/// configuration.
pub(crate) struct SignIn {
    pub id: String,
    pub wasm: Vec<u8>,
    pub hosts: Vec<String>,
    pub config: BTreeMap<String, String>,
}

/// Reads the sign-in kit in `dir`; `env` gives `[config-env]`.
pub(crate) fn sign_in(dir: &Path, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<SignIn> {
    let file = dir.join(crate::manifest::KIT_TOML);
    let toml: KitToml = toml::from_str(&read_text(&file)?).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    crate::manifest::validate_app_id(&toml.kit.id).map_err(|e| anyhow::anyhow!("{}: id: {e}", file.display()))?;
    let at = |e: anyhow::Error| e.context(file.display().to_string());
    let Some(sign_in) = &toml.sign_in else { bail!("{}: no [sign-in]: not a sign-in kit", file.display()) };
    let offers = !toml.kit.wit.is_empty() || !toml.kit.commands.is_empty() || !toml.grants.is_empty() || !toml.capabilities.grant.is_empty();
    anyhow::ensure!(!offers, "{}: a sign-in kit offers apps nothing and calls no kits: no wit, commands, grants or capabilities", file.display());
    let path = dir.join(&toml.kit.wasm);
    let wasm = std::fs::read(&path).with_context(|| format!("wasm = {:?}: {}", toml.kit.wasm, path.display())).map_err(at)?;
    Ok(SignIn { hosts: sign_in.hosts.clone(), config: config(&toml.config, &toml.config_env, env).map_err(at)?, wasm, id: toml.kit.id })
}

/// [config] and [config-env], read from `env`.
fn config(
    config: &BTreeMap<String, String>,
    config_env: &BTreeMap<String, String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<BTreeMap<String, String>> {
    let mut config = config.clone();
    for (key, var) in config_env {
        anyhow::ensure!(!config.contains_key(key), "{key} is in [config] and [config-env]: give it once");
        match env(var).filter(|s| !s.is_empty()) {
            Some(s) => config.insert(key.clone(), s),
            None => bail!("[config-env] {key} = {var:?}, but {var} is not set"),
        };
    }
    Ok(config)
}

fn read_text(path: &Path) -> anyhow::Result<String> {
    let size = std::fs::metadata(path).with_context(|| path.display().to_string())?.len();
    anyhow::ensure!(size <= MAX_TEXT, "{}: more than {MAX_TEXT} bytes", path.display());
    std::fs::read_to_string(path).with_context(|| path.display().to_string())
}

/// Whether a type is a value, all the way down.
fn value(t: &Type) -> bool {
    match t {
        Type::Own(_) | Type::Borrow(_) | Type::Future(_) | Type::Stream(_) | Type::ErrorContext => false,
        Type::List(l) => value(&l.ty()),
        Type::FixedLengthList(l) => value(&l.ty()),
        Type::Map(m) => value(&m.key()) && value(&m.value()),
        Type::Record(r) => r.fields().all(|f| value(&f.ty)),
        Type::Tuple(t) => t.types().all(|t| value(&t)),
        Type::Variant(v) => v.cases().all(|c| c.ty.as_ref().is_none_or(value)),
        Type::Option(o) => value(&o.ty()),
        Type::Result(r) => r.ok().as_ref().is_none_or(value) && r.err().as_ref().is_none_or(value),
        _ => true,
    }
}

/// Whether the app imports a function with the types the kit exports it
/// with.
fn same(a: &ComponentFunc, b: &ComponentFunc) -> bool {
    a.params().map(|(_, t)| t).eq(b.params().map(|(_, t)| t)) && a.results().eq(b.results())
}

/// The live version, or why there is none.
fn live(slot: &Slot, id: &str) -> wasmtime::Result<Arc<KitVersion>> {
    slot.read().clone().ok_or_else(|| wasmtime::format_err!("kit {id} is not loaded"))
}

impl Kit for WasmKit {
    fn wit(&self) -> Vec<(String, String)> {
        self.wit.clone()
    }

    fn grants(&self) -> Vec<Grant> {
        self.grants.clone()
    }

    fn caller(&self) -> Option<Caller> {
        Some(Caller { name: self.id.clone(), grants: self.held.clone() })
    }

    fn up(&self) -> bool {
        self.slot.read().is_some()
    }

    fn fake(&self) -> Option<Vec<u8>> {
        self.fake.clone()
    }

    fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
        let now = self.slot.read().clone();
        for function in self.grants.iter().flat_map(|g| &g.functions) {
            let Some((interface, name)) = function.split_once('#') else { continue };
            let versioned = now.as_ref().map_or_else(|| interface.to_string(), |v| v.versioned(interface));
            if let (Some(app), Some(kit)) = (linker.uses(interface, name), now.as_ref().and_then(|v| v.exports.get(function)))
                && !same(app, kit)
            {
                let short = interface.rsplit('/').next().unwrap_or(interface);
                bail!("{short}::{name} has other types than kit {} gives it: build against the platform's WIT (appos sync)", self.id);
            }
            if self.commands.contains(function) {
                linker.command_dynamic(&versioned, name)?;
                continue;
            }
            let (slot, id, interface, name) = (self.slot.clone(), self.id.clone(), interface.to_string(), name.to_string());
            linker.func_dynamic(&versioned, &name.clone(), move |mut store: StoreContextMut<'_, Ctx>, params, results| {
                let (slot, id, interface, name) = (slot.clone(), id.clone(), interface.clone(), name.clone());
                Box::pin(async move {
                    let kit = live(&slot, &id)?;
                    let (cx, mut left) = store.data().kit_call()?;
                    left.fuel = store.get_fuel()?;
                    let (cx, access) = cx.to_kit(&kit.sandbox.name, &kit.sandbox.held);
                    let called = call_kit(&kit.sandbox, cx, access, left, (&kit.versioned(&interface), &name), params, results).await;
                    store.set_fuel(called.left.fuel)?;
                    store.data_mut().kit_called(&called);
                    called.result
                })
            })?;
        }
        Ok(())
    }

    fn run(&self, cx: Context, msg: Message) -> Run<'_> {
        Box::pin(async move {
            let kit = live(&self.slot, &self.id).map_err(|e| anyhow::anyhow!("{e}"))?;
            let (interface, name) = msg.function.split_once('#').with_context(|| format!("no command {}", msg.function))?;
            anyhow::ensure!(kit.commands.contains(&msg.function), "no command {}", msg.function);
            let func = kit.exports.get(&msg.function).with_context(|| format!("no command {}", msg.function))?;
            let types: Vec<Type> = func.params().map(|(_, t)| t).collect();
            let args: Vec<Json> = serde_json::from_str(msg.raw_args())?;
            anyhow::ensure!(args.len() == types.len(), "{}: {} arguments, not {}", msg.function, args.len(), types.len());
            let params = args.iter().zip(&types).map(|(a, t)| from_json(a, t)).collect::<anyhow::Result<Vec<_>>>()?;
            let (cx, access) = cx.to_kit(&kit.sandbox.name, &kit.sandbox.held);
            let left = Left { fuel: FUEL, io: 0, depth: 1 };
            let Called { result, denied, .. } = call_kit(&kit.sandbox, cx, access, left, (&kit.versioned(interface), name), &params, &mut []).await;
            if let Some(grant) = denied {
                bail!("denied {grant}");
            }
            result.map_err(|e| anyhow::anyhow!("{e:?}"))
        })
    }
}

/// A value as JSON, for a command's arguments: records are objects,
/// variants and results objects of one case ({"ok": 1}), a case without a
/// value and an enum a string, flags a list of strings, none null. Some(x)
/// is x, but {"some": x} when x is an option itself.
pub(crate) fn to_json(v: &Val) -> wasmtime::Result<Json> {
    let list = |vs: &[Val]| vs.iter().map(to_json).collect::<wasmtime::Result<Vec<_>>>().map(Json::Array);
    let case = |name: &str, v: &Option<Box<Val>>| -> wasmtime::Result<Json> {
        Ok(match v {
            Some(v) => json!({ name: to_json(v)? }),
            None => json!({ name: null }),
        })
    };
    Ok(match v {
        Val::Bool(b) => json!(b),
        Val::S8(n) => json!(n),
        Val::U8(n) => json!(n),
        Val::S16(n) => json!(n),
        Val::U16(n) => json!(n),
        Val::S32(n) => json!(n),
        Val::U32(n) => json!(n),
        Val::S64(n) => json!(n),
        Val::U64(n) => json!(n),
        Val::Float32(n) => serde_json::Number::from_f64(f64::from(*n)).map(Json::Number).ok_or_else(|| wasmtime::format_err!("{n} is no JSON number"))?,
        Val::Float64(n) => serde_json::Number::from_f64(*n).map(Json::Number).ok_or_else(|| wasmtime::format_err!("{n} is no JSON number"))?,
        Val::Char(c) => json!(c.to_string()),
        Val::String(s) => json!(s),
        Val::List(vs) | Val::Tuple(vs) | Val::FixedLengthList(vs) => list(vs)?,
        Val::Map(pairs) => Json::Array(pairs.iter().map(|(k, v)| Ok(json!([to_json(k)?, to_json(v)?]))).collect::<wasmtime::Result<_>>()?),
        Val::Record(fields) => Json::Object(fields.iter().map(|(k, v)| Ok((k.clone(), to_json(v)?))).collect::<wasmtime::Result<_>>()?),
        Val::Variant(name, None) => json!(name),
        Val::Variant(name, v) => case(name, v)?,
        Val::Enum(name) => json!(name),
        Val::Option(None) => Json::Null,
        Val::Option(Some(v)) if matches!(**v, Val::Option(_)) => json!({ "some": to_json(v)? }),
        Val::Option(Some(v)) => to_json(v)?,
        Val::Result(Ok(v)) => case("ok", v)?,
        Val::Result(Err(v)) => case("err", v)?,
        Val::Flags(names) => json!(names),
        other => return Err(wasmtime::format_err!("a command takes values, not {other:?}")),
    })
}

/// A value of type `t` from `to_json`'s JSON.
fn from_json(j: &Json, t: &Type) -> anyhow::Result<Val> {
    let wrong = || anyhow::anyhow!("{j} is no {}", describe(t));
    let int = || j.as_i64().ok_or_else(wrong);
    let uint = || j.as_u64().ok_or_else(wrong);
    let array = || j.as_array().ok_or_else(wrong);
    let one_case = |j: &Json| -> anyhow::Result<(String, Json)> {
        let object = j.as_object().filter(|o| o.len() == 1).ok_or_else(wrong)?;
        let (k, v) = object.iter().next().ok_or_else(wrong)?;
        Ok((k.clone(), v.clone()))
    };
    let payload = |ty: Option<Type>, v: &Json| -> anyhow::Result<Option<Box<Val>>> {
        match ty {
            Some(ty) => Ok(Some(Box::new(from_json(v, &ty)?))),
            None if v.is_null() => Ok(None),
            None => Err(wrong()),
        }
    };
    Ok(match t {
        Type::Bool => Val::Bool(j.as_bool().ok_or_else(wrong)?),
        Type::S8 => Val::S8(int()?.try_into()?),
        Type::U8 => Val::U8(uint()?.try_into()?),
        Type::S16 => Val::S16(int()?.try_into()?),
        Type::U16 => Val::U16(uint()?.try_into()?),
        Type::S32 => Val::S32(int()?.try_into()?),
        Type::U32 => Val::U32(uint()?.try_into()?),
        Type::S64 => Val::S64(int()?),
        Type::U64 => Val::U64(uint()?),
        Type::Float32 => Val::Float32(j.as_f64().ok_or_else(wrong)? as f32),
        Type::Float64 => Val::Float64(j.as_f64().ok_or_else(wrong)?),
        Type::Char => {
            let s = j.as_str().ok_or_else(wrong)?;
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Val::Char(c),
                _ => bail!(wrong()),
            }
        }
        Type::String => Val::String(j.as_str().ok_or_else(wrong)?.to_string()),
        Type::List(l) => Val::List(array()?.iter().map(|v| from_json(v, &l.ty())).collect::<anyhow::Result<_>>()?),
        Type::FixedLengthList(l) => Val::FixedLengthList(array()?.iter().map(|v| from_json(v, &l.ty())).collect::<anyhow::Result<_>>()?),
        Type::Map(m) => Val::Map(
            array()?
                .iter()
                .map(|pair| match pair.as_array().map(Vec::as_slice) {
                    Some([k, v]) => Ok((from_json(k, &m.key())?, from_json(v, &m.value())?)),
                    _ => Err(wrong()),
                })
                .collect::<anyhow::Result<_>>()?,
        ),
        Type::Tuple(tt) => {
            let items = array()?;
            anyhow::ensure!(items.len() == tt.types().len(), wrong());
            Val::Tuple(items.iter().zip(tt.types()).map(|(v, t)| from_json(v, &t)).collect::<anyhow::Result<_>>()?)
        }
        Type::Record(r) => {
            let object = j.as_object().ok_or_else(wrong)?;
            let fields = r.fields().map(|f| Ok((f.name.to_string(), from_json(object.get(f.name).unwrap_or(&Json::Null), &f.ty)?)));
            Val::Record(fields.collect::<anyhow::Result<_>>()?)
        }
        Type::Variant(v) => {
            let (name, value) = match j {
                Json::String(s) => (s.clone(), Json::Null),
                _ => one_case(j)?,
            };
            let case = v.cases().find(|c| c.name == name).ok_or_else(wrong)?;
            Val::Variant(name, payload(case.ty, &value)?)
        }
        Type::Enum(e) => {
            let name = j.as_str().ok_or_else(wrong)?;
            anyhow::ensure!(e.names().any(|n| n == name), wrong());
            Val::Enum(name.to_string())
        }
        Type::Option(o) => match (j, o.ty()) {
            (Json::Null, _) => Val::Option(None),
            (_, inner @ Type::Option(_)) => {
                let (name, value) = one_case(j)?;
                anyhow::ensure!(name == "some", wrong());
                Val::Option(Some(Box::new(from_json(&value, &inner)?)))
            }
            (_, inner) => Val::Option(Some(Box::new(from_json(j, &inner)?))),
        },
        Type::Result(r) => {
            let (name, value) = one_case(j)?;
            match name.as_str() {
                "ok" => Val::Result(Ok(payload(r.ok(), &value)?)),
                "err" => Val::Result(Err(payload(r.err(), &value)?)),
                _ => bail!(wrong()),
            }
        }
        Type::Flags(f) => {
            let names: Vec<String> = array()?.iter().map(|n| n.as_str().map(str::to_string).ok_or_else(wrong)).collect::<anyhow::Result<_>>()?;
            anyhow::ensure!(names.iter().all(|n| f.names().any(|f| f == n)), wrong());
            Val::Flags(names)
        }
        _ => bail!("a command takes values, not {}", describe(t)),
    })
}

fn describe(t: &Type) -> &'static str {
    match t {
        Type::Bool => "bool",
        Type::S8 | Type::U8 | Type::S16 | Type::U16 | Type::S32 | Type::U32 | Type::S64 | Type::U64 => "integer of its size",
        Type::Float32 | Type::Float64 => "number",
        Type::Char => "char",
        Type::String => "string",
        Type::List(_) | Type::FixedLengthList(_) => "list",
        Type::Map(_) => "map",
        Type::Record(_) => "record",
        Type::Tuple(_) => "tuple",
        Type::Variant(_) => "variant",
        Type::Enum(_) => "enum",
        Type::Option(_) => "option",
        Type::Result(_) => "result",
        Type::Flags(_) => "flags",
        _ => "value",
    }
}

/// The fakes of the kits, as files below a directory: <name>/kit.toml,
/// <name>/kit.wasm and <name>/wit/*.wit. `appos dev` loads each as a
/// Wasm kit with no grants of its own: made-up data calls no one. A Wasm
/// kit's fake has its kit's id.
pub(crate) fn fake_files(kits: &Kits, engine: &wasmtime::Engine) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for Fake { name, wit, grants, commands, wasm } in kits.fakes(engine)? {
        let mut toml = toml::Table::new();
        let mut kit = toml::Table::new();
        kit.insert("id".into(), name.clone().into());
        kit.insert("name".into(), format!("{name}, a fake").into());
        kit.insert("version".into(), "0.0.0".into());
        kit.insert("owner".into(), "appos sync".into());
        kit.insert("group".into(), "Fakes".into());
        kit.insert("wasm".into(), "kit.wasm".into());
        kit.insert("wit".into(), wit.iter().map(|(f, _)| toml::Value::from(format!("wit/{f}"))).collect::<Vec<_>>().into());
        kit.insert("commands".into(), commands.iter().map(|c| toml::Value::from(c.as_str())).collect::<Vec<_>>().into());
        toml.insert("kit".into(), kit.into());
        let mut table = toml::Table::new();
        for g in grants {
            let mut grant = toml::Table::new();
            grant.insert("allows".into(), g.allows.into());
            grant.insert("functions".into(), g.functions.iter().map(|f| toml::Value::from(f.as_str())).collect::<Vec<_>>().into());
            grant.insert("cost".into(), i64::from(g.cost).into());
            table.insert(g.name, grant.into());
        }
        toml.insert("grants".into(), table.into());
        let text = format!("# The fake of a kit, for appos dev: written by appos sync.\n{}", toml::to_string(&toml)?);
        files.insert(format!("{name}/kit.toml"), text.into_bytes());
        files.insert(format!("{name}/kit.wasm"), wasm);
        for (file, text) in wit {
            files.insert(format!("{name}/wit/{file}"), text.into_bytes());
        }
    }
    Ok(files)
}

/// Writes the fakes into `dir`, replacing what it held.
pub(crate) fn write_fakes(kits: &Kits, engine: &wasmtime::Engine, dir: &Path) -> anyhow::Result<usize> {
    let files = fake_files(kits, engine)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    for (path, bytes) in &files {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes).with_context(|| path.display().to_string())?;
    }
    Ok(files.keys().filter(|p| p.ends_with("/kit.toml")).count())
}

/// Loads the fakes as `appos dev` does, with the built-in kits, in a
/// directory that goes again after; answers their names. On a thread of
/// its own, so it works in a Tokio runtime and outside one.
pub(crate) fn check_fakes(kits: &Kits) -> anyhow::Result<Vec<String>> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("appos-fakes-{}-{nanos}", std::process::id()));
    let engine = crate::runtime::engine()?;
    let checked = std::thread::scope(|s| {
        s.spawn(|| {
            write_fakes(kits, &engine, &dir.join("fakes"))?;
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
            let live = LiveKits::new(Kits::builtin());
            let loaded = runtime.block_on(load_dirs(&engine, &crate::tx::in_memory(), &live, &[dir.join("fakes")], &dir.join("data")))?;
            let mut names: Vec<String> = loaded.iter().map(|k| k.manifest.app.id.clone()).collect();
            names.sort();
            anyhow::Ok(names)
        })
        .join()
        .unwrap_or_else(|_| Err(anyhow::anyhow!("checking the fakes panicked")))
    });
    let _ = std::fs::remove_dir_all(&dir);
    checked
}

/// The kits' directories in `dirs`: each a kit's (with its kit.toml), or
/// holding kits' directories, as fakes/ does.
pub(crate) fn kit_dirs(dirs: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut kits = vec![];
    for dir in dirs {
        if dir.join(crate::manifest::KIT_TOML).is_file() {
            kits.push(dir.clone());
            continue;
        }
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir).with_context(|| dir.display().to_string())?.filter_map(|e| Some(e.ok()?.path())).collect();
        found.retain(|p| p.join(crate::manifest::KIT_TOML).is_file());
        found.sort();
        anyhow::ensure!(!found.is_empty(), "{}: no kit.toml", dir.display());
        kits.extend(found);
    }
    Ok(kits)
}

/// The kits in `dirs` (`kit_dirs`), loaded and made live in `kits` in
/// order: a kit loads once the kits whose grants it holds are live.
async fn load_dirs(
    engine: &wasmtime::Engine,
    db: &crate::tx::Db,
    kits: &LiveKits,
    dirs: &[PathBuf],
    data: &Path,
) -> anyhow::Result<Vec<Arc<crate::runtime::LoadedApp>>> {
    let mut checked = vec![];
    for dir in kit_dirs(dirs)? {
        let bundle = Bundle::from_dir(&dir).with_context(|| format!("kit {}", dir.display()))?;
        checked.push((dir, bundle.checked()?));
    }
    let mut loaded = vec![];
    while !checked.is_empty() {
        let now = kits.now();
        let Some(n) = checked.iter().position(|(_, c)| c.manifest.capabilities.grant.iter().all(|g| now.grant(g).is_some())) else {
            let (dir, c) = &checked[0];
            let missing: Vec<&str> = c.manifest.capabilities.grant.iter().filter(|g| now.grant(g).is_none()).map(String::as_str).collect();
            bail!("kit {} ({}): no kit offers {}", c.manifest.app.id, dir.display(), missing.join(", "));
        };
        let (dir, c) = checked.remove(n);
        let id = c.manifest.app.id.clone();
        let app = crate::runtime::load(engine, db.clone(), &now, &c, data).await.with_context(|| format!("kit {id} ({})", dir.display()))?;
        let version = app.kit.clone().context("no kit")?;
        let with = kits.with(&version).with_context(|| format!("kit {id} ({})", dir.display()))?;
        kits.put(version, with);
        loaded.push(Arc::new(app));
    }
    Ok(loaded)
}

/// Adds the kits in `dirs` to `kits`, not loaded: what they offer, for
/// the contract.
pub(crate) fn add_dirs(mut kits: Kits, dirs: &[PathBuf]) -> anyhow::Result<Kits> {
    for dir in dirs {
        let kit = WasmKit::read(dir)?;
        let id = kit.id().to_string();
        kits = kits.with_wasm(kit).with_context(|| format!("kit {id} ({})", dir.display()))?;
    }
    Ok(kits)
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;

    use super::*;
    use crate::access::{Access, Policies};
    use crate::bundle::Bundle;
    use crate::kit::User;
    use crate::platform::Outcome;
    use crate::runtime::{self, AdminLimits, Budget, Request};
    use crate::state::Live;

    const KIT: &str = include_str!("testing-wasm-kit.wat");
    const CRM_WIT: &str = "package acme:crm@1.0.0;\ninterface types { record lead { source: string } }\ninterface leads { count-leads: func(source: string) -> u32; add-lead: func(source: string); }\n";
    const KIT_TOML: &str = "[kit]\nid = \"crm-wasm\"\nname = \"CRM\"\nversion = \"1.0.0\"\nwit = [\"crm.wit\"]\ncommands = [\"acme:crm/leads#add-lead\"]\n\
        [grants.leads-read]\nallows = \"Count leads\"\nfunctions = [\"acme:crm/leads#count-leads\"]\ncost = 2\n\
        [grants.leads-write]\nallows = \"Add leads\"\nfunctions = [\"acme:crm/leads#add-lead\"]\ncost = 5\n\
        [capabilities]\ngrant = [\"bonus-read\"]\n";

    /// acme:bonus in Rust: bonus() answers its number, note(text) keeps
    /// who noted what (the caller and the user).
    #[derive(Clone, Default)]
    struct Bonus {
        bonus: u32,
        notes: Arc<Mutex<Vec<Note>>>,
    }

    /// A note: the caller, the user, the text.
    type Note = (String, Option<String>, String);

    impl Kit for Bonus {
        fn wit(&self) -> Vec<(String, String)> {
            vec![("bonus.wit".into(), "package acme:bonus@1.0.0;\ninterface b { bonus: func() -> u32; note: func(text: string); }\n".into())]
        }
        fn grants(&self) -> Vec<Grant> {
            vec![Grant::new("bonus-read", "Bonus: b::bonus, note", &["acme:bonus/b#bonus", "acme:bonus/b#note"], 1)]
        }
        fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
            let bonus = self.bonus;
            linker.func("acme:bonus/b@1.0.0", "bonus", move |_, ()| Box::pin(async move { Ok((bonus,)) }))?;
            let notes = self.notes.clone();
            linker.func("acme:bonus/b@1.0.0", "note", move |cx, (text,): (String,)| {
                notes.lock().push((cx.app().into(), cx.user().map(|u| u.email.clone()), text));
                Box::pin(async move { Ok(()) })
            })
        }
        fn run(&self, _: Context, msg: Message) -> Run<'_> {
            Box::pin(async move { bail!("no command {}", msg.function) })
        }
    }

    /// The platform: acme:bonus in Rust, acme:crm in Wasm (`kit` is its
    /// WAT, `toml` its kit.toml), loaded with its directory below `dir`.
    async fn platform(bonus: &Bonus, kit: &str, toml: &str, dir: &Path, db: &crate::tx::Db) -> anyhow::Result<Kits> {
        let at = dir.join("crm");
        std::fs::create_dir_all(&at)?;
        std::fs::write(at.join("kit.toml"), toml)?;
        std::fs::write(at.join("kit.wasm"), kit)?;
        std::fs::write(at.join("crm.wit"), CRM_WIT)?;
        let live = LiveKits::new(Kits::builtin().with(bonus.clone())?);
        load_dirs(&crate::runtime::engine()?, db, &live, &[at], &dir.join("data")).await?;
        Ok(live.now())
    }

    async fn app(kits: &Kits, grants: &str, wat: &[u8], db: crate::tx::Db) -> anyhow::Result<runtime::LoadedApp> {
        let bundle = Bundle {
            manifest: format!("[app]\nid = 'test'\nname = 'Test'\nversion = '1'\nowner = 'Tests'\ngroup = 'Tests'\n[capabilities]\ngrant = [{grants}]\n"),
            wasm: wat.to_vec(),
            icon: None,
            deps: None,
            kit: None,
        };
        runtime::load(&runtime::engine()?, db, kits, &bundle.checked()?, &std::env::temp_dir()).await
    }

    async fn run(app: &runtime::LoadedApp, handler: &str, user: Option<User>, live: Live) -> (Result<runtime::Page, runtime::Failed>, runtime::Cost) {
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
        runtime::call(app, handler, &req, Budget::of(&app.manifest, AdminLimits::default()), Access::ALL, user, live).await
    }

    fn ann() -> User {
        User { id: "ann".into(), email: "ann@acme.com".into(), name: "Ann".into(), groups: vec!["sales".into()] }
    }

    #[tokio::test]
    async fn an_app_calls_a_wasm_kit_which_calls_a_rust_kit() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let bonus = Bonus { bonus: 2, ..Bonus::default() };
        let kits = platform(&bonus, KIT, KIT_TOML, dir.path(), &db).await.unwrap();
        assert!(dir.path().join("data/kits/crm-wasm").is_dir(), "a directory of its own");
        let app = app(&kits, "'leads-read'", include_bytes!("testing-kit.wat"), db).await.unwrap();
        let (page, cost) = run(&app, "read", None, Live::default()).await;
        assert_eq!(page.unwrap_or_else(|f| panic!("{:#}", f.error)).html, [("page".to_string(), "seven".to_string())]);
        assert_eq!(cost.io, 2 + 1, "count-leads and the kit's bonus: one budget");
    }

    #[tokio::test]
    async fn a_wasm_kit_runs_on_its_callers_budget() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let kits = platform(&Bonus::default(), KIT, KIT_TOML, dir.path(), &db).await.unwrap();
        let app = app(&kits, "'leads-read'", include_bytes!("testing-kit.wat"), db).await.unwrap();
        let (page, cost) = run(&app, "read", None, Live::default()).await;
        let failed = page.expect_err("the kit spins");
        assert!(failed.kind == Outcome::OverBudget, "{:?}: {:#}", failed.kind, failed.error);
        assert!(cost.fuel >= runtime::FUEL, "the kit used the app's fuel: {}", cost.fuel);
    }

    #[tokio::test]
    async fn the_policy_gives_a_wasm_kit_its_grants() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let bonus = Bonus { bonus: 2, ..Bonus::default() };
        let kits = platform(&bonus, KIT, KIT_TOML, dir.path(), &db).await.unwrap();
        let app = app(&kits, "'leads-read'", include_bytes!("testing-kit.wat"), db).await.unwrap();
        let live = |rules: &'static str| {
            let grants = kits.names();
            async move { Live::with_policy(Arc::new(Policies::built_in(rules, grants).await.unwrap())) }
        };
        let (page, _) = run(&app, "read", None, live("p, anyone, crm-wasm, bonus-read").await).await;
        assert!(page.is_ok(), "the kit may use bonus-read");
        let (page, _) = run(&app, "read", None, live("p, anyone, test, leads-read").await).await;
        let failed = page.expect_err("the kit may not");
        assert!(failed.kind == Outcome::Denied, "{:#}", failed.error);
        assert_eq!(failed.error.to_string(), "denied bonus-read");
    }

    #[tokio::test]
    async fn commands_to_a_wasm_kit_run_after_the_commit() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let bonus = Bonus { bonus: 2, ..Bonus::default() };
        let kits = platform(&bonus, KIT, KIT_TOML, dir.path(), &db).await.unwrap();
        let app = app(&kits, "'leads-write'", include_bytes!("testing-command.wat"), db.clone()).await.unwrap();
        let (page, cost) = run(&app, "send", Some(ann()), Live::default()).await;
        assert!(page.is_ok() && cost.sent == 2);
        let sent = crate::tx::outbox(&db).unwrap();
        assert_eq!(sent.iter().map(|m| m.args.as_str()).collect::<Vec<_>>(), [r#"["web"]"#, r#"["ads"]"#]);
        crate::outbox::start(db.clone(), crate::platform::PlatformDb::in_memory(), Live::of_kits(kits.clone()));
        for _ in 0..300 {
            if crate::tx::outbox(&db).unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(crate::tx::outbox(&db).unwrap().is_empty(), "{:?}", crate::tx::outbox(&db).unwrap().first().map(|m| m.error.clone()));
        // The kit calls acme:bonus as itself, for whom the app sent it.
        let noted = |s: &str| ("crm-wasm".to_string(), Some("ann@acme.com".to_string()), s.to_string());
        assert_eq!(*bonus.notes.lock(), [noted("web"), noted("ads")]);
    }

    #[tokio::test]
    async fn an_app_and_a_wasm_kit_agree_on_the_types() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let kit = KIT.replace(r#"(func $count (param "source" string) (result u32)"#, r#"(func $count (param "source" string) (result s32)"#);
        let kits = platform(&Bonus::default(), &kit, KIT_TOML, dir.path(), &db).await.unwrap();
        let err = app(&kits, "'leads-read'", include_bytes!("testing-kit.wat"), db).await.err().unwrap();
        assert!(format!("{err:#}").contains("leads::count-leads has other types than kit crm-wasm gives it"), "{err:#}");
    }

    /// A fake of acme:crm/leads: count-leads answers 7, add-lead does
    /// nothing.
    const FAKE: &str = r#"(component
        (core module $m
            (memory (export "memory") 1)
            (func (export "realloc") (param i32 i32 i32 i32) (result i32) i32.const 64)
            (func (export "count") (param i32 i32) (result i32) i32.const 7)
            (func (export "add") (param i32 i32)))
        (core instance $i (instantiate $m))
        (alias core export $i "memory" (core memory $memory))
        (alias core export $i "realloc" (core func $realloc))
        (func $count (param "source" string) (result u32) (canon lift (core func $i "count") (memory $memory) (realloc $realloc)))
        (func $add (param "source" string) (canon lift (core func $i "add") (memory $memory) (realloc $realloc)))
        (instance $leads (export "count-leads" (func $count)) (export "add-lead" (func $add)))
        (export "acme:crm/leads@1.0.0" (instance $leads)))"#;

    /// acme:crm in Rust, with FAKE as its fake.
    struct RealCrm;

    impl Kit for RealCrm {
        fn wit(&self) -> Vec<(String, String)> {
            vec![("crm.wit".into(), CRM_WIT.into())]
        }
        fn grants(&self) -> Vec<Grant> {
            vec![
                Grant::new("leads-read", "Count leads", &["acme:crm/leads#count-leads"], 2),
                Grant::new("leads-write", "Add leads", &["acme:crm/leads#add-lead"], 5),
            ]
        }
        fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
            linker.func("acme:crm/leads@1.0.0", "count-leads", |_, (_,): (String,)| {
                Box::pin(async { Err::<(u32,), _>(wasmtime::format_err!("the real CRM is not here")) })
            })?;
            linker.command::<(String,)>("acme:crm/leads@1.0.0", "add-lead")
        }
        fn fake(&self) -> Option<Vec<u8>> {
            Some(FAKE.as_bytes().to_vec())
        }
    }

    #[tokio::test]
    async fn appos_dev_runs_a_kits_fake() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let kits = Kits::builtin().with(RealCrm).unwrap();
        assert_eq!(kits.check_fakes().unwrap(), ["acme-crm"]);
        assert_eq!(kits.write_fakes(&dir.path().join("app/fakes")).unwrap(), 1);
        let toml = std::fs::read_to_string(dir.path().join("app/fakes/acme-crm/kit.toml")).unwrap();
        assert!(toml.contains("commands = [\"acme:crm/leads#add-lead\"]"), "{toml}");
        let dev = crate::dev_kits(&dir.path().join("app")).unwrap();
        assert_eq!(dev.contract(), kits.contract(), "an app builds against the same WIT");
        let live = LiveKits::new(Kits::builtin());
        load_dirs(&runtime::engine().unwrap(), &db, &live, &[dir.path().join("app/fakes")], &dir.path().join("data")).await.unwrap();
        assert_eq!(live.now().contract(), kits.contract(), "loaded, the same");
        let app = app(&live.now(), "'leads-read'", include_bytes!("testing-kit.wat"), db).await.unwrap();
        let (page, _) = run(&app, "read", None, Live::default()).await;
        assert_eq!(page.unwrap_or_else(|f| panic!("{:#}", f.error)).html, [("page".to_string(), "seven".to_string())]);
    }

    #[tokio::test]
    async fn a_wasm_kit_is_checked_when_it_loads() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let fails = async |kit: &str, toml: &str| format!("{:#}", platform(&Bonus::default(), kit, toml, dir.path(), &db).await.err().unwrap());
        let ungranted = fails(KIT, &KIT_TOML.replace("grant = [\"bonus-read\"]", "grant = []")).await;
        assert!(ungranted.contains("uses b::bonus, which needs bonus-read (add it to [capabilities] grant in kit.toml)"), "{ungranted}");
        assert!(fails(KIT, &KIT_TOML.replace("\"bonus-read\"", "\"kv-read\"")).await.contains("kits get no kv"));
        assert!(fails(KIT, &KIT_TOML.replace("\"bonus-read\"", "\"nope\"")).await.contains("no kit offers nope"));
        let missing = fails(&KIT.replace("(export \"add-lead\" (func $add))", ""), KIT_TOML).await;
        assert!(missing.contains("exports no leads::add-lead, which a grant allows"), "{missing}");
        let kits = platform(&Bonus::default(), KIT, KIT_TOML, dir.path(), &db).await.unwrap();
        assert!(kits.taken().contains("crm-wasm"), "no app can be called crm-wasm");
        let err = app(&kits, "", include_bytes!("testing-kit.wat"), db.clone()).await.err().unwrap();
        assert!(format!("{err:#}").contains("uses leads::count-leads, which needs leads-read"), "{err:#}");
    }

    #[test]
    fn values_go_to_json_and_back() {
        let engine = crate::runtime::engine().unwrap();
        let wat = r#"(component
            (import "acme:t/i" (instance
                (type $kind (enum "lead" "customer"))
                (export "kind" (type $k (eq $kind)))
                (type $who (record (field "name" string) (field "kind" $k) (field "score" (option (option u32))) (field "tags" (list string))))
                (export "who" (type $w (eq $who)))
                (type $f (func (param "who" $w) (param "r" (result u32 (error string))) (param "t" (tuple char s64))))
                (export "f" (func (type $f)))
            ))
        )"#;
        let component = Component::new(&engine, wat).unwrap();
        let ty = component.component_type();
        let (_, item) = ty.imports(&engine).next().unwrap();
        let ComponentItem::ComponentInstance(i) = item.ty else { panic!() };
        let ComponentItem::ComponentFunc(f) = i.exports(&engine).find(|(n, _)| *n == "f").unwrap().1.ty else { panic!() };
        let types: Vec<Type> = f.params().map(|(_, t)| t).collect();
        let values = [
            Val::Record(vec![
                ("name".into(), Val::String("Nordwind".into())),
                ("kind".into(), Val::Enum("lead".into())),
                ("score".into(), Val::Option(Some(Box::new(Val::Option(None))))),
                ("tags".into(), Val::List(vec![Val::String("eu".into())])),
            ]),
            Val::Result(Err(Some(Box::new(Val::String("no".into()))))),
            Val::Tuple(vec![Val::Char('ä'), Val::S64(-3)]),
        ];
        let json: Vec<Json> = values.iter().map(|v| to_json(v).unwrap()).collect();
        assert_eq!(Json::Array(json.clone()).to_string(), r#"[{"kind":"lead","name":"Nordwind","score":{"some":null},"tags":["eu"]},{"err":"no"},["ä",-3]]"#);
        let back: Vec<Val> = json.iter().zip(&types).map(|(j, t)| from_json(j, t).unwrap()).collect();
        assert_eq!(back, values);
        assert!(from_json(&json!({"name": "x", "kind": "partner", "score": null, "tags": []}), &types[0]).unwrap_err().to_string().contains("is no enum"));
        assert!(from_json(&json!(["ab", 1]), &types[2]).is_err());
    }

    #[test]
    fn kit_toml_is_checked() {
        let base = "[kit]\nid = \"crm\"\nname = \"CRM\"\nversion = \"1.0.0\"\nwit = [\"crm.wit\"]\n";
        let env = |var: &str| (var == "SET").then(|| "s3cret".to_string());
        let fails = |toml: &str| Manifest::parse_kit(toml).err().map(|e| format!("{e:#}")).unwrap_or_default();
        let m =
            Manifest::parse_kit(&format!("{base}[grants.leads-read]\nallows = \"Count\"\nfunctions = [\"acme:crm/leads#count\"]\n[config-env]\nKEY = \"SET\""))
                .unwrap();
        let offers = m.kit.as_ref().unwrap();
        assert_eq!((m.app.id.as_str(), m.app.wasm.to_str(), offers.grants[0].cost), ("crm", Some("kit.wasm"), 1));
        assert_eq!(config(&offers.config, &offers.config_env, &env).unwrap(), BTreeMap::from([("KEY".to_string(), "s3cret".to_string())]));
        let unset = Manifest::parse_kit(&format!("{base}[config-env]\nKEY = \"UNSET\"")).unwrap();
        let unset = unset.kit.unwrap();
        assert!(config(&unset.config, &unset.config_env, &env).unwrap_err().to_string().contains("UNSET is not set"));
        assert!(fails(&base.replace("\"crm\"", "\"CRM!\"")).contains("id"));
        assert!(fails(&format!("{base}[kit2]\n")).contains("unknown field"));
        assert!(fails(&base.replace("wit = [", "commands = [\"acme:crm/leads#add\"]\nwit = [")).contains("no grant allows acme:crm/leads#add"));
        assert!(fails(&format!("{base}[sign-in]\nhosts = []\n")).contains("a sign-in kit"));
        assert!(fails(&base.replace("wit = [\"crm.wit\"]\n", "")).contains("wit = [...]"));
    }

    // Wasm kits deployed while the platform runs, as apps are.

    const CRM: &str = include_str!("testing-crm-kit.wat");
    const CRM_KIT: &str = "[kit]\nid = \"crm\"\nname = \"CRM\"\nversion = \"1.0.0\"\nowner = \"Sales Ops\"\ngroup = \"Sales\"\nwit = [\"crm.wit\"]\n\
        commands = [\"acme:crm/leads#add-lead\"]\n\
        [grants.leads-read]\nallows = \"Count leads\"\nfunctions = [\"acme:crm/leads#count-leads\"]\n\
        [grants.leads-write]\nallows = \"Add leads\"\nfunctions = [\"acme:crm/leads#add-lead\"]\n";
    const BOARD_KIT: &str = "[kit]\nid = \"board\"\nname = \"Board\"\nversion = \"1.0.0\"\nowner = \"Sales Ops\"\ngroup = \"Sales\"\nwit = [\"board.wit\"]\n\
        [grants.board-read]\nallows = \"Scores\"\nfunctions = [\"acme:board/score#score\"]\n[capabilities]\ngrant = [\"leads-read\"]\n";
    const BOARD_WIT: &str = "package acme:board@1.0.0;\ninterface score { score: func() -> u32; }\n";

    fn kit_zip(toml: &str, wat: &str, wit: (&str, &str)) -> Vec<u8> {
        let kit = Some(KitFiles { wit: vec![(wit.0.into(), wit.1.into())], fake: None });
        Bundle { manifest: toml.into(), wasm: wat.as_bytes().to_vec(), icon: None, deps: None, kit }.pack().unwrap()
    }

    /// The CRM kit, as `wat`.
    fn crm(wat: &str) -> Vec<u8> {
        kit_zip(CRM_KIT, wat, ("crm.wit", CRM_WIT))
    }

    /// The app "test", holding `grants`.
    fn app_zip(grants: &str, wat: &[u8]) -> Vec<u8> {
        let manifest = format!("[app]\nid = 'test'\nname = 'Test'\nversion = '1'\nowner = 'Tests'\ngroup = 'Tests'\n[capabilities]\ngrant = [{grants}]\n");
        Bundle { manifest, wasm: wat.to_vec(), icon: None, deps: None, kit: None }.pack().unwrap()
    }

    async fn deploy(shell: &crate::shell::Shell, zip: &[u8]) -> Result<(), String> {
        crate::deploy::deploy(shell, zip, crate::platform::Actor::Cli).await.map(|_| ()).map_err(|e| format!("{e:#}"))
    }

    async fn get(shell: &Arc<crate::shell::Shell>, path: &str) -> (u16, String) {
        use tower::ServiceExt;
        let request = axum::http::Request::builder().uri(path).body(axum::body::Body::empty()).unwrap();
        let response = crate::web::router(shell.clone()).oneshot(request).await.unwrap();
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    fn label(shell: &crate::shell::Shell, id: &str) -> String {
        shell.state.app(id).unwrap().label.clone()
    }

    #[tokio::test]
    async fn a_deployed_kit_serves_apps_and_its_new_versions_do_too() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let read = include_bytes!("testing-kit.wat");
        let err = deploy(&shell, &app_zip("'leads-read'", read)).await.unwrap_err();
        assert!(err.contains("unknown capability \"leads-read\"") && err.contains("a Wasm kit's grants come once it is deployed"), "{err}");

        deploy(&shell, &crm(CRM)).await.unwrap();
        assert!(dir.path().join("kits/crm").is_dir(), "a directory of its own");
        deploy(&shell, &app_zip("'leads-read'", read)).await.unwrap();
        let (status, body) = get(&shell, "/test").await;
        assert!(status == 200 && body.contains("seven"), "{status} {body}");
        let (status, body) = get(&shell, "/crm").await;
        assert!(status == 200 && body.contains("leads here"), "the kit's page: {status} {body}");
        let v1 = label(&shell, "crm");

        // The app calls the new version, without a deploy of its own.
        deploy(&shell, &crm(&CRM.replace("      i32.const 7)", "      i32.const 8)"))).await.unwrap();
        assert!(get(&shell, "/test").await.1.contains("other"));

        // A version that changes what an app uses is refused, naming it.
        let s32 = CRM.replace(r#"(func $count (param "source" string) (result u32)"#, r#"(func $count (param "source" string) (result s32)"#);
        let err = deploy(&shell, &crm(&s32)).await.unwrap_err();
        assert!(err.contains("test uses crm: ") && err.contains("a new version keeps what is used"), "{err}");
        let gone = CRM.replace(r#"(export "count-leads" (func $count)) "#, "");
        let toml = CRM_KIT.replace("[grants.leads-read]\nallows = \"Count leads\"\nfunctions = [\"acme:crm/leads#count-leads\"]\n", "");
        let err = deploy(&shell, &kit_zip(&toml, &gone, ("crm.wit", CRM_WIT))).await.unwrap_err();
        assert!(err.contains("test uses crm: "), "{err}");
        assert!(get(&shell, "/test").await.1.contains("other"), "the refused versions changed nothing");

        // An app can't take a kit's id, nor a kit an app's.
        let as_app = Bundle {
            manifest: "[app]\nid = 'crm'\nname = 'C'\nversion = '1'\nowner = 'T'\ngroup = 'T'\n".into(),
            ..Bundle::unpack(&app_zip("", read)).unwrap()
        };
        let err = deploy(&shell, &as_app.pack().unwrap()).await.unwrap_err();
        assert!(err.contains("app id \"crm\" is a kit's name"), "{err}");
        let as_kit = CRM_KIT.replace("id = \"crm\"", "id = \"test\"");
        assert!(deploy(&shell, &kit_zip(&as_kit, CRM, ("crm.wit", CRM_WIT))).await.unwrap_err().contains("test is an app here, not a kit"));

        crate::deploy::rollback(&shell, "crm", Some(&v1), crate::deploy::Ask::Cli { reason: None, person: None }).await.unwrap();
        assert!(get(&shell, "/test").await.1.contains("seven"), "rolled back");
        assert_eq!(label(&shell, "crm"), v1);
    }

    #[tokio::test]
    async fn an_app_waits_while_its_kit_is_off() {
        use crate::platform::Actor;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        deploy(&shell, &crm(CRM)).await.unwrap();
        deploy(&shell, &app_zip("'leads-read'", include_bytes!("testing-kit.wat"))).await.unwrap();
        shell.state.turn("crm", false, Actor::Cli, None, None).await.unwrap();
        let (status, body) = get(&shell, "/test").await;
        assert!(status == 503 && body.contains("waits for crm, which it uses"), "{status} {body}");
        // The admin screen says what for, and which is the kit.
        use tower::ServiceExt;
        let admin = axum::http::Request::builder().uri("/admin/apps/test").body(axum::body::Body::empty()).unwrap();
        let response = crate::web::platform_router(shell.clone(), None).oneshot(admin).await.unwrap();
        let body = String::from_utf8(axum::body::to_bytes(response.into_body(), 1 << 22).await.unwrap().to_vec()).unwrap();
        assert!(body.contains("Waits for crm, which it uses"), "{body}");
        // The list says which is the kit; the app's detail, the kits it
        // uses; the kit's, what uses it.
        assert!(body.contains(r#"<span class="c-kind k">kit</span><a class="c-name"#) && body.contains(r#"<span class="c-kind d">app</span>"#), "{body}");
        let uses = body.split(r#"id="kits""#).nth(1).and_then(|p| p.split("</table>").next()).unwrap_or_default();
        assert!(
            uses.contains(r#"<a href="/admin/apps/crm">CRM</a>"#) && uses.contains("Deployed, 1.0.0 live") && uses.contains("<code>leads-read</code>"),
            "{uses}"
        );
        assert!(uses.contains(">Off</span>") && !uses.contains("Serves"), "{uses}");
        let admin = axum::http::Request::builder().uri("/admin/apps/crm").body(axum::body::Body::empty()).unwrap();
        let response = crate::web::platform_router(shell.clone(), None).oneshot(admin).await.unwrap();
        let body = String::from_utf8(axum::body::to_bytes(response.into_body(), 1 << 22).await.unwrap().to_vec()).unwrap();
        let users = body.split(r#"id="users""#).nth(1).and_then(|p| p.split("</table>").next()).unwrap_or_default();
        assert!(users.contains(r#"<a href="/admin/apps/test">"#) && users.contains("<td>App</td>") && users.contains(">Waits for crm</span>"), "{users}");
        assert!(body.contains("It uses no kit"), "crm holds no grant");
        shell.state.turn("crm", true, Actor::Cli, None, None).await.unwrap();
        assert_eq!(get(&shell, "/test").await.0, 200);
    }

    #[tokio::test]
    async fn commands_wait_while_their_kit_is_off_and_go_when_it_is_on() {
        use crate::platform::Actor;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        // Its interface is new: a delivery task starts with the deploy.
        deploy(&shell, &crm(CRM)).await.unwrap();
        deploy(&shell, &app_zip("'leads-write'", include_bytes!("testing-command.wat"))).await.unwrap();
        shell.state.turn("crm", false, Actor::Cli, None, None).await.unwrap();
        let mut t = crate::tx::Tx::default();
        let (interface, function) = ("acme:crm/leads".to_string(), "acme:crm/leads#add-lead".to_string());
        t.send(crate::tx::Outgoing { id: "m1".into(), interface, function, args: r#"["web"]"#.into(), about: "{}".into() });
        assert!(t.commit(&shell.db, "test").unwrap());
        shell.outbox.wake();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let waiting = crate::tx::outbox(&shell.db).unwrap();
        assert!(waiting.len() == 1 && waiting[0].attempts == 0, "it waits, without using up tries: {:?}", waiting.first().map(|w| &w.error));
        shell.state.turn("crm", true, Actor::Cli, None, None).await.unwrap();
        for _ in 0..200 {
            if crate::tx::outbox(&shell.db).unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(crate::tx::outbox(&shell.db).unwrap().is_empty(), "delivered at once when the kit is on");
    }

    #[tokio::test]
    async fn a_kit_that_fails_rolls_back_not_the_app_that_called_it() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        deploy(&shell, &crm(CRM)).await.unwrap();
        deploy(&shell, &app_zip("'leads-read'", include_bytes!("testing-kit.wat"))).await.unwrap();
        let (v1, app) = (label(&shell, "crm"), label(&shell, "test"));
        deploy(&shell, &crm(&CRM.replace("      i32.const 7)", "      unreachable)"))).await.unwrap();
        for _ in 0..crate::deploy::FAILURES_BEFORE_ROLLBACK {
            assert_eq!(get(&shell, "/test").await.0, 503);
        }
        for _ in 0..200 {
            if label(&shell, "crm") == v1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(label(&shell, "crm"), v1, "the kit rolled back");
        assert_eq!(label(&shell, "test"), app);
        assert!(get(&shell, "/test").await.1.contains("seven"));
        let kinds = |id: &str| shell.platform.events(Some(id), 20).unwrap().into_iter().map(|e| e.kind).collect::<Vec<_>>();
        assert!(kinds("crm").contains(&crate::platform::Kind::AutoRolledBack));
        assert!(!kinds("test").contains(&crate::platform::Kind::AutoRolledBack));
    }

    #[tokio::test]
    async fn kits_load_in_order_at_startup_and_call_one_way() {
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let board = kit_zip(BOARD_KIT, include_str!("testing-board-kit.wat"), ("board.wit", BOARD_WIT));
        assert!(deploy(&shell, &board).await.unwrap_err().contains("unknown capability \"leads-read\""), "no CRM yet");
        deploy(&shell, &crm(CRM)).await.unwrap();
        deploy(&shell, &board).await.unwrap();
        deploy(&shell, &app_zip("'leads-read'", include_bytes!("testing-kit.wat"))).await.unwrap();

        // The CRM can't use the board now: kits call one way.
        let calls_back = format!("{CRM_KIT}[capabilities]\ngrant = [\"board-read\"]\n");
        let err = deploy(&shell, &kit_zip(&calls_back, CRM, ("crm.wit", CRM_WIT))).await.unwrap_err();
        assert!(err.contains("calls itself, through its grants"), "{err}");

        // After a restart: board needs the CRM, which loads first, though
        // board comes first by name.
        let parts = crate::shell::Parts {
            platform: shell.platform.clone(),
            db: shell.db.clone(),
            ..crate::testing::parts(dir.path().to_path_buf(), Default::default())
        };
        let again = crate::shell::Shell::new(parts).unwrap();
        let apps = crate::deploy::load_kits(&again, &[]).await.unwrap();
        crate::deploy::load_live(&again, apps).await;
        assert!(again.kits.live("board").is_some() && again.kits.live("crm").is_some());
        assert!(get(&again, "/test").await.1.contains("seven"));
    }
}
