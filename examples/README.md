# Example apps

Four apps for a made-up company, Kitewell, one per team and shape, in
three languages. Each app's request.md is the request it was built from,
written by someone who knows their domain but isn't an engineer. webinars
and discounts were built by an agent that knew only the `appos` binary (its
help and `appos guide`); pricing and status are ports of apps built the
same way, to show that AppOS runs WASM that isn't Rust.

| App | Team | Language | Shape | Grants |
|---|---|---|---|---|
| pricing-zig | Marketing | Zig | Price calculator: computes, saves nothing | none |
| webinars | Marketing | Rust | Registration with limited seats and a waitlist | kv-read, kv-write |
| status-ts | Customer service | TypeScript | Status page: support writes, customers read | kv-read, kv-write |
| discounts | Sales | Rust | Discount approval: requests, approval chain, history | kv-read, kv-write |

Each app stands alone. Copy one anywhere and run:

    appos dev        # http://127.0.0.1:8080/<app-id>
    appos sync       # the contract (wit/) of your appos, and the SDK where appos has one

## Rust: webinars, discounts

They need only the `appos` binary and Rust (rustup installs the version
in rust-toolchain.toml).

    make check       # formatting, lints, tests

In this repo, `make examples` builds and checks both. rustfmt.toml here
only keeps the repo's own formatting from reaching the apps; outside the
repo they use rustfmt's defaults.

## TypeScript: status-ts

Compiled with scriptc. `appos build` turns it into the same kind of
component as a Rust app; `appos new <name> --ts` starts a new one. Needs
Node 24+ and zig; `appos dev` runs `npm install` the first time.

## Zig: pricing-zig

Zig 0.17, with zt templates (src/calculator/view.zt). No adapter: zig
builds for wasm32-freestanding, `appos build` writes the `wit` module
(bindings/wit.zig) from wit/ and wraps the result in a component. 86 KB
with safety checks on. The first build fetches zt, so it needs the
network. src/appos.zig is the app's own SDK; `appos sync` updates only
wit/.

    make check                 # zig fmt, build, zig build test

In the repo root, `make typescript` builds the TypeScript example and
`make zig` runs the Zig one's check.

## A sign-in kit: auth-github

Not an app: a kit of the world `appos:auth/sign-in` (wit/auth.wit), which
signs in the members of one GitHub organization, with their teams as
groups. auth.toml names it, kit.toml says what it may call; the README's
"Your own sign-in" explains the world.

    make check       # formatting, lints, tests
    make build       # auth_github.wasm, next to kit.toml
