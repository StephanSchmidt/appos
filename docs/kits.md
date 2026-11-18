# Kits

Back to the [README](../README.md).

Besides `page`, `kv` and `users`, apps call the platform's kits:
functions in WIT, each behind a grant. An app that holds a kit's grant
imports its interface and calls its functions like its own; `appos sync`
brings the kit's WIT into the app's `wit/`, and the bindings in each
language come from it. AppOS checks the app's grant and the policy before
a call reaches the kit; a kit never checks permissions itself.

A kit is one of two things:

- Rust, compiled into the platform. The platform team builds its shell
  with `appos_shell::Platform::new().kit(...)`; the kit implements the
  `Kit` trait: its WIT, its grants, the Rust behind each function and its
  pages. [kitewell](../kitewell) is a platform with one such kit, its booking
  flow, and two apps by sales and product that call it.
- WebAssembly, deployed like an app. `appos new crm --kit` writes one in
  Rust: kit.toml (id, owner, group, grants and the functions each allows),
  its WIT in `api/`. `appos dev`, `deploy`, `versions`, `rollback`, `off`
  and `on` work in its folder, and it has versions, limits and an off
  switch like an app. [examples/acme-scoring](../examples/acme-scoring)
  scores the customers of [acme-platform](../examples/acme-platform)'s CRM
  kit; `appos guide kits` has the details.

A call an app waits for only reads. A change is a command: a function that
returns nothing, sent once the caller's request commits and run by the kit
after. A request that fails sends nothing; a command that fails is tried
again, and one that keeps failing waits on the admin screen. A kit may
have pages of its own, like Kitewell's `/book`; a Wasm kit's are at
`/<kit-id>`. A Wasm kit reaches other services only through `http::fetch`,
to the hosts in kit.toml's `[http]`, over https; apps have no network at
all. While a kit is off, the apps that hold its grants answer 503 and
their commands to it wait.
