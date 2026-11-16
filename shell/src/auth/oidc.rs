//! The OIDC provider: any OpenID Connect identity provider (Keycloak,
//! Entra ID, Okta, Google, Zitadel, Authentik, ...), with the authorization
//! code flow and PKCE.
//!
//! ```toml
//! provider = "oidc"
//! issuer = "https://login.acme.com"
//! client-id = "appos"
//! client-secret-env = "APPOS_OIDC_SECRET"   # the secret is never in the file
//! scopes = ["openid", "email", "profile"]   # the default
//! groups-claim = "groups"                   # where the groups are, if anywhere
//! ```
//!
//! The identity provider redirects back to `<origin>/_auth/callback`, the
//! origin of the listener the sign-in started on (the site, or the admin's
//! platform listener): register both with it. The
//! id token comes straight from its token endpoint over TLS, which OIDC
//! Core 3.1.3.7 accepts in place of checking the signature; the shell checks
//! iss, aud, azp, exp and the nonce. Discovery is fetched at the first
//! sign-in and kept an hour: an identity provider that is down stops
//! sign-in, not the site.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::routes::{Pending, Step};
use super::session::now;
use super::{Identity, need, take_list, take_str};
use crate::terminal_errorln as eprintln;

/// How long discovery is kept.
const KEEP: Duration = Duration::from_secs(3600);
/// Each call to the identity provider.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Clocks may differ this much.
const SKEW_SECS: i64 = 60;

pub struct Oidc {
    issuer: String,
    client_id: String,
    secret: Option<String>,
    scopes: String,
    groups_claim: Option<String>,
    discovery: Mutex<Option<(Instant, Arc<Discovery>)>>,
}

#[derive(Debug, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    end_session_endpoint: Option<String>,
}

impl Oidc {
    pub fn parse(table: &mut toml::Table, env: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Oidc> {
        let issuer = need(take_str(table, "issuer")?, "oidc", "issuer = \"https://login.acme.com\"")?;
        if !secure(&issuer) {
            anyhow::bail!("issuer = {issuer:?}: give an https URL (http only on localhost)");
        }
        let client_id = need(take_str(table, "client-id")?, "oidc", "client-id, as the identity provider calls this site")?;
        let secret = match take_str(table, "client-secret-env")? {
            Some(var) => match env(&var).filter(|s| !s.is_empty()) {
                Some(s) => Some(s),
                None => anyhow::bail!("client-secret-env = {var:?}, but {var} is not set"),
            },
            None => None,
        };
        let scopes = take_list(table, "scopes")?.unwrap_or_else(|| ["openid", "email", "profile"].map(String::from).to_vec());
        if !scopes.iter().any(|s| s == "openid") {
            anyhow::bail!("scopes need \"openid\"");
        }
        Ok(Oidc { issuer, client_id, secret, scopes: scopes.join(" "), groups_claim: take_str(table, "groups-claim")?, discovery: Mutex::new(None) })
    }

    async fn discovery(&self) -> Result<Arc<Discovery>, String> {
        if let Some((at, d)) = &*self.discovery.lock()
            && at.elapsed() < KEEP
        {
            return Ok(d.clone());
        }
        let url = format!("{}/.well-known/openid-configuration", self.issuer.trim_end_matches('/'));
        let (status, body) = http(move |agent| agent.get(&url).header("accept", "application/json").call()).await?;
        if status != 200 {
            return Err(format!("discovery answered {status}"));
        }
        let d: Discovery = serde_json::from_str(&body).map_err(|e| format!("discovery: {e}"))?;
        if d.issuer.trim_end_matches('/') != self.issuer.trim_end_matches('/') {
            return Err(format!("discovery names the issuer {:?}, not {:?}", d.issuer, self.issuer));
        }
        if ![&d.authorization_endpoint, &d.token_endpoint].iter().all(|u| secure(u)) {
            return Err("discovery gives endpoints that are not https".into());
        }
        let d = Arc::new(d);
        *self.discovery.lock() = Some((Instant::now(), d.clone()));
        Ok(d)
    }

    /// To the identity provider, with state, nonce and the PKCE challenge.
    pub async fn start(&self, pending: &Pending, origin: &str) -> Step {
        let d = match self.discovery().await {
            Ok(d) => d,
            Err(e) => return unavailable(&e),
        };
        let challenge = B64.encode(Sha256::digest(pending.verifier.as_bytes()));
        let query: String = form_urlencoded::Serializer::new(String::new())
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &redirect_uri(origin))
            .append_pair("scope", &self.scopes)
            .append_pair("state", &pending.state)
            .append_pair("nonce", &pending.nonce)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .finish();
        let sep = if d.authorization_endpoint.contains('?') { '&' } else { '?' };
        Step::Redirect(format!("{}{sep}{query}", d.authorization_endpoint))
    }

    /// Back from the identity provider (the state was checked): the code
    /// for an id token, and the id token checked.
    pub async fn callback(&self, query: &[(String, String)], pending: &Pending, origin: &str) -> Step {
        let field = |name: &str| query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        if let Some(error) = field("error") {
            let said = field("error_description").unwrap_or(error);
            return Step::Refuse(format!("The sign-in service said: {said}"));
        }
        let Some(code) = field("code") else { return Step::Refuse("The sign-in service sent no code. Sign in again.".into()) };
        let d = match self.discovery().await {
            Ok(d) => d,
            Err(e) => return unavailable(&e),
        };
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", redirect_uri(origin)),
            ("code_verifier", pending.verifier.clone()),
        ];
        let basic = match &self.secret {
            // client_secret_basic, each part form-encoded (RFC 6749 2.3.1).
            Some(secret) => Some(format!("Basic {}", STANDARD.encode(format!("{}:{}", encode(&self.client_id), encode(secret))))),
            None => {
                form.push(("client_id", self.client_id.clone()));
                None
            }
        };
        let url = d.token_endpoint.clone();
        let answer = http(move |agent| {
            let mut req = agent.post(&url).header("accept", "application/json");
            if let Some(b) = &basic {
                req = req.header("authorization", b);
            }
            req.send_form(form)
        })
        .await;
        let body = match answer {
            Ok((200, body)) => body,
            Ok((status, body)) => {
                eprintln!("sign-in: the token endpoint answered {status}: {}", body.chars().take(300).collect::<String>());
                return Step::Refuse("The sign-in service turned the sign-in down. Sign in again.".into());
            }
            Err(e) => return unavailable(&e),
        };
        let token = serde_json::from_str::<Value>(&body).ok().and_then(|v| v.get("id_token")?.as_str().map(str::to_string));
        let Some(token) = token else {
            eprintln!("sign-in: the token endpoint sent no id_token");
            return Step::Refuse("The sign-in service sent no identity. Sign in again.".into());
        };
        match self.identity(&token, &d.issuer, &pending.nonce, now()) {
            Ok(id) => Step::Done(id),
            Err(e) => {
                eprintln!("sign-in refused: {e}");
                Step::Refuse(format!("Sign-in didn't work: {e}."))
            }
        }
    }

    /// The id token's claims, checked, as an identity.
    fn identity(&self, token: &str, issuer: &str, nonce: &str, now: i64) -> Result<Identity, String> {
        let payload = token.split('.').nth(1).ok_or("the id token is no JWT")?;
        let claims: Value = B64.decode(payload.trim_end_matches('=')).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or("the id token can't be read")?;
        let text = |name: &str| claims.get(name).and_then(Value::as_str);
        if text("iss") != Some(issuer) {
            return Err(format!("the id token is from {:?}, not {issuer:?}", text("iss").unwrap_or("nobody")));
        }
        let aud: Vec<&str> = match claims.get("aud") {
            Some(Value::String(a)) => vec![a],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !aud.contains(&self.client_id.as_str()) {
            return Err("the id token is for another site".into());
        }
        if (aud.len() > 1 || text("azp").is_some()) && text("azp") != Some(&self.client_id) {
            return Err("the id token was given to another site".into());
        }
        match claims.get("exp").and_then(Value::as_i64) {
            Some(exp) if now < exp + SKEW_SECS => {}
            _ => return Err("the sign-in has expired".into()),
        }
        if text("nonce") != Some(nonce) {
            return Err("the id token belongs to another sign-in".into());
        }
        let email = text("email").ok_or("the sign-in service sent no email address")?;
        if matches!(claims.get("email_verified"), Some(Value::Bool(false))) || text("email_verified") == Some("false") {
            return Err(format!("{email} is not verified with the sign-in service"));
        }
        let name = text("name").or(text("preferred_username")).unwrap_or("");
        let groups: Vec<String> = match self.groups_claim.as_deref().and_then(|c| claims.get(c)) {
            Some(Value::Array(g)) => g.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            Some(Value::String(g)) => vec![g.clone()],
            _ => vec![],
        };
        Identity::new(text("sub").unwrap_or(""), email, name, groups)
    }

    /// Where signing out goes: the identity provider's end of session, if
    /// it has one, which comes back to the home page.
    pub async fn sign_out(&self, origin: &str) -> String {
        let end = self.discovery.lock().as_ref().and_then(|(_, d)| d.end_session_endpoint.clone());
        match end.filter(|e| secure(e)) {
            Some(end) => {
                let query: String = form_urlencoded::Serializer::new(String::new())
                    .append_pair("client_id", &self.client_id)
                    .append_pair("post_logout_redirect_uri", &format!("{}/", origin.trim_end_matches('/')))
                    .finish();
                let sep = if end.contains('?') { '&' } else { '?' };
                format!("{end}{sep}{query}")
            }
            None => "/".into(),
        }
    }
}

fn unavailable(e: &str) -> Step {
    eprintln!("sign-in: the identity provider: {e}");
    Step::Unavailable("The sign-in service can't be reached right now. Try again in a moment.".into())
}

fn encode(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Where the identity provider sends people back to.
fn redirect_uri(origin: &str) -> String {
    format!("{}/_auth/callback", origin.trim_end_matches('/'))
}

/// https, or http on this machine.
pub(super) fn secure(url: &str) -> bool {
    let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"];
    url.starts_with("https://") || local.iter().any(|l| url.strip_prefix(l).is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/'])))
}

/// One call to the identity provider, off the async workers: the status
/// and up to 1 MB of the body.
async fn http(call: impl FnOnce(&ureq::Agent) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> + Send + 'static) -> Result<(u16, String), String> {
    tokio::task::spawn_blocking(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into();
        let mut resp = call(&agent).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(1 << 20).read_to_string().map_err(|e| e.to_string())?;
        Ok((status, body))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use axum::Router;
    use axum::extract::{Form, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Json};
    use axum::routing::{get, post};
    use serde_json::json;

    use super::*;

    /// A fake identity provider on a free port. It hands out the id token
    /// `claims` makes, after checking the client's secret and PKCE.
    pub struct Idp {
        pub url: String,
        pub claims: Arc<Mutex<Value>>,
        /// The challenge the browser brought to the authorization endpoint.
        pub challenge: Arc<Mutex<String>>,
    }

    pub fn jwt(claims: &Value) -> String {
        format!("{}.{}.sig", B64.encode(br#"{"alg":"RS256"}"#), B64.encode(claims.to_string()))
    }

    pub async fn idp() -> Idp {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let claims = Arc::new(Mutex::new(Value::Null));
        let challenge = Arc::new(Mutex::new(String::new()));
        type S = (String, Arc<Mutex<Value>>, Arc<Mutex<String>>);
        let state: S = (url.clone(), claims.clone(), challenge.clone());
        let app = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(|State((url, _, _)): State<S>| async move {
                    Json(json!({
                        "issuer": url,
                        "authorization_endpoint": format!("{url}/authorize"),
                        "token_endpoint": format!("{url}/token"),
                        "end_session_endpoint": format!("{url}/logout"),
                    }))
                }),
            )
            .route(
                "/token",
                post(|State((_, claims, challenge)): State<S>, headers: HeaderMap, Form(form): Form<Vec<(String, String)>>| async move {
                    let field = |n: &str| form.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone()).unwrap_or_default();
                    let basic = format!("Basic {}", STANDARD.encode("appos:s3cret%2F"));
                    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(basic.as_str()) {
                        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid_client"}))).into_response();
                    }
                    if B64.encode(Sha256::digest(field("code_verifier").as_bytes())) != *challenge.lock() || field("code") != "the-code" {
                        return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"}))).into_response();
                    }
                    Json(json!({"access_token": "x", "id_token": jwt(&claims.lock())})).into_response()
                }),
            )
            .with_state(state);
        tokio::spawn(async move { axum::serve(listener, app).await });
        Idp { url, claims, challenge }
    }

    pub fn auth_toml(idp: &Idp) -> String {
        format!("provider = \"oidc\"\nissuer = \"{}\"\nclient-id = \"appos\"\nclient-secret-env = \"SECRET\"\ngroups-claim = \"groups\"", idp.url)
    }

    fn oidc(idp_url: &str) -> Oidc {
        let text = format!("issuer = \"{idp_url}\"\nclient-id = \"appos\"\nclient-secret-env = \"SECRET\"\ngroups-claim = \"groups\"");
        Oidc::parse(&mut toml::from_str(&text).unwrap(), &|_| Some("s3cret/".into())).unwrap()
    }

    #[test]
    fn id_tokens_are_checked() {
        let o = oidc("https://login.acme.com");
        let good = json!({"iss": "https://login.acme.com", "sub": "u1", "aud": "appos", "exp": 1000, "nonce": "n",
            "email": "Ann@Acme.com", "email_verified": true, "name": "Ann Lee", "groups": ["sales", "platform"]});
        let check = |change: &dyn Fn(&mut Value)| {
            let mut claims = good.clone();
            change(&mut claims);
            o.identity(&jwt(&claims), "https://login.acme.com", "n", 900)
        };
        let id = check(&|_| {}).unwrap();
        assert_eq!(id, Identity { id: "u1".into(), email: "ann@acme.com".into(), name: "Ann Lee".into(), groups: vec!["sales".into(), "platform".into()] });
        assert!(check(&|c| c["iss"] = json!("https://evil.example")).unwrap_err().contains("not \"https://login.acme.com\""));
        assert!(check(&|c| c["aud"] = json!("other")).unwrap_err().contains("another site"));
        assert!(check(&|c| c["aud"] = json!(["appos", "other"])).unwrap_err().contains("given to another site"), "azp needed");
        assert!(
            check(&|c| {
                c["aud"] = json!(["appos", "other"]);
                c["azp"] = json!("appos")
            })
            .is_ok()
        );
        assert!(check(&|c| c["exp"] = json!(800)).unwrap_err().contains("expired"));
        assert!(check(&|c| c["exp"] = json!(870)).is_ok(), "a minute of clock skew");
        assert!(check(&|c| c["nonce"] = json!("other")).unwrap_err().contains("another sign-in"));
        assert!(check(&|c| c.as_object_mut().unwrap().remove("email").map(|_| ()).unwrap()).unwrap_err().contains("no email"));
        assert!(check(&|c| c["email_verified"] = json!(false)).unwrap_err().contains("not verified"));
        assert_eq!(check(&|c| c["groups"] = json!("admins")).unwrap().groups, ["admins"]);
        assert!(o.identity("garbage", "https://login.acme.com", "n", 900).is_err());
    }

    #[test]
    fn auth_toml_needs_an_issuer_a_client_and_the_secret_set() {
        let parse = |text: &str, env: Option<&str>| {
            let env = env.map(str::to_string);
            Oidc::parse(&mut toml::from_str(text).unwrap(), &move |_| env.clone()).err().map(|e| e.to_string()).unwrap_or_default()
        };
        assert!(parse("client-id = \"a\"", None).contains("needs issuer"));
        assert!(parse("issuer = \"http://login.acme.com\"\nclient-id = \"a\"", None).contains("https"));
        assert!(parse("issuer = \"https://login.acme.com\"", None).contains("needs client-id"));
        assert!(parse("issuer = \"https://login.acme.com\"\nclient-id = \"a\"\nclient-secret-env = \"S\"", None).contains("S is not set"));
        assert!(parse("issuer = \"https://login.acme.com\"\nclient-id = \"a\"\nscopes = [\"email\"]", None).contains("openid"));
        assert_eq!(parse("issuer = \"https://login.acme.com\"\nclient-id = \"a\"\nclient-secret-env = \"S\"", Some("x")), "");
        assert!(secure("http://localhost:9000/x") && secure("http://127.0.0.1") && !secure("http://localhost.evil.example") && !secure("http://acme.com"));
    }
}
