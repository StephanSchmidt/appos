//! Who is asking. `appos-shell --auth auth.toml` names a sign-in
//! provider; the shell owns the session (a signed cookie, session.rs), so a
//! provider runs only while someone signs in, never per request.
//!
//! Providers: `dev` (pick a name, for trying things on localhost),
//! `header` (a proxy in front signs people in and sends who they are),
//! `oidc` (any OpenID Connect identity provider) and `kit` (a sign-in kit
//! in Wasm, for anything else).
//!
//! Who is asking feeds the policy (access.rs): a host call goes through
//! only if the app holds the grant and the policy lets this user make it.
//! Apps never see the user.

mod dev;
mod oidc;
mod proxy;
mod routes;
pub mod session;
mod sign_in_kit;
pub mod token;
pub mod tokens;

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;
use serde::{Deserialize, Serialize};

pub use routes::router;
use session::{Cookies, SESSION};

use crate::shell::Shell;

/// Someone signed in, as their provider told it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// What the provider calls them; stable.
    pub id: String,
    pub email: String,
    pub name: String,
    pub groups: Vec<String>,
}

/// Groups kept from a sign-in, and their length: some providers send
/// hundreds, and they all ride in the cookie.
const MAX_GROUPS: usize = 50;
const MAX_FIELD: usize = 200;

impl Identity {
    /// An identity as a provider gave it: trimmed, emails in lower case,
    /// groups deduplicated and capped. Refused without an email, which the
    /// policy names people by.
    pub fn new(id: &str, email: &str, name: &str, groups: impl IntoIterator<Item = String>) -> Result<Identity, String> {
        let email = email.trim().to_lowercase();
        if !email.contains('@') || email.len() > MAX_FIELD || email.chars().any(|c| c.is_whitespace() || c.is_control() || "<>\"'&".contains(c)) {
            return Err(format!("{email:?} is not an email address"));
        }
        let name = match name.trim() {
            "" => email.split('@').next().unwrap_or_default().to_string(),
            n => n.chars().filter(|c| !c.is_control()).take(MAX_FIELD).collect(),
        };
        let id = match id.trim() {
            "" => email.clone(),
            id => id.chars().take(MAX_FIELD).collect(),
        };
        let mut kept: Vec<String> = Vec::new();
        for g in groups {
            let g = g.trim();
            if !g.is_empty() && g.len() <= MAX_FIELD && !g.chars().any(char::is_control) && !kept.iter().any(|k| k == g) {
                kept.push(g.to_string());
            }
        }
        kept.truncate(MAX_GROUPS);
        Ok(Identity { id, email, name, groups: kept })
    }
}

/// Who sends a request: someone signed in, or nobody known.
#[derive(Debug, Clone, Default)]
pub struct Who(pub Option<Arc<Identity>>);

/// The origin of the listener a request came in on: the site's, or the
/// platform listener's for the admin screen. Sign-in sends people back
/// there, and its cookies are Secure when it is https.
#[derive(Clone)]
pub struct Site(pub Arc<str>);

/// Every request has these subjects in the policy.
pub const ANYONE: &str = "anyone";
/// Everyone signed in.
pub const SIGNED_IN: &str = "signed-in";

impl Who {
    pub fn signed_in(id: Identity) -> Who {
        Who(Some(Arc::new(id)))
    }

    pub fn identity(&self) -> Option<&Identity> {
        self.0.as_deref()
    }

    /// The email, for the events of what this person did.
    pub fn person(&self) -> Option<String> {
        self.identity().map(|id| id.email.clone())
    }

    /// The names the policy may give this request: the email, each group
    /// as `group:<name>`, `signed-in` and `anyone`.
    pub fn subjects(&self) -> Vec<String> {
        let mut subjects = Vec::new();
        if let Some(id) = self.identity() {
            subjects.push(id.email.clone());
            subjects.extend(id.groups.iter().map(|g| format!("group:{g}")));
            subjects.push(SIGNED_IN.into());
        }
        subjects.push(ANYONE.into());
        subjects
    }
}

/// Handlers take `Who` as an argument; `identify` put it there.
impl<S: Send + Sync> FromRequestParts<S> for Who {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<Who>().cloned().unwrap_or_default())
    }
}

pub enum Provider {
    Dev,
    Header(proxy::Header),
    Oidc(oidc::Oidc),
    Kit(sign_in_kit::SignInKit),
    /// No auth.toml: the link with a token that the shell prints.
    Token(token::Token),
}

/// Sign-in, as auth.toml sets it up.
pub struct Auth {
    pub provider: Provider,
    pub cookies: Cookies,
    /// How long a session lasts.
    pub session_secs: i64,
}

const DEFAULT_HOURS: i64 = 8;
const MAX_HOURS: i64 = 24 * 30;

impl Auth {
    /// Reads auth.toml. The session key lives in `data`; `origin` is where
    /// the site is reached (https makes cookies Secure); `addr` is where it
    /// listens (the dev provider only on localhost).
    pub fn load(path: &Path, origin: &str, data: &Path, addr: &str) -> anyhow::Result<Auth> {
        let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let key = Cookies::load_key(data)?;
        let dir = path.parent().unwrap_or(Path::new("."));
        Auth::parse_in(&text, origin, key, addr, &|var| std::env::var(var).ok(), dir).map_err(|e| anyhow::anyhow!("{}: {e:#}", path.display()))
    }

    #[cfg(test)]
    pub fn parse(text: &str, origin: &str, key: [u8; 32], addr: &str) -> anyhow::Result<Auth> {
        Auth::parse_with(text, origin, key, addr, &|var| std::env::var(var).ok())
    }

    /// `env` reads the environment (the OIDC client secret).
    #[cfg(test)]
    pub fn parse_with(text: &str, origin: &str, key: [u8; 32], addr: &str, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Auth> {
        Auth::parse_in(text, origin, key, addr, env, Path::new("."))
    }

    /// `dir` is auth.toml's directory, where a sign-in kit's directory is.
    pub fn parse_in(text: &str, origin: &str, key: [u8; 32], addr: &str, env: &dyn Fn(&str) -> Option<String>, dir: &Path) -> anyhow::Result<Auth> {
        let mut table: toml::Table = toml::from_str(text)?;
        let session_secs = match table.remove("session-hours") {
            None => DEFAULT_HOURS,
            Some(toml::Value::Integer(h)) if (1..=MAX_HOURS).contains(&h) => h,
            Some(v) => anyhow::bail!("session-hours = {v}: give whole hours from 1 to {MAX_HOURS}"),
        } * 3600;
        let provider = match table.remove("provider") {
            Some(toml::Value::String(p)) => p,
            Some(v) => anyhow::bail!("provider = {v}: give one of dev, header, oidc, kit"),
            None => anyhow::bail!("no provider: give provider = \"dev\", \"header\", \"oidc\" or \"kit\""),
        };
        let provider_name = provider;
        let provider = match provider_name.as_str() {
            "dev" => {
                if !is_loopback(addr) {
                    anyhow::bail!("provider \"dev\" lets anyone sign in as anyone: only with --addr on localhost, not {addr}");
                }
                Provider::Dev
            }
            "header" => Provider::Header(proxy::Header::parse(&mut table)?),
            "oidc" => Provider::Oidc(oidc::Oidc::parse(&mut table, env)?),
            "kit" => Provider::Kit(sign_in_kit::SignInKit::parse(&mut table, dir, env)?),
            other => anyhow::bail!("unknown provider {other:?}; known: dev, header, oidc, kit"),
        };
        no_more(&table, &provider_name)?;
        Ok(Auth { provider, cookies: Cookies::new(key, origin.starts_with("https://")), session_secs })
    }

    /// Who sends this request.
    pub fn who(&self, headers: &HeaderMap, peer: Option<IpAddr>) -> Who {
        match &self.provider {
            Provider::Header(h) => h.who(headers, peer),
            Provider::Dev | Provider::Oidc(_) | Provider::Kit(_) | Provider::Token(_) => Who(self.cookies.read::<Identity>(headers, SESSION).map(Arc::new)),
        }
    }

    /// The token sign-in, for a shell started without auth.toml: `secret`
    /// signs in as its admin (token.rs). The session key lives in `data`.
    pub fn token(secret: String, origin: &str, data: &Path) -> anyhow::Result<Auth> {
        let key = Cookies::load_key(data)?;
        Ok(Auth::token_with(secret, origin, key))
    }

    /// The dev sign-in of `appos dev`, where anyone signs in as anyone,
    /// a test user filled in (dev.rs). The session key lives in `data`.
    pub fn dev(origin: &str, data: &Path) -> anyhow::Result<Auth> {
        let cookies = Cookies::new(Cookies::load_key(data)?, origin.starts_with("https://"));
        Ok(Auth { provider: Provider::Dev, cookies, session_secs: DEFAULT_HOURS * 3600 })
    }

    pub fn token_with(secret: String, origin: &str, key: [u8; 32]) -> Auth {
        let cookies = Cookies::new(key, origin.starts_with("https://"));
        Auth { provider: Provider::Token(token::Token::new(secret)), cookies, session_secs: DEFAULT_HOURS * 3600 }
    }

    /// Where signing out goes.
    pub async fn sign_out(&self, origin: &str) -> String {
        match &self.provider {
            Provider::Dev | Provider::Kit(_) | Provider::Token(_) => "/".into(),
            Provider::Header(h) => h.sign_out(),
            Provider::Oidc(o) => o.sign_out(origin).await,
        }
    }

    /// The Set-Cookie header that signs `id` in, on the listener at `origin`.
    pub fn sign_in(&self, id: &Identity, origin: &str) -> HeaderValue {
        self.cookies.on(origin).set(SESSION, id, self.session_secs)
    }
}

/// Refuses keys a provider doesn't take.
fn no_more(table: &toml::Table, provider: &str) -> anyhow::Result<()> {
    match table.keys().next() {
        Some(key) => anyhow::bail!("provider \"{provider}\" takes no {key:?}"),
        None => Ok(()),
    }
}

/// A text setting, taken out of auth.toml.
fn take_str(table: &mut toml::Table, key: &str) -> anyhow::Result<Option<String>> {
    match table.remove(key) {
        None => Ok(None),
        Some(toml::Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Some(v) => anyhow::bail!("{key} = {v}: give a text"),
    }
}

/// A list of texts, taken out of auth.toml.
fn take_list(table: &mut toml::Table, key: &str) -> anyhow::Result<Option<Vec<String>>> {
    match table.remove(key) {
        None => Ok(None),
        Some(toml::Value::Array(items)) if !items.is_empty() => items
            .into_iter()
            .map(|v| match v {
                toml::Value::String(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
                v => anyhow::bail!("{key}: {v} is no text"),
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .map(Some),
        Some(v) => anyhow::bail!("{key} = {v}: give a list, like [\"a\", \"b\"]"),
    }
}

/// A setting the provider can't do without.
fn need<T>(value: Option<T>, provider: &str, what: &str) -> anyhow::Result<T> {
    value.ok_or_else(|| anyhow::anyhow!("provider \"{provider}\" needs {what}"))
}

pub(crate) fn is_loopback(addr: &str) -> bool {
    match addr.parse::<SocketAddr>() {
        Ok(a) => a.ip().is_loopback(),
        Err(_) => addr.split(':').next() == Some("localhost"),
    }
}

/// Puts `Who` into every request. Pages for someone signed in show their
/// name, so no shared cache may keep them.
pub async fn identify(State(shell): State<Arc<Shell>>, mut req: Request, next: Next) -> Response {
    let Some(auth) = &shell.auth else { return next.run(req).await };
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let who = auth.who(req.headers(), peer);
    let signed_in = who.0.is_some();
    req.extensions_mut().insert(who);
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.append(header::VARY, HeaderValue::from_static("Cookie"));
    if signed_in {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_cleaned_and_need_an_email() {
        let id = Identity::new("", " Ann@Acme.COM ", "", ["sales".into(), " sales ".into(), "".into(), "ops".into()]).unwrap();
        assert_eq!(id, Identity { id: "ann@acme.com".into(), email: "ann@acme.com".into(), name: "ann".into(), groups: vec!["sales".into(), "ops".into()] });
        assert!(Identity::new("1", "nobody", "N", []).is_err());
        assert!(Identity::new("1", "a b@acme.com", "N", []).is_err());
        assert!(Identity::new("1", "<b>@acme.com", "N", []).is_err(), "it shows on admin pages");
        let many = Identity::new("1", "a@b.c", "A", (0..500).map(|i| format!("g{i}"))).unwrap();
        assert_eq!(many.groups.len(), MAX_GROUPS);
    }

    #[test]
    fn subjects_are_the_email_the_groups_signed_in_and_anyone() {
        assert_eq!(Who::default().subjects(), ["anyone"]);
        let who = Who::signed_in(Identity::new("1", "ann@acme.com", "Ann", ["sales".into()]).unwrap());
        assert_eq!(who.subjects(), ["ann@acme.com", "group:sales", "signed-in", "anyone"]);
    }

    #[test]
    fn auth_toml_is_checked() {
        let parse = |text: &str, addr: &str| Auth::parse(text, "http://localhost", [0; 32], addr).map(|_| ()).map_err(|e| e.to_string());
        assert!(parse("provider = \"dev\"", "127.0.0.1:8080").is_ok());
        assert!(parse("provider = \"dev\"\nsession-hours = 2", "localhost:8080").is_ok());
        assert!(parse("provider = \"dev\"", "0.0.0.0:8080").unwrap_err().contains("only with --addr on localhost"));
        assert!(parse("provider = \"dev\"\nissuer = \"x\"", "127.0.0.1:1").unwrap_err().contains("takes no \"issuer\""));
        assert!(parse("provider = \"dev\"\nsession-hours = 0", "127.0.0.1:1").unwrap_err().contains("session-hours"));
        assert!(parse("", "127.0.0.1:1").unwrap_err().contains("no provider"));
        assert!(parse("provider = \"ldap\"", "127.0.0.1:1").unwrap_err().contains("unknown provider"));
    }
}
