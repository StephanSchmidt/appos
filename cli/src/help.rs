//! The help texts and the guide: the documentation of appos, for people and
//! their agents.

// Help and guide text go to a terminal like everything else.
use appos_shell::terminal_print as print;

pub const HELP: &str = r#"appos: build apps for an AppOS shell

An AppOS app is a small program in Rust, TypeScript or Java, compiled to
WebAssembly. The shell runs it in a sandbox, mounts it at /<app-id>/, wraps
its pages in the site layout, and gives it storage. Pages are templates
(Askama in Rust, html`...` in TypeScript, html("...") in Java); htmx makes
them interactive without writing JavaScript.

Apps:
  appos new <name> [--group <team>] [--ts | --java]
                       create an app in ./<name>: a working hello world,
                       in TypeScript with --ts, in Java with --java. Change
                       it into your app.
  appos dev            build the app in this directory, run it in a local
                       shell, and rebuild and reload it whenever you change it
  appos build          build only
  appos review         have appfuzz run the app many thousands of times
                       against a simulated shell and say what it found
  appos sync           update the app's SDK (src/appos.rs, src/appos.ts or
                       src/main/java/app/Appos.java) and wit/ to this
                       appos and the current platform
  appos guide          how apps work: routes, the page, htmx, storage, styles, rules

Platforms (the shells you deploy to, like kubectl contexts). Not needed for
local development: `appos new` and `appos dev` work without one, and there
is nothing to configure.
  appos login <url>    connect to a shell's platform API and make it current.
                       A shell also serves: curl -fsSL <url>/_platform/install | sh
  appos logout         end the token of the current platform
  appos platforms      list known platforms
  appos use <name>     switch the current platform
  appos hosts          what the current platform offers: capabilities, apps
  appos deploy [app.zip]
                       build the app in this directory and run it on the
                       current platform; or send an app.zip made elsewhere
  appos pack [file]    build the app as one file, app.zip, to deploy later
  appos versions       the versions of this app on the current platform
  appos log [--failures]
                       what happened to this app on the current platform,
                       with the errors behind rollbacks and limit hits
  appos rollback [version] [--reason <text>]
                       make an earlier version live again
  appos off [--reason <text>]
                       turn the app off: it answers with a page saying so
  appos on [--reason <text>]
                       turn it on again, unless an admin turned it off

Every command has its own help: appos <command> --help

Start: appos new <name>, cd <name>, appos dev, then read src/lib.rs and src/hello/.
"#;

const GUIDE: &str = r#"How AppOS apps work

The hello world from `appos new` is only a starting point. Change, rename,
restructure or replace anything in it: the HTML, the CSS, the Rust code,
the templates, the file layout, the look. Fixed are only the contract with
the shell (wit/, the page:: and kv:: calls) and the sandbox limits under
RULES. FORMS, STORAGE and ERRORS are what keeps an app safe and easy to
change as it grows.

Each section also prints on its own: appos guide <section>, one of files,
lifecycle, request, page, templates, htmx, forms, storage, errors, styles,
rules, git, dev, testing, typescript, java.

FILES (as appos new creates them)
  app.toml             id, name, version, owner (who to ask about the app,
                       like "Jane Doe <jane@example.com>"), group (the team
                       it belongs to, like "Marketing"; a deploy needs both),
                       the .wasm path, and the capabilities the app is
                       granted
  Cargo.toml           a normal Rust crate. Add crates that build for
                       wasm32-wasip2 (pure Rust, no networking or files).
                       serde, serde_json, thiserror and uuid are already
                       there. Its [lints] deny unwrap() outside tests.
  Cargo.lock           the exact versions of those crates; commit it
  Makefile             make check: formatting, lints and unit tests;
                       make fmt fixes the formatting; make dev runs appos dev
  clippy.toml          lets tests unwrap()
  rust-toolchain.toml  the Rust toolchain; rustup installs what it lists
  .gitignore           keeps target/ and appos-data/ out of git
  src/lib.rs           your app: init() and start(); MODULES lists the
                       modules, and it explains their layers
  src/hello/           a module: routes (mod.rs), handlers (web.rs), one
                       usecase per file, IO (db.rs), data and rules, including
                       input checks (domain.rs)
  src/appos.rs         the SDK: bindings to the shell and helpers (routes and
                       modules, the request's time, dates, kv with JSON, form
                       errors). `appos sync` replaces it with the one of your
                       appos, so keep your own code elsewhere
  templates/           Askama templates (Jinja syntax), checked at compile
                       time: the shared style.css, and a folder per module
  wit/                 the contract with the shell, from the platform (or
                       built into appos). The bindings are generated from it
                       at build time; a shell refuses an app built against a
                       contract it doesn't have, so don't edit it: `appos
                       sync` updates it, and appos dev and appos deploy warn
                       when it differs
  icon.svg             optional: the app's icon, square, up to 64 KB; the
                       shell shows it in the browser tab of the app's pages
                       (unless the app sets its own), in the app list and
                       on the admin screen

LIFECYCLE
  init()               runs once when the shell loads the app. Each module
                       lists its routes and menu entries in its mod.rs:
                         pub const MODULE: Module = Module {
                             routes: &[
                                 Route::get("/", web::index),
                                 Route::post("/tasks/:id/done", web::done),
                             ],
                             menu: &[("Tasks", "/")],
                         };
                       src/lib.rs lists the modules,
                         const MODULES: &[Module] = &[tasks::MODULE];
                       and init() registers them: appos::register(MODULES).
                       Paths are below the app's base; :name segments arrive as
                       req.param("name").
  start(handler, req)  runs per request in a fresh sandbox: nothing in memory
                       survives between requests, keep state in kv.
                       appos::dispatch(MODULES, &handler, &req) calls the
                       route's function, which builds the page with page::
                       calls; then page::done() sends the result to the
                       browser.

REQUEST
  req.param(n) req.query(n) req.form(n)   route, query and form values (text;
                                          check them, they can hold anything)
  req.htmx             true for htmx fragment swaps; false for full pages,
                       including history restoration
  req.url("/tasks")    "/<app-id>/tasks"; req.url("/") is the app's home
  req.url_with("/", &[("q", "a b")])      the same with an encoded query string
  req.base             "/<app-id>"; pass it to templates and write {{ base }}/tasks
  req.now()            the current time, an appos::time::Time in the
                       platform's offset: .unix() seconds, .date(), .clock()
                       (h, m, s), .plus(secs); it prints and stores (serde)
                       as RFC 3339 text, "2026-11-11T14:05:09+01:00", and
                       Time::parse reads that back
  appos::time          Date: Date::parse("2026-11-11"), .weekday() (0 is
                       Monday), .plus_days(n), .at(h, m, s, offset);
                       time::parse_time("14:05"); MINUTE, HOUR, DAY seconds

THE PAGE
  Per request the app calls these, like it calls kv:
    page::set_html("page", &html(&t))    the whole page content; the shell adds
                                         layout, menu and scripts
    page::set_html("tasks", &html(&t.as_tasks()))
                                         a named part, for htmx requests
    page::add_header(html)               adds to <head>; helpers:
                                           appos::style(css)    a <style> block
                                           appos::title("Tasks")
    page::redirect(&req.url("/"))        send the browser to another page
    page::set_status(code)               200 unless set
    appos::not_found()                   a 404 page
    page::done()                         the app is done: it stops right
                                         there and is unloaded, and the page
                                         is sent. Nothing after it runs, so
                                         save to kv first. Returning from
                                         start() without done() also sends
                                         what was set.
  There are two kinds of request:
  - a full-page request (req.htmx is false): set "page", a full page. After a
    form post, page::redirect(&req.url("/")) instead; the next page reads
    what it shows from kv. History restoration also needs the full page.
  - an htmx request (req.htmx is true): set named parts. The shell sends each
    one as
        <div id="tasks" hx-swap-oob="innerHTML">...</div>
    and htmx puts it into the element with id "tasks", which must already be
    on the page. Parts with no matching element are ignored.
  Only what you set changes. If the server changes something else the user
  sees, such as a form's input values, set that part too.
  Named parts on a normal request show the user an error page; "page" sent
  to an htmx request from an element with hx-swap="none" changes nothing.

TEMPLATES (ASKAMA)
  Give each part that can change an element with an id and a block of the
  same name, and list the blocks in the template's struct:
      <ul id="tasks">{% block tasks %}...{% endblock %}</ul>
      #[derive(Template)]
      #[template(path = "index.html", blocks = ["tasks", "count", "form"])]
      struct Index { base: String, ... }
  t.as_tasks() renders only that block. One template serves the full page
  and all of its updates.
  A block inside {% if %}, {% for %} or {% match %} renders empty on its
  own, with no error. Put the condition inside the block instead:
      <p id="error">{% block error %}{% if let Some(e) = error %}{{ e }}{% endif %}{% endblock %}</p>
  and test each block alone (t.as_error().render()). The same for a form
  that turns into a thank-you: one block, the if inside it.
  Askama is Rust inside Jinja syntax; fields are borrowed, so compare with
  a deref where needed (e.g. {% if item.id == *selected %}):
      {% if let Some(e) = error %}<p class="error">{{ e }}</p>{% endif %}
      {% for t in tasks %}<li>{{ t.title }}</li>{% else %}<li>Nothing yet</li>{% endfor %}
      {% match status %}{% when Status::Open(by) %}open, {{ by }}{% when Status::Done %}done{% endmatch %}
      {% if !q.is_empty() %} ... {{ tasks.len() }}
  Everything is HTML-escaped. Never use |safe on input.

HTMX
  <form method="post" action="{{ base }}/tasks" hx-post="{{ base }}/tasks" hx-swap="none">
  hx-swap="none": the named parts the app sets do the updating. Keep
  method/action so the form also works without JavaScript.
  htmx swaps the parts of a 409 or 422 response like those of a 200 (see
  FORMS).
  The shell loads htmx 2.0.4. What makes htmx run code is blocked by the
  Content Security Policy (RULES): hx-on, trigger filters such as
  keyup[key=='Enter'], and js: in hx-vals. An element htmx replaces keeps
  focus and cursor only if it has an id.

FORMS
  Parse, don't validate. Check every form value on the server; browser
  checks (maxlength, required, type="email") only help the user while
  typing. Parse each value into a domain type that can only hold valid
  values (Name::parse in domain.rs returns Result<Name, NameError>); usecases
  take only those types, never raw text. A form with more fields parses
  each one. Parsing and its errors belong to the handler (see ERRORS).
  appos::FormErrors<E> collects them by field: errors.keep("name",
  Name::parse(text)) is Some(name), or None and the error kept; the form
  parsed if errors.is_empty(). In the template, {{ errors.message("name") }}
  is that field's message, "" if it has none.
  Input that doesn't pass: page::set_status(422) and tell the user why.
  Don't lose what they typed: on an htmx request set only the parts that
  change, so the inputs keep their text and focus; on a normal request
  send the whole page again with what they typed.

STORAGE
  kv::get(key) -> Option<String>     kv::set(key, value)
  kv::delete(key)
  kv::scan(prefix, after, limit, reverse) -> Vec<(key, value)>
    keys starting with prefix, in key order, at most limit (up to 1000);
    reverse starts from the last key; after = Some(last key of the previous
    page) goes on from there. A scan that returns more than 4 MB of keys
    and values stops the request: page through big values.
  Values are strings; the SDK stores structs as JSON (serde):
    kv::get_json::<T>(key) -> Result<Option<T>, kv::Unreadable>
    kv::set_json(key, &value)
    kv::scan_json::<T>(prefix, after, limit, reverse) -> Vec<(key, T)>
    kv::scan_all_json::<T>(prefix, reverse)  every entry, 100 keys per scan
  Stored data can be anything: another version of the app, a person or a
  bug may have written it. get_json returns kv::Unreadable for a value that
  isn't the JSON of a T (see ERRORS); the scans log and leave out an entry
  they can't read.
  Private to the app, and shared by all its visitors: apps don't see who
  is signed in, so everyone sees the same state. Keys up to 1 KB, values up
  to 1 MB.
  Needs grants in app.toml: "kv-read" for get and scan, "kv-write" for set
  and delete. An app that only shows data grants only "kv-read".
  Each request is a transaction. Its writes are applied together when it
  ends, or not at all if it fails; until then only the request itself sees
  them. If another request changed a key this one read with kv::get, this
  one's writes are dropped and it runs again from the start. So reading a
  value and writing it back changed (a counter, "register unless already
  registered") is safe. Scans are not checked.
  Many requests run at once. Requests that write the same key run again
  and wait on each other. So a shared key is fine where a rule needs it
  (seats left, checked and written by each sign-up), but not for what every
  request touches (a page view count). Give each new item its own key
  (prefix/<uuid v7>: unique and sorted by time, see db.rs).
  Each request may spend 100 I/O units. kv::get and kv::scan cost 1, a
  scan of 1000 keys too, so list with one scan, not a get per item.
  kv::set and kv::delete cost 5, so one request writes at most 20 keys.

ERRORS
  Four kinds, each handled one way:
  - Input that doesn't parse (a form field, an id in the URL): the handler
    parses with Type::parse and answers. A form: 422 and the message next to
    the field (FORMS). A path: 404 (appos::not_found()), because a broken id
    is as unknown as one nobody has.
  - An outcome the data decides (no such greeting, already registered,
    event full): the usecase returns it in an error enum of its own,
    Result<Greeting, ShowError>, with only the variants that usecase can
    produce. The handler matches every variant by name, without `_`, so a
    new variant doesn't compile until the handler decides what the user
    sees: a 404, a 409 or 422 with a message, another page.
  - Stored data that can't be read: kv::get_json returns kv::Unreadable.
    It is an outcome like the others: the usecase's error enum has
        #[error(transparent)]
        Unreadable(#[from] kv::Unreadable),
    load(id)? returns it before the usecase writes anything, and the
    handler answers appos::unreadable(&e): status 500, a page saying the
    page can't be shown, and the key and why in the log. A list skips (and
    logs) an item it can't read instead, so one bad entry doesn't take the
    page down.
  - A bug: something that can't happen did. Don't return it; panic with
    what broke:
        .expect("why this can't fail")
    The request stops and writes nothing, the user sees "This app is
    unavailable right now", and the terminal shows the message with file
    and line.
  Error enums derive thiserror::Error; #[error("...")] is their text, for
  a user (NameError) or for the log. Code and tests match on variants,
  never on text. kv calls can't fail from the app's side: if the store
  does, the shell stops the request. make check denies unwrap() outside
  tests: use expect("why") instead. println!/eprintln! go to the terminal
  too; log::info and log::warn are meant for it.
  Who may read and write is the platform's to say, not the app's: a
  visitor its policy doesn't let is sent to sign in or shown a page by the
  shell, and the request writes nothing. The app does nothing about it.

STYLES
  The app's design is entirely the app's: any HTML, any modern CSS. The
  shell adds only the top bar with the menu (its classes start with appos-).
  CSS goes into <head> with page::add_header (appos::style(css) wraps it in
  <style>) or into style="..." attributes. The shell serves no files for
  apps and blocks other sites, so use CSS, system fonts, inline SVG and
  data: URLs rather than external stylesheets, fonts or images.

RULES
  - No JavaScript of your own: the shell's Content Security Policy blocks
    inline scripts and event handlers. htmx is loaded for you.
  - The shell writes <html> and <head>; the app sets the content and adds
    to <head> with page::add_header.
  - Redirects and hx-* URLs must stay on this site.
  - The sandbox has no files, network or environment. Data goes through kv.
  - Using a capability that app.toml doesn't grant makes the shell refuse
    the app.
  - Each request has a budget: 100 I/O units (see STORAGE), fuel (1 billion
    instructions), 64 MB memory and 2 seconds. A request over budget is
    stopped: the user sees "This app is unavailable right now" and the
    terminal shows why. An app that needs more time sets it in app.toml, up
    to 30:
      [limits]
      seconds = 5

GIT
  Keep every app in git, so every working state is saved and can be restored.
  appos doesn't do it for you; right after `appos new`:
      git init -b main && git add -A && git commit -m "New app"
  Then commit every time the app works after a change, Cargo.lock included:
      git add -A && git commit -m "Let users mark tasks as done"
  If the platform keeps app repositories on a git server, `appos new` prints
  the remote to add and push to. See what changed: git status, git diff.
  Undo uncommitted changes: git restore .

LOCAL DEVELOPMENT
  appos dev serves the app at http://127.0.0.1:8080/<app-id>, watches src/,
  templates/, wit/, .cargo/, app.toml, Cargo.toml, Cargo.lock, build.rs,
  rust-toolchain.toml, icon.svg and, for a TypeScript app, package.json,
  package-lock.json and tsconfig.json, for a Zig app build.zig and
  build.zig.zon, for a Java app pom.xml, and rebuilds and reloads on every
  change. If a build fails, the last good version keeps running.
  The first build downloads crates, so it needs the network and takes a
  while; the app is ready when the shell prints "AppOS shell on ...", and
  again after each rebuild when it prints "loaded <app-id> ...". kv data is
  kept in ./appos-data across restarts; to start fresh, stop appos dev, then
  delete that folder. Stop it with Ctrl-C, or, running in the background,
  with kill and the process id it prints when it starts. Errors
  from the app and the shell are printed to the terminal. If the port is
  taken, appos dev says so before building; pick another with --addr. Run
  one appos dev per app: if it is already running for this app, appos dev
  says where and exits, because the running one picks up every change.

TESTING
  cargo test runs the app's unit tests on your machine, without the shell;
  make check also checks formatting and lints.
  In the hello world every file tests its layer at its bottom: the rules,
  the usecases (with fake IO) and the templates.

TYPESCRIPT
  appos new <name> --ts creates the same hello world in TypeScript. It needs
  Node.js 24 or later and zig. The first build runs npm install, which puts
  scriptc, the compiler, into node_modules/. scriptc compiles ahead of time,
  with no JavaScript engine in the app, and appos build turns its output
  into the same kind of WebAssembly component as a Rust app: everything
  else in this guide holds, kv, the limits and the grants included.
  The files that differ from FILES:
  package.json         scriptc, pinned; npm test runs the unit tests
  tsconfig.json        strict TypeScript; imports end in .ts
  src/lib.ts           your app: init() and start(); MODULES lists the modules
  src/hello/           a module: routes (index.ts), handlers (web.ts), pages
                       (view.ts), one usecase per file, IO (db.ts), data and
                       rules, including input checks (domain.ts); tests next
                       to each file (domain.test.ts)
  src/appos.ts         the SDK: get/post/register/dispatch, param/query/form,
                       url, html templates, getJson/setJson/scanJson, newId
                       (time-ordered ids for kv keys). `appos sync` replaces
                       it, so keep your own code elsewhere
  src/style.ts         the design, as a string
  bindings/            generated from wit/ by every build, typed:
                       page.setHtml, kv.get, log.info. A function the
                       platform adds to its contract is there after the next
                       build. Don't edit it; it stays out of git
  Pages are html templates instead of Askama: html`<p>${name}</p>` escapes
  every value; another template, a list of them, or raw(text) go in as they
  are. A part htmx swaps is a function of its own in view.ts. The request:
  param(req, "id"), query(req, "q"), form(req, "name"), req.htmx,
  url(req, "/tasks"), req.now (RFC 3339 text). An outcome the data decides
  is in the usecase's result, { ok: false, error: "not-found" }, and the
  handler switches on every case; a thrown error is a bug and stops the
  request like a panic (ERRORS). getJson(key) returns { ok: true, value }
  or, for stored text that isn't JSON, Unreadable: { ok: false, error:
  "unreadable", why }. The usecase returns it as it is and the handler
  answers unreadable(result).
  scriptc compiles most of TypeScript, not all of it: the build names what
  it can't compile. The tests run under Node, which can't run parameter
  properties (constructor(readonly x)), enums or namespaces: don't use them.
  make check runs appos build (which type-checks) and npm test.

JAVA
  appos new <name> --java creates the same hello world in Java 21. It needs
  a JDK 21 or later and Maven; the first build downloads TeaVM, the
  compiler, so it needs the network. TeaVM compiles ahead of time to
  WebAssembly with GC, with no JVM and no JavaScript engine in the app, and
  appos build turns its output into the same kind of WebAssembly component
  as a Rust app: everything else in this guide holds, kv, the limits and
  the grants included.
  The files that differ from FILES:
  pom.xml              the build: TeaVM in the profile wasm, which appos build
                       turns on, and JUnit. A library added here is compiled
                       into the app; the build names what TeaVM can't
                       compile (reflection, files, the network)
  src/main/java/app/App.java
                       your app: init() and start(); MODULES lists the modules
  src/main/java/app/hello/
                       a module: routes (Hello.java), handlers (Web.java),
                       pages (View.java), one usecase per file
                       (GreetUsecase.java), IO (Db.java, package-private),
                       data and rules, including input checks (Name.java,
                       Greeting.java)
  src/main/java/app/Appos.java
                       the SDK: get/post/register/dispatch, param/query/form,
                       url, html templates, Json, getJson/setJson/scanJson,
                       newId (time-ordered ids for kv keys). `appos sync`
                       replaces it, so keep your own code elsewhere
  src/main/java/app/Style.java
                       the design, as a string
  src/test/java/       JUnit tests, in the package of the code they test
  bindings/            generated from wit/ by every build: Wit.Page.setHtml,
                       Wit.Kv.get, Wit.Log.info. A function the platform
                       adds to its contract is there after the next build.
                       Don't edit it; it stays out of git
  Pages are html templates instead of Askama: html("<p>{}</p>", name)
  escapes every value; another template, a list of them, or raw(text) go in
  as they are, and text blocks hold the longer ones. A part htmx swaps is a
  method of its own in View.java. The request: param(req, "id"),
  query(req, "q") and form(req, "name") as Optional, req.htmx(),
  url(req, "/tasks"), req.now() (RFC 3339 text). What a visitor typed is
  parsed into a type of its own: Name.parse returns Parsed.Ok or
  Parsed.Invalid. An outcome the data decides is a record of the usecase's
  sealed result (ShowResult.NotFound), and the handler switches on every
  case, so a new case doesn't compile until it is handled; a thrown
  exception is a bug and stops the request like a panic (ERRORS).
  getJson(key, Greeting::fromJson) returns Read.Found, Read.Missing or, for
  stored text that isn't what fromJson expects, Read.Unreadable(why): the
  usecase returns it in its result and the handler answers failed(why).
  Each request starts the app fresh, so a call that loads Java's Unicode
  tables pays for it every time: String.strip costs about 1.7 ms,
  String.trim nothing.
  make check runs appos build and mvn test (the tests run on the JVM).
"#;

const HELP_NEW: &str = "appos new <name> [--group <team>] [--ts | --java]

Creates ./<name> with a working hello world: a route, a menu entry, an Askama
page, an htmx update and state in kv. <name> becomes the app id and its URL
(/<name>): lowercase letters, digits and dashes. With --ts, the same in
TypeScript, compiled with scriptc (appos guide typescript); with --java, in
Java, compiled with TeaVM (appos guide java).

app.toml gets the owner from git (user.name and user.email) and the group
from --group: the team the app belongs to, like \"Marketing\". The admin
screen groups apps by it. A deploy needs both; set them in app.toml.

If a platform is current (appos platforms), the app is built against that
platform's contract (wit/); otherwise against the one built into appos, which
is all you need for local development. src/appos.rs (src/appos.ts,
src/main/java/app/Appos.java) is the SDK of this appos; `appos sync`
updates both later.
";

const HELP_DEV: &str = "appos dev [--addr 127.0.0.1:8080]

Builds the app in this directory and runs it in a local shell at
http://<addr>/<app-id>. Watches src/, templates/, wit/, .cargo/, app.toml,
Cargo.toml, Cargo.lock, build.rs, rust-toolchain.toml, icon.svg and, for a
TypeScript app, package.json, package-lock.json and tsconfig.json, for a Zig
app build.zig and build.zig.zon, for a Java app pom.xml. File edits,
additions and deletions rebuild and reload the app. If a build fails, the last good version keeps
running. kv data is kept in ./appos-data, with each route's last request
that worked, for appos review. Stop with Ctrl-C; in the background, with
kill and the process id it prints when it starts.

It warns when the SDK (src/appos.rs, src/appos.ts or
src/main/java/app/Appos.java) or wit/ differ from this appos's; `appos
sync` updates them.

One dev server per app: if one is already running for this app, appos dev
prints its address and exits. If the port is taken by something else, it
says so before building; use --addr.
";

const HELP_REVIEW: &str = "appos review [--time 30s]

Builds the app in this directory and has appfuzz (https://appfuzz.dev)
run it many thousands of times against a simulated shell: every route,
with changed requests and stored data, looking for crashes, requests over
the shell's limits, request or stored text that reaches the page as
markup, and more. It prints what it found. Exit code 0: no findings; 1:
warnings; 2: failures; 3: the review could not finish.

The shell writes what appfuzz simulates: what app.toml grants, what each
kv call costs, and the I/O budget, fuel and memory of one request. The
data and the requests come from appos dev: the app's kv in ./appos-data
and, for each route, the last request that worked there. A route not
opened in appos dev gets a request made from its pattern (:name becomes
\"name\"), and review lists it: use the app in appos dev first, so the
review reaches every page.

--time is how long appfuzz searches; without it, appfuzz's default. The
files stay in ./appos-data/review (appfuzz.toml, plan.json and appfuzz's
runs): appfuzz --human findings there shows the findings again.

Needs appfuzz 0.8.0 or newer: curl -fsSL https://appfuzz.dev/install.sh | sh
APPOS_APPFUZZ names another appfuzz binary.
";

const HELP_BUILD: &str = "appos build

Builds the app in this directory (cargo build --release --target
wasm32-wasip2; for a TypeScript app, scriptc and the bindings in
bindings/; for a Zig app, one with a build.zig, zig build --release with
bindings/wit.zig; for a Java app, one with a pom.xml, mvn -P wasm
process-classes, TeaVM, with bindings/appos/wit/Wit.java) and prints where
the .wasm went.
";

const HELP_LOGIN: &str = "appos login <url>

Connects to an AppOS shell's platform API at <url>, saves it (in
~/.config/appos/config.toml, or $APPOS_CONFIG) and makes it the current
platform. Not needed for local development.

A shell with sign-in takes a token: make one on its admin screen at
<url>/admin/token, signed in as an admin, and paste it when asked, or
pipe it in (echo \"$TOKEN\" | appos login <url>). It acts as you, while the
shell's policy makes you an admin, for 30 days. A token kept from before
is used again while it works.
";

const HELP_LOGOUT: &str = "appos logout

Ends the current platform's token, on the platform and here.
";

const HELP_PLATFORMS: &str = "appos platforms

Lists the platforms appos knows; * marks the current one.
";

const HELP_USE: &str = "appos use <name>

Makes a known platform the current one.
";

const HELP_HOSTS: &str = "appos hosts [--wit]

Shows what the current platform offers: the capabilities an app can be
granted and the apps running there. --wit prints the full contract.
";

const HELP_DEPLOY: &str = "appos deploy [app.zip]

Builds the app in this directory and sends it to the current platform
(see `appos platforms`) as app.zip: app.toml, the .wasm, and icon.svg if
there is one. With a file, sends that app.zip instead, as `appos pack`
makes it, so an app built elsewhere (CI, another machine) deploys without
building. The platform checks it as it does at startup and makes it live:
the next request gets the new version, and the data in kv stays. Prints
the version and the app's address. Connect first with `appos login <url>`.

app.toml must name an owner under [app], the person or team to ask about
the app: owner = \"Jane Doe <jane@example.com>\", and the group it belongs
to: group = \"Marketing\". The admin screen shows both with every version
and groups the apps by group.

Every version is kept. If the live version keeps failing (5 failed
requests within a minute), the platform rolls back to the version before
it by itself. After that, deploys are stored but not made live until an
admin makes a version live on the admin screen; the CLI prints where.

Before deploying, it tells what happened to the app since you last looked
(here or with `appos versions`): rollbacks, refused deploys, versions an
admin made live, the app turned off or on, and who did it. It warns when
the app's wit/ isn't the platform's contract, or its SDK (src/appos.rs and
the like) isn't the one of this appos; `appos sync` updates both.
";

const HELP_PACK: &str = "appos pack [file]

Builds the app in this directory and writes it as one file, app.zip:
app.toml, the .wasm, and icon.svg if there is one. Writes target/app.zip,
or the file given. `appos deploy app.zip` sends it to a platform.
`appos deploy` alone builds, packs and sends in one go; pack is for
deploying later or from somewhere else.

The same app gives the same bytes, so the zip of a commit can be compared.
app.toml must name an owner and a group, as for a deploy (appos deploy --help).
";

const HELP_VERSIONS: &str = "appos versions

Lists the versions of the app in this directory on the current platform,
newest first, one line each: the version, its status and when it was
deployed, with the reason on the next line when there is one.

  live       the one requests go to
  retired    was live; `appos rollback <version>` makes it live again
  failed     rolled back automatically: it kept failing
  refused    never ran: the platform said no at deploy, and why
  waiting    stored after an automatic rollback; an admin makes it live

A version is the version from app.toml followed by 32 hex digits from a
hash of all bundle files, including the icon. Every changed bundle is a
new version.

First it tells what happened to the app since you last looked, as
`appos deploy` does: rollbacks, refused deploys, versions an admin made
live, the app turned off or on, and who did it.
";

const HELP_LOG: &str = "appos log [--failures] [--before <n>]

Shows what happened to the app in this directory on the current platform,
newest first, 20 at a time: deploys, refused deploys, rollbacks, limits hit
and the app turned off or on, each with when, the version, how bad and who
did it. An automatic rollback or a limit hit lists the failed requests
behind it with their errors: when the app failed, start there.

  --failures      only refused deploys, automatic rollbacks and limit hits
  --before <n>    the page after this one: the last line says the number

The same as JSON, for scripts: <platform>/_platform/log?app=<app-id>
(also kind, severity, by, from, to, q, before, limit).
";

const HELP_ROLLBACK: &str = "appos rollback [version] [--reason <text>]

Makes an earlier version of the app in this directory live again: the one
given (see `appos versions`), or else the one deployed before the live one.
The data in kv stays as it is.

--reason says why, like --reason \"the new form loses leads\". It is kept
with the rollback in the platform's history, where admins see it.

A version waiting after an automatic rollback goes live only from the admin
screen.
";

const HELP_SWITCH: &str = "appos off [--reason <text>]
appos on [--reason <text>]

`appos off` turns the app in this directory off on the current platform:
every route answers with a page saying it is turned off (status 503), and
nothing of it runs. Its versions and kv data stay; deploys and rollbacks go
on as usual, behind the switch. `appos on` turns it on again.

--reason says why, like --reason \"the form collects too much\". It is kept
in the platform's history, where admins see it.

An app an admin turned off goes back on only from the admin screen.
";

const HELP_SYNC: &str = "appos sync

Brings the app in this directory up to date with this appos: writes its
SDK as src/appos.rs (src/appos.ts in a TypeScript app,
src/main/java/app/Appos.java in a Java app; a Zig app keeps its own), and
the contract of the current platform (or the one
built into appos) into wit/, removing contract files it doesn't have.
Nothing else changes: change your own code to the new SDK where the build
says so. `git diff` shows what changed.

appos dev and appos deploy warn when the SDK or wit/ differ from what the
app runs against. Keep your own code out of the SDK: sync replaces it.
";

const HELP_GUIDE: &str = "appos guide [section]

Prints how apps work: routes, the page, htmx, storage, styles, rules.
With a section, only that one: files, lifecycle, request, page, templates,
htmx, forms, storage, errors, styles, rules, git, dev, testing, typescript,
java.
";

/// A command's help, for `appos <command> --help`.
pub fn command(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "new" => HELP_NEW,
        "sync" => HELP_SYNC,
        "dev" => HELP_DEV,
        "build" => HELP_BUILD,
        "review" => HELP_REVIEW,
        "guide" => HELP_GUIDE,
        "login" => HELP_LOGIN,
        "logout" => HELP_LOGOUT,
        "platforms" => HELP_PLATFORMS,
        "use" => HELP_USE,
        "hosts" => HELP_HOSTS,
        "deploy" => HELP_DEPLOY,
        "pack" => HELP_PACK,
        "versions" => HELP_VERSIONS,
        "log" => HELP_LOG,
        "rollback" => HELP_ROLLBACK,
        "off" | "on" => HELP_SWITCH,
        _ => return None,
    })
}

/// The guide's sections by the name `appos guide <section>` takes, and
/// their headings.
const SECTIONS: &[(&str, &str)] = &[
    ("files", "FILES"),
    ("lifecycle", "LIFECYCLE"),
    ("request", "REQUEST"),
    ("page", "THE PAGE"),
    ("templates", "TEMPLATES"),
    ("htmx", "HTMX"),
    ("forms", "FORMS"),
    ("storage", "STORAGE"),
    ("errors", "ERRORS"),
    ("styles", "STYLES"),
    ("rules", "RULES"),
    ("git", "GIT"),
    ("dev", "LOCAL DEVELOPMENT"),
    ("testing", "TESTING"),
    ("typescript", "TYPESCRIPT"),
    ("java", "JAVA"),
];

/// `appos guide [section]`: all of it, or one section.
pub fn guide(name: Option<&str>) -> anyhow::Result<()> {
    let Some(name) = name else {
        print!("{GUIDE}");
        return Ok(());
    };
    match SECTIONS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        Some((_, heading)) => print!("{}", section(GUIDE, heading).unwrap_or_default()),
        None => anyhow::bail!("no section {name:?}; the guide has {}", SECTIONS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")),
    }
    Ok(())
}

/// A section of the guide: its heading line up to the next heading. A
/// heading starts at the margin with a word in capitals: "TEMPLATES (ASKAMA)".
fn section<'a>(guide: &'a str, heading: &str) -> Option<&'a str> {
    let is_heading =
        |line: &str| line.split_whitespace().next().is_some_and(|w| w.len() > 1 && w.chars().all(|c| c.is_ascii_uppercase())) && !line.starts_with(' ');
    let (mut start, mut end) = (None, guide.len());
    let mut at = 0;
    for line in guide.split_inclusive('\n') {
        let text = line.trim_end();
        if start.is_some() && is_heading(text) {
            end = at;
            break;
        }
        if start.is_none() && (text == heading || text.starts_with(&format!("{heading} ("))) {
            start = Some(at);
        }
        at += line.len();
    }
    Some(&guide[start?..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every section prints on its own, from its heading to the next, and
    /// together they are the whole guide after the intro.
    #[test]
    fn every_guide_section_prints_alone() {
        let mut total = 0;
        for (name, heading) in SECTIONS {
            let text = section(GUIDE, heading).unwrap_or_else(|| panic!("no section {name}"));
            assert!(text.starts_with(heading), "{name}: {text:.40}");
            total += text.len();
        }
        assert_eq!(total, GUIDE.len() - GUIDE.find("\nFILES").unwrap() - 1);
        // RULES in the intro's prose is not the RULES section.
        assert!(section(GUIDE, "RULES").unwrap().starts_with("RULES\n"));
    }

    /// The guide's intro and `appos guide --help` name every section.
    #[test]
    fn the_section_lists_are_complete() {
        for text in [GUIDE.split("\nFILES").next().unwrap(), HELP_GUIDE] {
            let listed = text.split_once("one of ").or(text.split_once("only that one: ")).unwrap().1.split_once(".\n").unwrap().0;
            let listed: Vec<_> = listed.split(',').map(str::trim).collect();
            assert_eq!(listed, SECTIONS.iter().map(|(n, _)| *n).collect::<Vec<_>>());
        }
    }

    /// The guide's numbers are the shell's limits.
    #[test]
    fn the_guide_states_the_shells_limits() {
        use appos_shell::api::limits::*;
        let guide = GUIDE.split_whitespace().collect::<Vec<_>>().join(" ");
        let billions = ["", "1 billion", "2 billion", "3 billion", "4 billion"];
        for said in [
            format!("{IO_BUDGET} I/O units"),
            format!("fuel ({} instructions)", billions[(FUEL / 1_000_000_000) as usize]),
            format!("{} MB memory and {SECONDS} seconds", MEMORY >> 20),
            format!("up to {MAX_SECONDS}:"),
            format!("(up to {MAX_SCAN})"),
            format!("scan of {MAX_SCAN} keys"),
            format!("more than {} MB of keys", MAX_SCAN_BYTES >> 20),
            format!("Keys up to {} KB, values up to {} MB", MAX_KEY >> 10, MAX_VALUE >> 20),
            format!("up to {} KB", MAX_ICON >> 10),
            format!("{IO_BUDGET} I/O units. kv::get"),
        ] {
            assert!(guide.contains(&said), "the guide doesn't say {said:?}");
        }
        assert_eq!(FUEL % 1_000_000_000, 0, "say the fuel in words");
    }

    /// The guide's LOCAL DEVELOPMENT and `appos dev --help` name what
    /// appos dev watches (sources.rs).
    #[test]
    fn the_watch_lists_name_every_watched_source() {
        let dev = section(GUIDE, "LOCAL DEVELOPMENT").unwrap().split_whitespace().collect::<Vec<_>>().join(" ");
        let help = HELP_DEV.split_whitespace().collect::<Vec<_>>().join(" ");
        for watched in crate::sources::WATCHED {
            assert!(dev.contains(watched), "the guide doesn't say appos dev watches {watched}");
            assert!(help.contains(watched), "appos dev --help doesn't say it watches {watched}");
        }
    }
}
