//! Kits in Wasm (kit.rs): a component and its kit.toml, loaded when the
//! platform starts (`--kit DIR`).
//!
//! ```toml
//! [kit]
//! id = "scoring"                      # its name: in policy.csv, for its directory
//! name = "Lead scoring"
//! version = "1.0.0"
//! owner = "Jane Doe <jane@acme.com>"
//! wasm = "kit.wasm"                   # next to kit.toml
//! wit = ["wit/scoring.wit"]           # what it offers
//! commands = ["acme:scoring/leads#add-lead"]   # its functions that are commands
//!
//! [grants.scoring-read]
//! allows = "Score leads: leads::score"
//! functions = ["acme:scoring/leads#score"]
//! cost = 1                            # I/O units a call costs; 1 when left out
//!
//! [capabilities]
//! grant = ["crm-read"]                # other kits' grants it calls with
//!
//! [config]                            # environment variables
//! REGION = "eu"
//! [config-env]                        # read from the shell's environment
//! API_KEY = "SCORING_API_KEY"
//! ```
//!
//! It runs like an app: a fresh instance for each call, with what its
//! caller has left of the request's fuel, I/O and time, and memory of its
//! own. A chain of calls goes at most `MAX_DEPTH` deep. Unlike an app it has
//! a directory of its own (data/kits/<id>, "/" in its WASI), the network
//! and its configuration as environment variables. It gets no kv: it keeps
//! its own data. What it takes and gives are values: no resources, streams
//! or futures. A command to it runs once the sender committed, with a whole
//! budget of its own.
//!
//! A sign-in kit (auth/sign_in_kit.rs) has a kit.toml too, with
//! `[sign-in] hosts = [...]`, the hosts it may fetch from, and no WIT or
//! grants: auth.toml names it, and `config.get` gives it its [config].

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, bail};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use wasmtime::StoreContextMut;
use wasmtime::component::types::{ComponentFunc, ComponentItem, Type};
use wasmtime::component::{Component, Linker, Val};

use crate::kit::{Caller, Context, Fake, Grant, Kit, KitLinker, Kits, Message, Run, Start};
use crate::runtime::{Called, Ctx, FRAMEWORK, FUEL, KitSandbox, Left, call_kit};

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
    #[serde(default = "default_wasm")]
    wasm: PathBuf,
    #[serde(default)]
    wit: Vec<PathBuf>,
    #[serde(default)]
    commands: Vec<String>,
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

/// A kit in Wasm.
pub struct WasmKit {
    id: String,
    /// Like "Lead scoring 1.0.0".
    label: String,
    wit: Vec<(String, String)>,
    grants: Vec<Grant>,
    /// The grants it calls other kits with.
    held: Vec<String>,
    /// Its functions that are commands, like "acme:crm/customers#add-note".
    commands: Vec<String>,
    /// Interfaces with their package's version: "acme:crm/customers" →
    /// "acme:crm/customers@0.1.0".
    versions: HashMap<String, String>,
    wasm: Vec<u8>,
    env: Vec<(String, String)>,
    /// Set by `start`.
    started: Arc<OnceLock<Started>>,
}

/// A started kit: compiled and linked.
struct Started {
    sandbox: KitSandbox,
    /// The functions it exports, with their types, by
    /// "acme:crm/customers#get".
    exports: HashMap<String, ComponentFunc>,
}

impl WasmKit {
    /// Reads the kit in `dir` (its kit.toml, WIT and component); the
    /// environment gives `[config-env]`.
    pub fn load(dir: &Path) -> anyhow::Result<WasmKit> {
        Self::load_with(dir, &|var| std::env::var(var).ok())
    }

    fn load_with(dir: &Path, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<WasmKit> {
        let (file, toml) = read_toml(dir)?;
        let at = |e: anyhow::Error| e.context(file.display().to_string());
        anyhow::ensure!(toml.sign_in.is_none(), "{}: a sign-in kit: auth.toml names it (provider = \"kit\")", file.display());
        anyhow::ensure!(!toml.kit.wit.is_empty(), "{}: wit = [...]: the WIT of what it offers", file.display());
        let mut wit = vec![];
        let mut versions = HashMap::new();
        for path in &toml.kit.wit {
            let text = read_text(&dir.join(path)).map_err(at)?;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (package, version, interfaces) = crate::kit::read_wit(&name, &text).map_err(at)?;
            for i in interfaces {
                let versioned = version.as_ref().map_or_else(|| format!("{package}/{i}"), |v| format!("{package}/{i}@{v}"));
                versions.insert(format!("{package}/{i}"), versioned);
            }
            wit.push((name, text));
        }
        let mut grants = vec![];
        for (name, g) in &toml.grants {
            crate::manifest::validate_grant_name(name).map_err(|e| at(anyhow::anyhow!("[grants.{name}]: {e}")))?;
            let functions: Vec<&str> = g.functions.iter().map(String::as_str).collect();
            grants.push(Grant::new(name, &g.allows, &functions, g.cost));
        }
        for c in &toml.kit.commands {
            anyhow::ensure!(grants.iter().any(|g| g.functions.contains(c)), "{}: commands: no grant allows {c}", file.display());
        }
        let config = config(&toml, env).map_err(at)?;
        let wasm = read_wasm(dir, &toml).map_err(at)?;
        let owner = toml.kit.owner.map(|o| format!(", {o}")).unwrap_or_default();
        Ok(WasmKit {
            label: format!("{} {}{owner}", toml.kit.name, toml.kit.version),
            id: toml.kit.id,
            wit,
            grants,
            held: toml.capabilities.grant,
            commands: toml.kit.commands,
            versions,
            wasm,
            env: config.into_iter().collect(),
            started: Arc::new(OnceLock::new()),
        })
    }

    /// Its id: its name in policy.csv.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// "acme:crm/customers@0.1.0" of "acme:crm/customers".
    fn versioned(&self, interface: &str) -> String {
        self.versions.get(interface).cloned().unwrap_or_else(|| interface.to_string())
    }

    /// Compiles it, checks what it imports and exports, and links it to the
    /// kits whose grants it holds.
    fn compile(&self, engine: &wasmtime::Engine, kits: &Kits, dir: PathBuf, db: crate::tx::Db) -> anyhow::Result<Started> {
        let engine = engine.clone();
        let component = Component::new(&engine, &self.wasm).map_err(anyhow::Error::from).context("not a valid WebAssembly component")?;
        let ty = component.component_type();
        let mut problems = vec![];
        let mut uses = HashMap::new();
        for (name, item) in ty.imports(&engine) {
            let ComponentItem::ComponentInstance(i) = item.ty else {
                problems.push(format!("imports {name}, which is no interface"));
                continue;
            };
            let interface = name.split('@').next().unwrap_or(name);
            if interface.starts_with("wasi:") || interface == format!("{FRAMEWORK}log") || interface == format!("{FRAMEWORK}types") {
                continue;
            }
            let short = interface.rsplit('/').next().unwrap_or(interface);
            if !kits.offers(interface) && !kits.knows(interface) {
                problems.push(format!("imports {interface}, which kits can never use"));
                continue;
            }
            for (f, item) in i.exports(&engine) {
                let ComponentItem::ComponentFunc(func) = item.ty else { continue };
                match kits.grant_for(interface, f) {
                    Some(g) if self.held.contains(&g.name) => {}
                    Some(g) => problems.push(format!("uses {short}::{f}, which needs {} (add it to [capabilities] grant in kit.toml)", g.name)),
                    None => problems.push(format!("uses {short}::{f}, which no grant allows")),
                }
                uses.insert(format!("{interface}#{f}"), func);
            }
        }
        let mut exports = HashMap::new();
        for (name, item) in ty.exports(&engine) {
            let ComponentItem::ComponentInstance(i) = item.ty else { continue };
            let interface = name.split('@').next().unwrap_or(name);
            for (f, item) in i.exports(&engine) {
                if let ComponentItem::ComponentFunc(func) = item.ty {
                    exports.insert(format!("{interface}#{f}"), func);
                }
            }
        }
        for function in self.grants.iter().flat_map(|g| &g.functions) {
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
        let mut linker: Linker<Ctx> = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        crate::runtime::add_log(&mut linker)?;
        kits.link(&mut linker, &self.held, &uses)?;
        let pre = linker.instantiate_pre(&component)?;
        std::fs::create_dir_all(&dir).with_context(|| format!("its directory, {}", dir.display()))?;
        let sandbox = KitSandbox { pre, name: self.id.clone(), held: self.held.clone(), db, dir, env: self.env.clone() };
        Ok(Started { sandbox, exports })
    }

    /// The function's parameter types, for a command's JSON.
    fn params(&self, function: &str) -> anyhow::Result<Vec<Type>> {
        let started = self.started.get().context("the kit has not started")?;
        let func = started.exports.get(function).with_context(|| format!("no command {function}"))?;
        Ok(func.params().map(|(_, t)| t).collect())
    }
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
    let (file, toml) = read_toml(dir)?;
    let at = |e: anyhow::Error| e.context(file.display().to_string());
    let Some(sign_in) = &toml.sign_in else { bail!("{}: no [sign-in]: not a sign-in kit", file.display()) };
    let offers = !toml.kit.wit.is_empty() || !toml.kit.commands.is_empty() || !toml.grants.is_empty() || !toml.capabilities.grant.is_empty();
    anyhow::ensure!(!offers, "{}: a sign-in kit offers apps nothing and calls no kits: no wit, commands, grants or capabilities", file.display());
    Ok(SignIn { hosts: sign_in.hosts.clone(), config: config(&toml, env).map_err(at)?, wasm: read_wasm(dir, &toml).map_err(at)?, id: toml.kit.id })
}

/// kit.toml of `dir`, read, with a valid id.
fn read_toml(dir: &Path) -> anyhow::Result<(PathBuf, KitToml)> {
    let file = dir.join("kit.toml");
    let toml: KitToml = toml::from_str(&read_text(&file)?).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    crate::manifest::validate_app_id(&toml.kit.id).map_err(|e| anyhow::anyhow!("{}: id: {e}", file.display()))?;
    Ok((file, toml))
}

/// [config] and [config-env], read from `env`.
fn config(toml: &KitToml, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<BTreeMap<String, String>> {
    let mut config = toml.config.clone();
    for (key, var) in &toml.config_env {
        anyhow::ensure!(!config.contains_key(key), "{key} is in [config] and [config-env]: give it once");
        match env(var).filter(|s| !s.is_empty()) {
            Some(s) => config.insert(key.clone(), s),
            None => bail!("[config-env] {key} = {var:?}, but {var} is not set"),
        };
    }
    Ok(config)
}

fn read_wasm(dir: &Path, toml: &KitToml) -> anyhow::Result<Vec<u8>> {
    let path = dir.join(&toml.kit.wasm);
    std::fs::read(&path).with_context(|| format!("wasm = {:?}: {}", toml.kit.wasm, path.display()))
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

    fn start(&self, start: &Start<'_>) -> anyhow::Result<()> {
        let dir = start.data().join("kits").join(&self.id);
        let started = self.compile(start.engine(), start.kits(), dir, start.db().clone()).with_context(|| format!("kit {} ({})", self.id, self.label))?;
        let _ = self.started.set(started);
        Ok(())
    }

    fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
        for function in self.grants.iter().flat_map(|g| &g.functions) {
            let Some((interface, name)) = function.split_once('#') else { continue };
            let versioned = self.versioned(interface);
            if let (Some(app), Some(started)) = (linker.uses(interface, name), self.started.get())
                && let Some(kit) = started.exports.get(function)
                && !same(app, kit)
            {
                let short = interface.rsplit('/').next().unwrap_or(interface);
                bail!("{short}::{name} has other types than kit {} gives it: build against the platform's WIT (appos sync)", self.id);
            }
            if self.commands.contains(function) {
                linker.command_dynamic(&versioned, name)?;
                continue;
            }
            let started = self.started.clone();
            let name = name.to_string();
            linker.func_dynamic(&versioned.clone(), &name.clone(), move |mut store: StoreContextMut<'_, Ctx>, params, results| {
                let (started, interface, name) = (started.clone(), versioned.clone(), name.clone());
                Box::pin(async move {
                    let started = started.get().ok_or_else(|| wasmtime::format_err!("the kit has not started"))?;
                    let (cx, mut left) = store.data().kit_call()?;
                    left.fuel = store.get_fuel()?;
                    let (cx, access) = cx.to_kit(&started.sandbox.name, &started.sandbox.held);
                    let called = call_kit(&started.sandbox, cx, access, left, (&interface, &name), params, results).await;
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
            let started = self.started.get().context("the kit has not started")?;
            let (interface, name) = msg.function.split_once('#').with_context(|| format!("no command {}", msg.function))?;
            anyhow::ensure!(self.commands.contains(&msg.function), "no command {}", msg.function);
            let types = self.params(&msg.function)?;
            let args: Vec<Json> = serde_json::from_str(msg.raw_args())?;
            anyhow::ensure!(args.len() == types.len(), "{}: {} arguments, not {}", msg.function, args.len(), types.len());
            let params = args.iter().zip(&types).map(|(a, t)| from_json(a, t)).collect::<anyhow::Result<Vec<_>>>()?;
            let (cx, access) = cx.to_kit(&started.sandbox.name, &started.sandbox.held);
            let left = Left { fuel: FUEL, io: 0, depth: 1 };
            let Called { result, denied, .. } = call_kit(&started.sandbox, cx, access, left, (&self.versioned(interface), name), &params, &mut []).await;
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
/// Wasm kit.
pub(crate) fn fake_files(kits: &Kits, engine: &wasmtime::Engine) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for Fake { name, wit, grants, commands, wasm } in kits.fakes(engine)? {
        let mut toml = toml::Table::new();
        let mut kit = toml::Table::new();
        kit.insert("id".into(), name.clone().into());
        kit.insert("name".into(), format!("{name}, a fake").into());
        kit.insert("version".into(), "0.0.0".into());
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

/// Starts each fake alone with the built-in kits, in a directory of its
/// own that goes again after; answers their names.
pub(crate) fn check_fakes(kits: &Kits) -> anyhow::Result<Vec<String>> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("appos-fakes-{}-{nanos}", std::process::id()));
    let engine = crate::runtime::engine()?;
    let checked = (|| {
        write_fakes(kits, &engine, &dir.join("fakes"))?;
        let mut names = vec![];
        for entry in std::fs::read_dir(dir.join("fakes"))? {
            let path = entry?.path();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let fake = add_dirs(Kits::builtin(), std::slice::from_ref(&path), false)?;
            fake.start(&engine, &dir.join("data"), &crate::tx::in_memory()).with_context(|| format!("the fake {name}"))?;
            names.push(name);
        }
        names.sort();
        Ok(names)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    checked
}

/// Adds `--kit` directories to `kits`, saying which.
pub(crate) fn with_dirs(kits: Kits, dirs: &[PathBuf]) -> anyhow::Result<Kits> {
    add_dirs(kits, dirs, true)
}

pub(crate) fn add_dirs(mut kits: Kits, dirs: &[PathBuf], say: bool) -> anyhow::Result<Kits> {
    for dir in dirs {
        let kit = WasmKit::load(dir)?;
        let (id, label) = (kit.id().to_string(), kit.label.clone());
        kits = kits.with(kit).with_context(|| format!("kit {id} ({})", dir.display()))?;
        if say {
            crate::terminal_logln!("kit {id}: {label}, from {}", dir.display());
        }
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
    /// WAT, `toml` its kit.toml), started in `dir`.
    fn platform(bonus: &Bonus, kit: &str, toml: &str, dir: &Path, db: &crate::tx::Db) -> anyhow::Result<Kits> {
        let at = dir.join("crm");
        std::fs::create_dir_all(&at)?;
        std::fs::write(at.join("kit.toml"), toml)?;
        std::fs::write(at.join("kit.wasm"), kit)?;
        std::fs::write(at.join("crm.wit"), CRM_WIT)?;
        let kits = with_dirs(Kits::builtin().with(bonus.clone())?, &[at])?;
        kits.start(&crate::runtime::engine()?, &dir.join("data"), db)?;
        Ok(kits)
    }

    async fn app(kits: &Kits, grants: &str, wat: &[u8], db: crate::tx::Db) -> anyhow::Result<runtime::LoadedApp> {
        let bundle = Bundle {
            manifest: format!("[app]\nid = 'test'\nname = 'Test'\nversion = '1'\nowner = 'Tests'\ngroup = 'Tests'\n[capabilities]\ngrant = [{grants}]\n"),
            wasm: wat.to_vec(),
            icon: None,
            deps: None,
        };
        runtime::load(&runtime::engine()?, db, kits, &bundle.checked()?).await
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
        let kits = platform(&bonus, KIT, KIT_TOML, dir.path(), &db).unwrap();
        assert!(dir.path().join("data/kits/crm-wasm").is_dir(), "a directory of its own");
        let app = app(&kits, "'leads-read'", include_bytes!("testing-kit.wat"), db).await.unwrap();
        let (page, cost) = run(&app, "read", None, Live::default()).await;
        assert_eq!(page.unwrap_or_else(|f| panic!("{:#}", f.error)).html, [("page".to_string(), "seven".to_string())]);
        assert_eq!(cost.io, 2 + 1, "count-leads and the kit's bonus: one budget");
    }

    #[tokio::test]
    async fn a_wasm_kit_runs_on_its_callers_budget() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let kits = platform(&Bonus::default(), KIT, KIT_TOML, dir.path(), &db).unwrap();
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
        let kits = platform(&bonus, KIT, KIT_TOML, dir.path(), &db).unwrap();
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
        let kits = platform(&bonus, KIT, KIT_TOML, dir.path(), &db).unwrap();
        let app = app(&kits, "'leads-write'", include_bytes!("testing-command.wat"), db.clone()).await.unwrap();
        let (page, cost) = run(&app, "send", Some(ann()), Live::default()).await;
        assert!(page.is_ok() && cost.sent == 2);
        let sent = crate::tx::outbox(&db).unwrap();
        assert_eq!(sent.iter().map(|m| m.args.as_str()).collect::<Vec<_>>(), [r#"["web"]"#, r#"["ads"]"#]);
        crate::outbox::start(db.clone(), crate::platform::PlatformDb::in_memory(), &kits, Live::default()).wake();
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
        let kits = platform(&Bonus::default(), &kit, KIT_TOML, dir.path(), &db).unwrap();
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
        dev.start(&runtime::engine().unwrap(), &dir.path().join("data"), &db).unwrap();
        let app = app(&dev, "'leads-read'", include_bytes!("testing-kit.wat"), db).await.unwrap();
        let (page, _) = run(&app, "read", None, Live::default()).await;
        assert_eq!(page.unwrap_or_else(|f| panic!("{:#}", f.error)).html, [("page".to_string(), "seven".to_string())]);
    }

    #[test]
    fn a_wasm_kit_is_checked_when_the_platform_starts() {
        let (dir, db) = (tempfile::tempdir().unwrap(), crate::tx::in_memory());
        let fails = |kit: &str, toml: &str| format!("{:#}", platform(&Bonus::default(), kit, toml, dir.path(), &db).err().unwrap());
        let ungranted = fails(KIT, &KIT_TOML.replace("grant = [\"bonus-read\"]", "grant = []"));
        assert!(ungranted.contains("uses b::bonus, which needs bonus-read (add it to [capabilities] grant in kit.toml)"), "{ungranted}");
        assert!(fails(KIT, &KIT_TOML.replace("\"bonus-read\"", "\"kv-read\"")).contains("kits get no kv"));
        assert!(fails(KIT, &KIT_TOML.replace("\"bonus-read\"", "\"nope\"")).contains("no kit offers nope"));
        let missing = fails(&KIT.replace("(export \"add-lead\" (func $add))", ""), KIT_TOML);
        assert!(missing.contains("exports no leads::add-lead, which a grant allows"), "{missing}");
        let kits = platform(&Bonus::default(), KIT, KIT_TOML, dir.path(), &db).unwrap();
        assert!(kits.taken().contains("crm-wasm"), "no app can be called crm-wasm");
        let err = tokio::runtime::Runtime::new().unwrap().block_on(app(&kits, "", include_bytes!("testing-kit.wat"), db.clone())).err().unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        let write = |toml: &str| std::fs::write(dir.path().join("kit.toml"), toml).unwrap();
        std::fs::write(dir.path().join("crm.wit"), "package acme:crm@1.0.0;\ninterface leads { count: func() -> u32; }\n").unwrap();
        std::fs::write(dir.path().join("kit.wasm"), "(component)").unwrap();
        let base = "[kit]\nid = \"crm\"\nname = \"CRM\"\nversion = \"1.0.0\"\nwit = [\"crm.wit\"]\n";
        let env = |var: &str| (var == "SET").then(|| "s3cret".to_string());
        let load = |toml: &str| {
            write(toml);
            WasmKit::load_with(dir.path(), &env).map_err(|e| format!("{e:#}"))
        };
        let fails = |toml: &str| load(toml).err().unwrap_or_default();
        let kit = load(&format!("{base}[grants.leads-read]\nallows = \"Count\"\nfunctions = [\"acme:crm/leads#count\"]\n[config-env]\nKEY = \"SET\"")).unwrap();
        assert_eq!((kit.id(), kit.versioned("acme:crm/leads").as_str(), kit.grants[0].cost), ("crm", "acme:crm/leads@1.0.0", 1));
        assert_eq!(kit.env, [("KEY".to_string(), "s3cret".to_string())]);
        assert!(fails(&format!("{base}[config-env]\nKEY = \"UNSET\"")).contains("UNSET is not set"));
        assert!(fails(&base.replace("\"crm\"", "\"CRM!\"")).contains("id:"));
        assert!(fails(&format!("{base}[kit2]\n")).contains("unknown field"));
        assert!(fails(&base.replace("wit = [", "commands = [\"acme:crm/leads#add\"]\nwit = [")).contains("no grant allows acme:crm/leads#add"));
    }
}
