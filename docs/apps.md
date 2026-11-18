# How an app works

The contract between the shell and an app. `appos guide` explains it for
the people and agents who build apps. Back to the [README](../README.md).

An app is a WebAssembly component with two functions ([wit/appos.wit](../wit/appos.wit)):

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

There is no SDK crate: bindings are generated in each app from its `wit/`,
and the helpers (routes and modules, the request's time and dates, kv with
JSON, form errors) are `src/appos.rs`. Both are part of the scaffold: the
`appos` binary carries them, `appos new` writes them and `appos sync`
brings an app up to date with the binary; `appos dev` and `appos deploy`
warn when they differ. The template and the Rust examples carry the same
copies, byte for byte (`cargo test -p appos` checks).
