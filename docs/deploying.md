# Deploying

How apps reach a platform, its versions and its log. Back to the [README](../README.md).

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

An app travels as one file, app.zip: app.toml, the .wasm, icon.svg and,
for a Rust or TypeScript app, deps.json, the packages compiled into it (see
[Gardening](operations.md#gardening)). `appos deploy` builds and packs it in memory; `appos
pack` writes it (`target/app.zip`, the same bytes for the same app), and
`appos deploy app.zip` sends one built elsewhere, by CI or another
machine. app.toml
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

An app whose grants let it keep data (`kv-write`, or any kit's grant) says
what it keeps, where and why, one `[[data]]` entry for each kind of data,
so the people who answer for the company's data (GDPR) can find it; a
deploy without them is refused. `where` is `kv` or the kit the app hands
the data to:

```toml
[[data]]
what = "Name and email address of each sign-up"
where = "kv"
why = "Hold seats and send the webinar link"
```

The admin screen shows the entries with the app, and
`/_platform/data` lists them for every running app as JSON. They are what
the app's builder says. The review (Hardening in the README) checks the
ones in kv: with `where = "kv"`, what the app reads from kv is personal
data, and it may go to the page and back into kv, but not into logs,
error pages or other calls to the platform.

The shell checks a deployed app as it does at startup, keeps its app.zip
in `<data>/apps/<app-id>/<version>/` and makes it live; its kv data stays.
A version is the version from app.toml followed by 32 hex digits from a
SHA-256 hash of app.toml, the .wasm, icon.svg and deps.json. Stored
versions are immutable: changing an icon creates a new version too.
Versions use 1–200 ASCII letters, digits, dots, underscores, dashes or plus
signs, so their stored bundles stay inside the app's directory.

A shell started with `--review` reviews each app deploy before it goes
live. The deploy is stored and waits while the shell writes appfuzz.toml,
plan.json and the stored .wasm into
`<data>/apps/<app-id>/<version>/review/` and runs the command there, one
review at a time, for at most 30 minutes. Exit 0 makes the version live;
anything else refuses it, with the end of review.log as the reason. The
review runs on the bytes that go live, so it holds for that version's
hash. The review gets no data from the shell's kv, which holds people's
data. appfuzz is one way to review; any program that reads these files
and exits 0 when the app passes will do:

```sh
appos-shell --review 'appfuzz review --plan plan.json'   # a program and its arguments, no shell
```

While the review runs, the version is waiting; an admin can still make it
live on the admin screen. Kits aren't reviewed: deploying one needs
admin-write.

```sh
appos versions             # live, retired, failed, refused or waiting, and why
appos rollback             # back to the version deployed before the live one
appos rollback 0.1.0-3f2a9c1d0e4b5867a8b9c0d1e2f30415 --reason "the new form loses leads"
```

If the live version fails 5 requests within a minute (timeouts, over
budget, crashes), the shell rolls back to the version before it by itself.
Then deploys wait: they are stored but go live only once an admin makes a
version live on the admin screen.

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

The admin screen and the platform API are for those the policy gives
admin-read, to look, and admin-write, to change; admin-deploy and
admin-off give deploying and turning off for some apps only, through the
API (see [Who may do what](access.md#who-may-do-what)). The API takes a
token (see Platforms).
The shell answers only under its own names: the host of `--origin`, of the
platform listener, localhost and IP addresses, so a web page can't reach
a shell on your machine under a name of its own (DNS rebinding). Behind a
proxy, give `--origin` the address people use.
Browser writes to app routes and platform endpoints must come from the
same origin. A TLS terminating reverse proxy should set `X-Forwarded-Proto`
to `https`.
