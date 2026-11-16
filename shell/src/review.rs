//! What `appos review` hands appfuzz (https://appfuzz.dev): a manifest, a
//! kv plan and one example request per route. appfuzz runs the app many
//! thousands of times against a simulated shell. The shell writes what is
//! simulated, because only the shell knows what the app is granted, what
//! each host call costs and what one request may use: an app can't loosen
//! its own review.
//!
//! The examples and the data come from `appos dev`. It keeps each route's
//! last request that worked (status below 400) next to the app's kv. A
//! route nobody opened there gets a request made from its pattern. The
//! manifest lists one request per route, and appfuzz's search starts from
//! all of them.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context as _;

use serde_json::{Map, Value, json};

use crate::bundle::Bundle;
use crate::manifest::{Grant, Manifest};
use crate::runtime::{self, ALLOWED_WASI, ALWAYS, FRAMEWORK, FUEL, IO_BUDGET, MEMORY, READ, Request, Route, WRITE};
use crate::shell::Shell;
use crate::terminal_errorln as eprintln;
use crate::tx::{self, MAX_SCAN};

/// The oldest appfuzz that reads what this writes: `[entry] examples`,
/// `req.` paths, `clock.rfc3339` and the `json` label came in 0.8.0.
pub const MIN_APPFUZZ: (u32, u32, u32) = (0, 8, 0);

/// Where `appos dev` serves, for the requests made up here.
const ORIGIN: &str = "http://127.0.0.1:8080";

/// What appfuzz gets for one app.
pub struct Review {
    /// appfuzz.toml.
    pub manifest: String,
    /// The plan (JSON): what kv::get and kv::scan answer, from the app's
    /// kv in appos dev.
    pub plan: String,
    /// The routes, in the order init registered them.
    pub examples: Vec<Example>,
    /// kv entries the plan holds.
    pub entries: usize,
}

pub struct Example {
    /// As the app wrote it, like "GET /sessions/:session".
    pub route: String,
    /// Kept by appos dev; false for one made from the route's pattern.
    pub kept: bool,
}

/// Keeps `req` as its route's example, off the request's thread.
pub(crate) fn keep(shell: &Shell, app: &str, handler: &str, req: &Request) {
    let (db, app, handler, json) = (shell.db.clone(), app.to_string(), handler.to_string(), request_json(req).to_string());
    tokio::task::spawn_blocking(move || {
        if let Err(e) = tx::keep_example(&db, &app, &handler, &json) {
            eprintln!("[{app}] could not keep the example of {handler}: {e}");
        }
    });
}

/// The review of the app in `dir`, whose .wasm appfuzz finds at `wasm`,
/// with the data appos dev keeps in `data` (none if it never ran there).
pub async fn prepare(dir: &Path, data: &Path, wasm: &str) -> anyhow::Result<Review> {
    let path = dir.to_path_buf();
    let checked = tokio::task::spawn_blocking(move || Bundle::from_dir(&path)?.checked()).await??;
    // init can't use kv; the routes are all this load is for.
    let app = runtime::load(&runtime::engine()?, tx::in_memory(), &checked).await?;
    let m = &app.manifest;
    let (entries, kept) = match data.join("appos.db") {
        db if db.is_file() => {
            let db = tx::open_db(&db)?;
            (tx::entries(&db, &m.app.id)?, tx::examples(&db, &m.app.id)?)
        }
        _ => (vec![], vec![]),
    };
    let kept: Vec<(String, Value)> = kept.into_iter().filter_map(|(h, json)| Some((h, serde_json::from_str(&json).ok()?))).collect();
    let now = latest(kept.iter().filter_map(|(_, req)| req["now"].as_str())).unwrap_or_else(chrono::Utc::now).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let (mut requests, mut examples) = (Vec::new(), Vec::new());
    for r in &app.routes {
        let (request, kept) = match kept.iter().find(|(h, _)| *h == r.handler) {
            Some((_, req)) => (req.clone(), true),
            None => (request_json(&made_up(r, &m.app.id, &now)), false),
        };
        requests.push(json!([r.handler, request]));
        examples.push(Example { route: r.pattern(), kept });
    }
    let handlers: Vec<&str> = app.routes.iter().map(|r| r.handler.as_str()).collect();
    Ok(Review {
        manifest: manifest(m, wasm, &requests, &now, &handlers)?,
        plan: serde_json::to_string_pretty(&plan(m, &entries))?,
        examples,
        entries: entries.len(),
    })
}

/// The latest of these RFC 3339 times.
fn latest<'a>(times: impl Iterator<Item = &'a str>) -> Option<chrono::DateTime<chrono::Utc>> {
    times.filter_map(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.to_utc()).max()
}

/// The request as appfuzz writes a WIT record: lists of pairs as arrays.
fn request_json(r: &Request) -> Value {
    let pairs = |l: &[(String, String)]| l.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>();
    json!({
        "method": r.method,
        "path": r.path,
        "params": pairs(&r.params),
        "query": pairs(&r.query),
        "form": pairs(&r.form),
        "base": r.base,
        "origin": r.origin,
        "htmx": r.htmx,
        "now": r.now,
    })
}

/// A request for a route appos dev kept no example of: each `:name` is "name".
fn made_up(route: &Route, app: &str, now: &str) -> Request {
    let params = route.segments.iter().filter_map(|s| s.strip_prefix(':')).map(|n| (n.to_string(), n.to_string())).collect();
    let path: Vec<&str> = route.segments.iter().map(|s| s.strip_prefix(':').unwrap_or(s)).collect();
    Request {
        method: route.method.clone(),
        path: format!("/{}", path.join("/")),
        params,
        query: vec![],
        form: vec![],
        base: format!("/{app}"),
        origin: ORIGIN.into(),
        htmx: false,
        now: now.into(),
    }
}

/// A TOML value.
fn toml_value(v: impl serde::Serialize) -> anyhow::Result<String> {
    Ok(toml::Value::try_from(v)?.to_string())
}

/// appfuzz.toml: how to call the app (`start(handler, req)`, one request
/// per route), what it may import, what one request may use, and what
/// each host function costs and does.
fn manifest(m: &Manifest, wasm: &str, requests: &[Value], now: &str, handlers: &[&str]) -> anyhow::Result<String> {
    let a = &m.app;
    let mut t = String::new();
    writeln!(t, "# appfuzz manifest for {} {}, written by `appos review` from what the", a.id, a.version)?;
    writeln!(t, "# AppOS shell gives the app. Written again on every review: edits are lost.\n")?;
    writeln!(t, "[app]\nwasm = {}\nname = {}\n", toml_value(wasm)?, toml_value(&a.name)?)?;
    writeln!(t, "[entry]\nexport = {}", toml_value(format!("{FRAMEWORK}handler#start"))?)?;
    let (first, rest) = requests.split_first().context("init registered no routes")?;
    writeln!(t, "request = {}", toml_value(first)?)?;
    if !rest.is_empty() {
        writeln!(t, "examples = [")?;
        for r in rest {
            writeln!(t, "  {},", toml_value(r)?)?;
        }
        writeln!(t, "]")?;
    }
    writeln!(t, "now = {}", toml_value(now)?)?;
    // The shell fills in the time, and these are the HTTP request's parts.
    let fields = [("now", "clock.rfc3339"), ("method", "http.method"), ("path", "http.path"), ("query", "http.query"), ("form", "http.form")];
    let fields: Vec<String> = fields.iter().map(|(f, role)| format!("\"req.{f}\" = \"{role}\"")).collect();
    writeln!(t, "fields = {{ {} }}", fields.join(", "))?;
    // The shell only ever calls a handler init registered.
    if handlers.iter().all(|h| !h.contains(',')) {
        writeln!(t, "values = {{ handler = {} }}", toml_value(format!("enum:{}", handlers.join(",")))?)?;
    }

    let mut imports: Vec<String> = ALWAYS.iter().filter(|i| **i != "handler").map(|i| format!("{FRAMEWORK}{i}")).collect();
    let (read, write) = (m.grants(Grant::KvRead), m.grants(Grant::KvWrite));
    if read || write {
        imports.push(format!("{FRAMEWORK}kv"));
    }
    let wasi: Vec<&str> = ALLOWED_WASI.iter().map(|w| w.trim_end_matches('/')).collect();
    writeln!(t, "\n[grants]\nimports = {}\nwasi = {}\n", toml_value(&imports)?, toml_value(&wasi)?)?;

    writeln!(t, "[limits]\nfuel = {FUEL}\nmemory = \"{}MiB\"", MEMORY / (1024 * 1024))?;
    writeln!(t, "cost_per_request = {IO_BUDGET}\ndefault_cost = 0")?;

    let function = |t: &mut String, name: &str, body: &str| writeln!(t, "\n[functions.\"{FRAMEWORK}{name}\"]\n{body}");
    // The SDK stores JSON (set_json, get_json). A scan's values stay
    // unlabelled: a label can't name one element of its (key, value) pairs.
    if read {
        function(&mut t, "kv#get", &format!("cost = {READ}\neffect = \"read\"\nresult = \"json\""))?;
        function(&mut t, "kv#scan", &format!("cost = {READ}\neffect = \"read\""))?;
    }
    if write {
        function(&mut t, "kv#set", &format!("cost = {WRITE}\neffect = \"write\"\nparams = {{ value = \"json\" }}"))?;
        function(&mut t, "kv#delete", &format!("cost = {WRITE}\neffect = \"delete\""))?;
    }
    // The shell puts both into the page as they are.
    function(&mut t, "page#set-html", "params = { html = \"html\" }")?;
    function(&mut t, "page#add-header", "params = { html = \"html\" }")?;
    Ok(t)
}

/// What kv::get and kv::scan answer: the app's kv in appos dev. A scan
/// is answered for every prefix that ends at a "/" of a key, and "", from
/// its first page; a later page is empty, and so is any other prefix.
fn plan(m: &Manifest, entries: &[(String, String)]) -> Value {
    let mut plan = Map::new();
    if !m.grants(Grant::KvRead) {
        return Value::Object(plan);
    }
    let get: Vec<Value> = entries.iter().map(|(k, v)| json!({ "when": { "key": k }, "return": v })).collect();
    plan.insert(format!("{FRAMEWORK}kv#get"), json!({ "rules": get, "default": { "return": null } }));

    let mut prefixes = BTreeSet::from([String::new()]);
    for (key, _) in entries {
        prefixes.extend(key.match_indices('/').map(|(i, _)| key[..=i].to_string()));
    }
    let mut scan = Vec::new();
    for prefix in &prefixes {
        let under: Vec<Value> = entries.iter().filter(|(k, _)| k.starts_with(prefix.as_str())).map(|(k, v)| json!([k, v])).collect();
        let page = |reverse: bool| -> Vec<Value> {
            let n = MAX_SCAN as usize;
            if reverse { under.iter().rev().take(n).cloned().collect() } else { under.iter().take(n).cloned().collect() }
        };
        for reverse in [false, true] {
            scan.push(json!({ "when": { "prefix": prefix, "after": null, "reverse": reverse }, "return": page(reverse) }));
        }
    }
    plan.insert(format!("{FRAMEWORK}kv#scan"), json!({ "rules": scan, "default": { "return": [] } }));
    Value::Object(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(grants: &str) -> Manifest {
        Manifest::parse(&format!("[app]\nid = 'events'\nname = 'Events \"2026\"'\nversion = '1.0'\n[capabilities]\ngrant = [{grants}]\n")).unwrap()
    }

    fn route(pattern: &str, handler: &str) -> Route {
        let (method, path) = pattern.split_once(' ').unwrap();
        Route { method: method.into(), segments: path.split('/').filter(|s| !s.is_empty()).map(String::from).collect(), handler: handler.into() }
    }

    fn written(m: &Manifest, handlers: &[&str]) -> toml::Table {
        let requests: Vec<Value> = handlers.iter().map(|h| json!([h, request_json(&made_up(&route(h, h), "events", "2026-11-15T10:00:00Z"))])).collect();
        manifest(m, "events.wasm", &requests, "2026-11-15T10:00:00Z", handlers).unwrap().parse().unwrap()
    }

    #[test]
    fn the_manifest_holds_the_shells_limits_and_costs() {
        let t = written(&manifest_with("'kv-read', 'kv-write'"), &["GET /", "POST /items"]);
        assert_eq!(t["app"]["name"].as_str(), Some("Events \"2026\""));
        assert_eq!(t["entry"]["export"].as_str(), Some("appos:framework/handler#start"));
        assert_eq!(t["entry"]["request"][0].as_str(), Some("GET /"));
        assert_eq!(t["entry"]["request"][1]["base"].as_str(), Some("/events"));
        let examples = t["entry"]["examples"].as_array().unwrap();
        assert_eq!(examples.len(), 1, "the routes after the first");
        assert_eq!(examples[0][1]["path"].as_str(), Some("/items"));
        assert_eq!(t["entry"]["values"]["handler"].as_str(), Some("enum:GET /,POST /items"));
        assert_eq!(t["entry"]["fields"]["req.now"].as_str(), Some("clock.rfc3339"));
        assert_eq!(t["entry"]["fields"]["req.method"].as_str(), Some("http.method"));
        assert_eq!(t["limits"]["fuel"].as_integer(), Some(FUEL as i64));
        assert_eq!(t["limits"]["memory"].as_str(), Some("64MiB"));
        assert_eq!(t["limits"]["cost_per_request"].as_integer(), Some(i64::from(IO_BUDGET)));
        let f = &t["functions"];
        assert_eq!(f["appos:framework/kv#get"]["cost"].as_integer(), Some(i64::from(READ)));
        assert_eq!(f["appos:framework/kv#set"]["cost"].as_integer(), Some(i64::from(WRITE)));
        assert_eq!(f["appos:framework/kv#get"]["result"].as_str(), Some("json"));
        assert_eq!(f["appos:framework/kv#set"]["params"]["value"].as_str(), Some("json"));
        assert_eq!(f["appos:framework/kv#delete"]["effect"].as_str(), Some("delete"));
        assert_eq!(f["appos:framework/page#set-html"]["params"]["html"].as_str(), Some("html"));
        let wasi: Vec<&str> = t["grants"]["wasi"].as_array().unwrap().iter().filter_map(|w| w.as_str()).collect();
        assert_eq!(wasi, ["wasi:io", "wasi:clocks", "wasi:random", "wasi:cli"]);
    }

    #[test]
    fn the_manifest_grants_only_what_app_toml_does() {
        let t = written(&manifest_with(""), &["GET /"]);
        let imports: Vec<&str> = t["grants"]["imports"].as_array().unwrap().iter().filter_map(|i| i.as_str()).collect();
        assert!(!imports.contains(&"appos:framework/kv"), "{imports:?}");
        assert!(imports.contains(&"appos:framework/page"), "{imports:?}");
        assert!(t["functions"].get("appos:framework/kv#get").is_none());

        let t = written(&manifest_with("'kv-read'"), &["GET /"]);
        assert!(t["functions"].get("appos:framework/kv#get").is_some());
        assert!(t["functions"].get("appos:framework/kv#set").is_none());
        assert!(plan(&manifest_with(""), &[]).as_object().unwrap().is_empty());
    }

    #[test]
    fn a_handler_with_a_comma_gets_no_enum() {
        let t = written(&manifest_with(""), &["GET /", "GET /a,b"]);
        assert!(t["entry"].get("values").is_none());
    }

    #[test]
    fn the_plan_answers_gets_and_first_scans_from_the_dev_data() {
        let entries: Vec<(String, String)> = [("a/1", "x"), ("a/2", "y"), ("a/b/3", "z"), ("top", "t")].map(|(k, v)| (k.into(), v.into())).into();
        let plan = plan(&manifest_with("'kv-read'"), &entries);
        let get = &plan["appos:framework/kv#get"];
        assert_eq!(get["rules"].as_array().unwrap().len(), 4);
        assert_eq!(get["rules"][3], json!({ "when": { "key": "top" }, "return": "t" }));
        assert_eq!(get["default"], json!({ "return": null }));

        let scan = &plan["appos:framework/kv#scan"];
        let answer = |prefix: &str, reverse: bool| {
            let rule = scan["rules"].as_array().unwrap().iter().find(|r| r["when"] == json!({ "prefix": prefix, "after": null, "reverse": reverse }));
            let keys = |r: &Value| r["return"].as_array().unwrap().iter().map(|e| e[0].as_str().unwrap().to_string()).collect::<Vec<_>>();
            rule.map(keys)
        };
        assert_eq!(answer("a/", false).unwrap(), ["a/1", "a/2", "a/b/3"]);
        assert_eq!(answer("a/", true).unwrap(), ["a/b/3", "a/2", "a/1"]);
        assert_eq!(answer("a/b/", false).unwrap(), ["a/b/3"]);
        assert_eq!(answer("", false).unwrap().len(), 4);
        assert_eq!(answer("a/1", false), None, "only prefixes that end at a /");
        assert_eq!(scan["default"], json!({ "return": [] }));
    }

    #[test]
    fn a_route_without_an_example_gets_one_made_from_its_pattern() {
        let r = made_up(&route("POST /sessions/:session/regs/:reg", "h"), "events", "2026-11-15T10:00:00Z");
        assert_eq!(r.path, "/sessions/session/regs/reg");
        assert_eq!(r.params, [("session".to_string(), "session".to_string()), ("reg".to_string(), "reg".to_string())]);
        assert_eq!((r.method.as_str(), r.base.as_str(), r.now.as_str()), ("POST", "/events", "2026-11-15T10:00:00Z"));
        assert_eq!(made_up(&route("GET /", "h"), "events", "").path, "/");
    }

    #[test]
    fn the_latest_time_is_in_utc() {
        let t = latest(["2026-11-15T10:00:00+01:00", "2026-11-15T09:30:00Z", "yesterday"].into_iter()).unwrap();
        assert_eq!(t.format("%Y-%m-%dT%H:%M:%SZ").to_string(), "2026-11-15T09:30:00Z");
        assert!(latest(std::iter::empty()).is_none());
    }

    #[test]
    fn a_kept_example_replaces_the_one_before_per_route() {
        let db = tx::in_memory();
        tx::keep_example(&db, "events", "GET /", "1").unwrap();
        tx::keep_example(&db, "events", "GET /", "2").unwrap();
        tx::keep_example(&db, "events", "GET /x", "3").unwrap();
        tx::keep_example(&db, "other", "GET /", "4").unwrap();
        assert_eq!(tx::examples(&db, "events").unwrap(), [("GET /".to_string(), "2".to_string()), ("GET /x".to_string(), "3".to_string())]);
    }
}
