# Operations

The admin screen, the budget of every request, and gardening the live
apps. Back to the [README](../README.md).

## Admin screen

`/admin`, next to the platform API, is one page. At its top is a console.
Its screen shows what needs you, one app at a time (Seen clears it until
something new happens), every app with its group and its last 16 minutes
(or hours, days), needs-you first and turned-off apps in red, an app's
story, one minute's requests, its versions and a map by group; Find, in its
first line, narrows it by name, owner, group or the data an app keeps. On
the left you browse: the App knob (a light per app, A to Z; click one for
that app), Group (one group's apps) and Range. On the right are what to do with the app (its
story, requests, versions), its on/off slide (a green LED while visitors
get it, red once turned off) and three faders for its limits per request:
memory, time and fuel (instructions). Bottom right is the platform: a power
button (green while apps run) turns every app off and back on, and the
Freeze slide (up, amber) holds every new deploy for an admin. As on a
device, a control acts at once; each change is logged as admin and can be
undone from its toast for ten minutes.

Below the console are the last 30 days in numbers (apps live, deploys,
what the platform stopped, time to recovery), the last hour in charts, one
app in detail (its icon from `icon.svg`, owner, routes, grants and their
risk, the data it keeps from app.toml's `[[data]]`, versions with "Make
live", last failures, "Turn off"), the inventory of all apps with their
groups and owners, and what was refused, rolled
back or turned off. The page updates within a second of a deploy, rollback
or turn-off, and every few seconds while requests come in. The log at
`/admin/log` lists every event, newest first, filtered by text, app, kind,
severity, who and days.

The shell keeps it all in `<data>/platform.db`, separate from the apps' kv:
every version, every event (deploys, refusals, rollbacks, limit hits, risk
changes, limits set) and the requests per app and hour, all for good; of
the requests themselves, the newest 5000 per app.

## Every request has a budget

An endless loop or a page that eats memory stops when its request runs
out of budget. Its visitor gets an error, the request counts as a failure
(5 within a minute roll the app back, see [Deploying](deploying.md)) and the shell and
the other apps keep running.

Each request gets 1 billion instructions (fuel), 64 MB memory, 100 I/O
units and 2 seconds; an app may ask for up to 30 seconds in app.toml
(`[limits] seconds = 5`). An admin can set an app's memory (16–512 MB),
time (1–30 s) and instructions (250 million–8 billion) on the admin screen;
they stay across deploys until set back. The time limit stops an app
wherever it is, also while it waits in a host call.
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

## Gardening

Gardening means checking the live apps regularly: what failed, whose owner
left, and which packages turned unsafe after the deploy. `appos audit`
lists the live apps that use a package with a known vulnerability, and
who owns them:

```sh
appos audit                # the live apps on the current platform
appos audit --json         # the same as JSON, for scripts
```

`appos deploy` and `appos pack` put deps.json into a Rust app's app.zip:
the crates.io crates compiled into its .wasm, as package URLs like
`pkg:cargo/serde@1.0.228`. appos reads them from Cargo.lock and the
crates' Cargo.toml as cargo resolves them for wasm32-wasip2, without
running cargo: no dev- or build-dependencies, no proc-macros, only what
the features and the target turn on.

For a TypeScript app, deps.json holds the npm packages scriptc compiles
into it (`pkg:npm/semver@7.7.3`): those `bindings/entry.ts` imports,
theirs and so on, through node_modules as Node finds them, at the
versions in package-lock.json, without running node, npm or scriptc. What
scriptc serves itself (Node's builtins, polyfills of them) isn't listed.
appos reads the imports from the syntax: an import that brings only
types says so (`import type`), and every other one loads its module. That
holds for JavaScript files, and for a TypeScript file when the nearest
tsconfig.json above it sets `"verbatimModuleSyntax": true`, or there is
none. New apps have it, and `appos sync` sets it in older ones. Packages
from git, a tarball or another registry are left out with a warning, as
their names can be another package's in the npm registry; set
`APPOS_NPM_MIRRORS` to the URLs of the registry mirrors you install from.

The shell keeps deps.json with the version; `/_platform/packages` lists
it for every live app.

`appos audit` matches these lists against the advisories OSV
(https://osv.dev) publishes for each ecosystem: for crates.io those of
RustSec and GitHub and the known malicious crates, for npm those of
GitHub and the known malicious packages. It downloads all of them once a
week into ~/.cache/appos, and the ones that changed since at most once an
hour; it sends osv.dev no package names. For each app with a vulnerable
package it names the owner, the advisory and the versions that fix it,
and it exits with 1 if there is one, so CI can run it every day.
Unmaintained and unsound crates are warnings.

Where appos can't tell which packages are built in, the app.zip goes
without deps.json and `appos deploy` and `appos pack` warn. For a Rust
app: resolver 1 (edition 2015 or 2018 without `resolver = "2"`),
rustflags that change the target's cfg, `[replace]`, a crate missing
from the cargo cache (build the app first). For a TypeScript app: a
TypeScript file under a tsconfig.json without verbatimModuleSyntax, a
scriptc other than the one the template pins, a package-lock.json from
before npm 7, a node_modules that isn't what package-lock.json says (run
npm install). The audit lists those apps as not checked, like Java and
Zig apps, which get no deps.json yet.

The audit finds packages that turned unsafe by accident, such as after a
forgotten `cargo update` or `npm update`. What an app may do is limited by
the sandbox and its grants, whatever it is built with.
