# AppOS

A shell that runs WebAssembly apps, written by anyone (or their agents),
behind one layout. The shell owns the page and the data; apps get only the
capabilities their manifest grants, and people only what the platform's
policy lets them (see [Who may do what](#who-may-do-what)).

```sh
make            # needs Rust 1.100+, Python 3 and: rustup target add wasm32-wasip2
make run        # the shell with the hello world at http://127.0.0.1:8080
```

`make` builds the shell and the CLI, the hello world and the Rust example
apps (each with its own check), and runs the checks and regressions.
`make typescript`, `make zig` and `make java` check the other languages.

## Building an app

```sh
appos new events --group Marketing  # a working hello world in ./events
                                    # (--ts: TypeScript, --java: Java)
cd events
appos dev                           # run it at http://127.0.0.1:8080/events; changes rebuild and reload
appos guide                         # how apps work
appos review                        # appfuzz runs it against a simulated shell
```

`appos review` is the app review: the shell writes the appfuzz manifest
(grants, kv costs, the I/O budget, fuel, memory) and takes the data and
one request per route from what you did in `appos dev`. Needs
[appfuzz](https://appfuzz.dev).

`appos` with no arguments explains itself. Telling an agent "use the appos
CLI to build an app" is meant to be enough.

## Platforms

A shell serves a platform API so the CLI can build against it, like a
kubectl context. Bind it to an internal address to keep it off the public
site:

```sh
appos-shell --name acme --addr 0.0.0.0:8080 --platform-addr 10.0.0.5:9090 \
            --git https://gitlab.acme.com/marketing-apps --risk risk.toml \
            --auth auth.toml --policy policy.csv apps/*
```

On a developer's machine, either of these connects:

```sh
curl -fsSL http://10.0.0.5:9090/_platform/install | sh
appos login http://10.0.0.5:9090
```

A shell with sign-in takes a token on its platform API. `appos login` asks
for one: an admin makes it on the admin screen at `/admin/token` and
pastes it (or pipes it in, for CI). The token acts as them, checked against
the policy at every call, so taking someone out of the policy stops their
tokens at once. It lasts 30 days; `appos logout` ends it sooner. Only its
hash is kept. With `--no-auth`, the API takes no token.

Then `appos hosts` shows what the platform offers, `appos new` writes its
contract (`wit/`) into new apps, and `appos sync` into existing ones
(`appos deploy` warns when an app's contract differs). With `--git`, `appos new` also tells the
user (or their agent) to push the app to `<git>/<app-id>.git`; appos never
runs git itself, it only says how to use it. Platforms are kept in
`~/.config/appos/config.toml` (or `$APPOS_CONFIG`); `appos platforms` and
`appos use <name>` switch between them. Without `--platform-addr`, the API
is served on the main address under `/_platform`.

## Deploying

A shell on one port, site and platform API together, and an app deployed
to it:

```sh
appos-shell --name demo              # http://127.0.0.1:8080; prints a link that signs you in as admin
appos login http://127.0.0.1:8080    # open the link, then paste a token from /admin/token
cd calculator && appos deploy        # http://127.0.0.1:8080/calculator
```

An app travels as one file, app.zip: app.toml, the .wasm and icon.svg.
`appos deploy` builds and packs it in memory; `appos pack` writes it
(`target/app.zip`, the same bytes for the same app), and `appos deploy
app.zip` sends one built elsewhere, by CI or another machine. app.toml
names the app's owner, the person or team to ask about it, and its group,
the team it belongs to; the admin screen groups apps by it. A deploy
without either is refused:

```toml
[app]
id = "calculator"
name = "Calculator"
version = "0.1.0"
owner = "Jane Doe <jane@example.com>"   # appos new fills it in from git
group = "Marketing"                     # appos new --group Marketing
```

The shell checks a deployed app as it does at startup, keeps its app.zip
in `<data>/apps/<app-id>/<version>/` and makes it live; its kv data stays.
A version is the version from app.toml followed by 32 hex digits from a
SHA-256 hash of app.toml, the .wasm and icon.svg. Stored versions are
immutable: changing an icon creates a new version too.
Versions use 1–200 ASCII letters, digits, dots, underscores, dashes or plus
signs, so their stored bundles stay inside the app's directory.

```sh
appos versions             # live, retired, failed, refused or waiting, and why
appos rollback             # back to the version deployed before the live one
appos rollback 0.1.0-3f2a9c1d0e4b5867a8b9c0d1e2f30415 --reason "the new form loses leads"
```

If the live version fails 5 requests within a minute (timeouts, over
budget, crashes), the shell rolls back to the version before it by itself.
Then deploys wait: they are stored but go live only once an admin makes a
version live on the admin screen. The builder can't ship over a failure an
admin hasn't seen.

`appos deploy` and `appos versions` first tell what happened to the app
since the last look: automatic rollbacks, refused deploys, versions an
admin or `appos rollback` made live, each with the reason.

```sh
appos log                  # everything that happened to the app, newest first
appos log --failures       # refused deploys, automatic rollbacks, limits hit
```

An automatic rollback or a limit hit lists the failed requests behind it
with their errors, so a builder or their agent can see why the app failed.
Scripts get the same as JSON from `/_platform/log?app=<app-id>`.

The admin screen and the platform API are for those the policy makes
admins (see Who may do what); the API takes a token (see Platforms).
The shell answers only under its own names: the host of `--origin`, of the
platform listener, localhost and IP addresses, so a web page can't reach
a shell on your machine under a name of its own (DNS rebinding). Behind a
proxy, give `--origin` the address people use.
Browser writes to app routes and platform endpoints must come from the
same origin. A TLS terminating reverse proxy should set `X-Forwarded-Proto`
to `https`.

## Admin screen

`/admin`, next to the platform API, one page. On top a console: what
needs you, one app at a time (Seen clears it until something new
happens); a screen with every app, its group and its last 16 minutes (or
hours, days), needs-you first, turned-off apps in red, Find in its first
line narrowing it by name, owner or group, an app's story, one minute's requests,
its versions, a map by group. Left, browse: the App knob (a light per app, A to Z;
click one for that app), Group (one group's apps) and Range. Right, what to
do with the app (its story, requests, versions), its on/off slide
(green LED while visitors get it, red once turned off) and three
faders for its limits per request: memory, time and fuel (instructions).
Bottom right, the platform: a power button (green while apps run) turns
every app off, and back on, and the Freeze slide (up, amber) holds every new deploy for an admin. Like
on a device, a control acts at once; each change is logged as admin and
can be undone from its toast for ten minutes. Below: the last 30 days in numbers (apps live, deploys,
what the platform stopped, time to recovery), the last hour in charts,
one app in detail (its icon (`icon.svg`, sent in app.zip), owner, routes,
its grants and their risk, versions with "Make live", last failures,
"Turn off"), the inventory of all apps with their groups and owners, and what was
refused, rolled back or turned off. It is live: within a
second of a deploy, rollback or turn-off, and every few seconds while
requests come in. The log, `/admin/log`: every event, newest first,
filtered by text, app, kind, severity, who and days.

The shell keeps it all in `<data>/platform.db`, separate from the apps' kv:
every version, every event (deploys, refusals, rollbacks, limit hits, risk
changes, limits set) and the requests per app and hour, all for good; of
the requests themselves, the newest 5000 per app.

## Risk

The company that runs the shell rates each grant from 0 (harmless) to 10,
in a file it keeps; apps can't change it:

```toml
# risk.toml, used with: appos-shell --risk risk.toml
kv-read = 1
kv-write = 2
```

An app is as risky as its riskiest grant; a grant without a level leaves
the app unrated. The admin screen shows each app's risk, `appos hosts` the
grants'. What a level means, such as who may use grants above 5, is up to
the company.

## Who may do what

An app's grants say what the app may do. The platform's policy says which
people may make it do that, under the same names. A host call goes through
only if both allow it; apps don't see who is signed in and check nothing.

```sh
appos-shell --auth auth.toml --policy policy.csv
```

The two files go together. Without them the shell runs on this machine
only: it refuses an address or `--origin` other machines reach, and prints
a link at each start, with a token that is new every time. The link signs
in as `admin@localhost`, who may use every grant, the admin screen and the
platform API; nobody else may use anything. `--no-auth` runs the shell
open: anyone may do anything, on the admin screen and the platform API
too; it says so at the start. `appos dev` runs that way, on localhost.

```toml
# auth.toml: how people sign in (dev, header, oidc or plugin; see Sign-in)
provider = "dev"         # anyone signs in as anyone; localhost only
session-hours = 8        # the default
```

```
# policy.csv: p, who, where, what
p, anyone, webinars, kv-read
p, signed-in, webinars, kv-write
p, group:sales, discounts, kv-*
p, group:marketing, apps:Marketing, kv-read
p, alice@acme.com, pricing, kv-read
# the admin screen
p, group:platform, *, admin
# g, who, group: puts someone in a group
g, bob@acme.com, group:platform
```

Who: an email, `group:<name>` (from the sign-in, or given with `g`),
`signed-in` or `anyone`. Where: an app id, `apps:<group>` (the group in
app.toml) or `*`. What: a grant, a pattern like `kv-*`, or `admin`. What no
line allows is denied. The rules
are [casbin](https://casbin.org)'s, with a model built into the shell.

A host call the policy denies ends the request and drops its writes. A
visitor who isn't signed in is sent to sign in and comes back to the page;
one who is gets "Not allowed", naming the app's owner. A denial is not the
app failing: it never counts toward an automatic rollback.

The shell checks the file every 2 s and reads it again when it changes; the
admin log shows each change. A file with a mistake is refused, with the
line, and the policy stays as it was; at startup it stops the shell.

The admin screen is for those with `admin`; anyone else is sent to sign in
or gets "Not allowed". It shows who is signed in, logs each change with
their email ("Admin screen, ann@acme.com"), and lists the policy under
Policy, as sentences by who, with a file that was refused and why. With
`--platform-addr`, sign-in for the admin screen runs on that address: an
OIDC identity provider needs `http://<platform-addr>/_auth/callback`
registered too.

## Sign-in

auth.toml names one provider. The shell keeps the session in a signed
cookie, so a provider runs only while someone signs in.

```toml
provider = "header"                     # a proxy in front signs people in
trusted-proxies = ["10.0.0.2"]          # its headers count only from here
email-header = "X-Forwarded-Email"      # these three are the defaults
name-header = "X-Forwarded-User"
groups-header = "X-Forwarded-Groups"    # comma separated
sign-in-url = "/oauth2/start?rd={to}"   # {to}: the page to come back to
sign-out-url = "/oauth2/sign_out"
```

```toml
provider = "oidc"                       # Keycloak, Entra ID, Okta, Google, ...
issuer = "https://login.acme.com"
client-id = "appos"
client-secret-env = "APPOS_OIDC_SECRET" # the secret is never in the file
scopes = ["openid", "email", "profile"] # the default
groups-claim = "groups"                 # where the groups are, if anywhere
```

The identity provider sends people back to `<origin>/_auth/callback`.

### Your own sign-in

Anything else is a sign-in plugin: a WebAssembly component of the world
`appos:auth/sign-in` ([wit-auth/auth.wit](wit-auth/auth.wit), also in
`/_platform/spec` as `sign_in`). It exports `start` (show a page or send the
browser to the identity provider) and `finish` (the form it posted, or the
callback; the shell has checked the state) and returns a page, a redirect,
an identity, or why not.

```toml
provider = "plugin"
wasm = "acme-auth.wasm"                 # found next to auth.toml
hosts = ["login.acme.com"]              # the only hosts it may call
[config]                                # what config.get returns
tenant = "acme"
[config-env]                            # secrets: from the environment
client-secret = "ACME_SECRET"
```

The plugin is sandboxed like an app: no files, no environment, no storage.
It may call `config.get` and `http.fetch`, which reaches only `hosts`, over
https (http only to localhost), and follows no redirects. Each step runs in a
fresh instance with an app's fuel and memory, for at most 10 s. A plugin
that doesn't load stops the shell at startup; one that fails while someone
signs in shows "Sign-in isn't working right now", and the site goes on.

[examples/auth-github](examples/auth-github) signs in the members of one
GitHub organization, with their teams as groups: Rust and wit-bindgen,
`make build` makes `auth_github.wasm`.

## Limits

Each request gets 1 billion instructions (fuel), 64 MB memory, 100 I/O
units and 2 seconds; an app may ask for up to 30 seconds in app.toml
(`[limits] seconds = 5`). An admin can set an app's memory (16–512 MB),
time (1–30 s) and instructions (250 million–8 billion) on the admin screen;
they stay across deploys until set back. A request over a limit stops: its
visitor gets an error and it counts as a failure.
The time limit stops an app wherever it is, also
while it waits in a host call.
Database reads and commits run off the async worker threads. Waiting for
the KV connection or a SQLite writer lock uses the request's remaining
time; an expired transaction rolls back its writes.
The memory budget covers all Wasm memories in the request together;
tables share a limit of 100,000 elements. Request bodies are limited to
256 KB; unreadable or invalid UTF-8 form bodies are refused.
Page output (including its head and fragment names) and init's route/menu
metadata each have a 4 MB budget. A KV scan returns at most 1,000 rows and
4 MB of keys and values, after merging the request's own writes.

App logs and platform responses quote terminal control characters when
printed. Every physical app log line carries its app id, and rendered
output is capped at 64 KB per request. Native logging runs on a bounded
worker queue: a closed or stalled collector may drop console messages,
without blocking requests or losing the error details kept by the platform.

## How an app works

An app is a WebAssembly component with two functions (`wit/appos.wit`):

- `init()` runs once when the shell loads the app. It registers routes
  (with `:param` segments) and menu entries.
- `start(handler, request)` runs once per request in a fresh sandbox. Like it
  calls `log` and `kv`, the app builds the page with host calls:
  `page::set-html(name, html)` ("page" is the whole page content),
  `page::add-header(html)` for `<head>` (e.g. the app's own `<style>`),
  `page::redirect`, `page::set-status`, and finally `page::done()`: the app stops
  there and is unloaded, and the shell sends the result to the browser.
  - For htmx requests, every other name replaces the content of the element
    with that id.

`{% block name %}` doubles as the fragment `name`: list it in
`#[template(..., blocks = ["name"])]` and render it with `t.as_name()`, so one
template serves the full page and its htmx updates.

History restoration requests receive the full page, including the shell
layout. `req.htmx` is true for fragment swaps and false for full-page
loads, including history restoration. Responses vary by `HX-Request` and
`HX-History-Restore-Request` so caches keep these representations separate.

Apps are mounted at `/<app-id>/`. The manifest is small. `kv-read` grants
`kv::get` and `kv::scan`, `kv-write` grants `kv::set` and `kv::delete`; an
app that imports a kv function its manifest doesn't grant is refused:

```toml
[app]
id = "events"
name = "Events"
version = "0.1.0"
wasm = "target/wasm32-wasip2/release/events.wasm"

[capabilities]
grant = ["kv-read", "kv-write"]
```

## Layout

```
wit/appos.wit         the contract between shell and apps
wit-auth/auth.wit     the contract between shell and sign-in plugins
shell/                the shell: library + appos-shell binary
                      (axum, wasmtime, SQLite for kv and the platform
                      database, platform API, admin screen)
cli/                  the appos CLI; runs the same shell for `appos dev`
cli/template/         the hello world every new app starts from
cli/template-ts/      the same in TypeScript (appos new --ts)
cli/template-java/    the same in Java (appos new --java)
examples/             four apps (pricing, webinars, status, discounts) in
                      Rust, TypeScript and Zig, each from a request; see
                      its README
examples/auth-github/  a sign-in plugin: GitHub, teams as groups
```

There is no SDK crate: bindings are generated in each app from its `wit/`,
and the helpers (routes and modules, the request's time and dates, kv with
JSON, form errors) are `src/appos.rs`. Both are part of the scaffold: the
`appos` binary carries them, `appos new` writes them and `appos sync`
brings an app up to date with the binary; `appos dev` and `appos deploy`
warn when they differ. The template and the Rust examples carry the same
copies, byte for byte (`cargo test -p appos` checks).
