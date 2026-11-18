# Access

Who may do what, how people sign in, and how risky each grant is. Back to the [README](../README.md).

## Who may do what

An app's grants say what the app may do. The platform's policy says which
people may make it do that, under the same names. A host call goes through
only if both allow it, and the app checks nothing itself. An app with the
grant user-read can ask who is signed in (`appos guide users`).

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
# auth.toml: how people sign in (dev, header, oidc or kit; see Sign-in)
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
# the admin screen and the platform API: admin-read looks, admin-write changes
p, group:platform, *, admin-*
p, group:support, *, admin-read
# g, who, group: puts someone in a group
g, bob@acme.com, group:platform
```

Who: an email, `group:<name>` (from the sign-in, or given with `g`),
`signed-in` or `anyone`. Where: an app id, `apps:<group>` (the group in
app.toml) or `*`. What: a grant or a pattern like `kv-*`. The admin
screen's grants, admin-read and admin-write, go with `*` and by name: a
pattern like `*` that would also give them is refused. What no line
allows is denied. The rules
are [casbin](https://casbin.org)'s, with a model built into the shell.

A host call the policy denies ends the request and drops its writes. A
visitor who isn't signed in is sent to sign in and comes back to the page;
one who is gets "Not allowed", naming the app's owner. A denial is not the
app failing: it never counts toward an automatic rollback.

The shell checks the file every 2 s and reads it again when it changes; the
admin log shows each change. A file with a mistake is refused, with the
line, and the policy stays as it was; at startup it stops the shell.

The admin screen is AdminKit's pages, a kit built into the shell: those
with admin-read look, those with admin-write also change (with admin-read
alone the controls show and don't act, and the header says "Look only");
anyone else is sent to sign in or gets "Not allowed". The platform API
asks the same of a token: admin-read for a GET, admin-write for a deploy
or another change. It shows who is signed in, logs each change with
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

Anything else is a sign-in kit: a kit in WebAssembly of the world
`appos:auth/sign-in` ([wit-auth/auth.wit](../wit-auth/auth.wit), also in
`/_platform/spec` as `sign_in`). It exports `start` (show a page or send the
browser to the identity provider) and `finish` (the form it posted, or the
callback; the shell has checked the state) and returns a page, a redirect,
an identity, or why not.

```toml
# auth.toml
provider = "kit"
kit = "acme-sign-in"                    # its directory, next to auth.toml

# acme-sign-in/kit.toml
[kit]
id = "acme-sign-in"
name = "Acme sign-in"
version = "1.0.0"
wasm = "acme-auth.wasm"
[http]
hosts = ["login.acme.com"]              # the only hosts it may call
[config]                                # what config.get returns
tenant = "acme"
[config-env]                            # secrets: from the environment
client-secret = "ACME_SECRET"
```

The kit is sandboxed like an app: no files, no environment, no storage.
It may call `config.get` and `http.fetch`, which reaches only its [http]
hosts, over https (http only to localhost), and follows no redirects, as
any kit's fetch. Each step runs in a
fresh instance with an app's fuel and memory, for at most 10 s. A kit
that doesn't load stops the shell at startup; one that fails while someone
signs in shows "Sign-in isn't working right now", and the site goes on.

[examples/auth-github](../examples/auth-github) signs in the members of one
GitHub organization, with their teams as groups: Rust and wit-bindgen,
`make build` makes `auth_github.wasm`.

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
