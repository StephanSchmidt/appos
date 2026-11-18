//! Sign-in with GitHub: an AppOS sign-in kit (wit/auth.wit). The members
//! of one GitHub organization sign in; their teams in it become their
//! groups, which policy.csv names as `group:<team>`.
//!
//! auth.toml names the kit (provider = "kit", kit = "."), and kit.toml
//! says the rest (this directory has both):
//!
//!   [kit]
//!   id = "auth-github"
//!   wasm = "auth_github.wasm"
//!   [http]
//!   hosts = ["github.com", "api.github.com"]
//!   [config]
//!   client-id = "Iv1.0123456789abcdef"   # the GitHub OAuth app's
//!   org = "kitewell"                     # only its members sign in
//!   [config-env]
//!   client-secret = "GITHUB_CLIENT_SECRET"
//!
//! The OAuth app's callback URL is `<origin>/_auth/callback`. The flow:
//! start sends the browser to GitHub with the shell's state and PKCE
//! challenge; finish trades the code for a token, then asks the API who
//! this is, for their verified primary email, whether they are in the org,
//! and their teams there.

wit_bindgen::generate!({ world: "sign-in", path: "wit" });

use appos::auth::types::{Identity, Request, Step};
use appos::auth::{config, http};
use serde::Deserialize;

const AUTHORIZE: &str = "https://github.com/login/oauth/authorize";
const TOKEN: &str = "https://github.com/login/oauth/access_token";
const API: &str = "https://api.github.com";

struct GitHub;

impl exports::appos::auth::provider::Guest for GitHub {
    fn start(req: Request) -> Step {
        Step::Redirect(authorize_url(&setting("client-id"), &req))
    }

    fn finish(req: Request) -> Step {
        let field = |name: &str| {
            req.query
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        if let Some(error) = field("error") {
            return Step::Refuse(format!(
                "GitHub said: {}",
                field("error_description").unwrap_or(error)
            ));
        }
        let Some(code) = field("code") else {
            return Step::Refuse("GitHub sent no code. Sign in again.".into());
        };
        match sign_in(code, &req) {
            Ok(step) => step,
            Err(e) => Step::Unavailable(format!(
                "GitHub can't be reached right now ({e}). Try again in a moment."
            )),
        }
    }
}

export!(GitHub);

/// A setting from auth.toml. Without it the plugin can't work: it stops,
/// and the shell logs this message.
fn setting(key: &str) -> String {
    config::get(key).unwrap_or_else(|| panic!("auth.toml: [config] or [config-env] needs {key}"))
}

fn authorize_url(client_id: &str, req: &Request) -> String {
    let query = [
        ("client_id", client_id),
        ("redirect_uri", req.callback.as_str()),
        ("state", req.state.as_str()),
        ("scope", "read:user user:email read:org"),
        ("code_challenge", req.challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("allow_signup", "false"),
    ];
    format!("{AUTHORIZE}?{}", form(&query))
}

/// The steps after GitHub sent the browser back. An error is a call that
/// failed; a refusal is a step.
fn sign_in(code: &str, req: &Request) -> Result<Step, String> {
    let org = setting("org");
    let body = form(&[
        ("client_id", setting("client-id").as_str()),
        ("client_secret", setting("client-secret").as_str()),
        ("code", code),
        ("redirect_uri", req.callback.as_str()),
        ("code_verifier", req.verifier.as_str()),
    ]);
    let token: Token = call("POST", TOKEN, None, body)?;
    let token = match token {
        Token {
            access_token: Some(t),
            ..
        } => t,
        Token {
            error_description,
            error,
            ..
        } => {
            return Ok(Step::Refuse(format!(
                "GitHub turned the sign-in down: {}",
                error_description.or(error).unwrap_or_default()
            )));
        }
    };
    let api = |path: &str| format!("{API}{path}");
    let user: User = call("GET", &api("/user"), Some(&token), String::new())?;
    let emails: Vec<Email> = call("GET", &api("/user/emails"), Some(&token), String::new())?;
    let Some(email) = primary_email(&emails) else {
        return Ok(Step::Refuse(
            "Your GitHub account has no verified primary email.".into(),
        ));
    };
    let member: Result<Membership, String> = call(
        "GET",
        &api(&format!("/user/memberships/orgs/{org}")),
        Some(&token),
        String::new(),
    );
    if !member.is_ok_and(|m| m.state == "active") {
        return Ok(Step::Refuse(format!(
            "Only members of {org} on GitHub sign in here."
        )));
    }
    let teams: Vec<Team> = call(
        "GET",
        &api("/user/teams?per_page=100"),
        Some(&token),
        String::new(),
    )?;
    Ok(Step::Done(Identity {
        id: format!("github:{}", user.id),
        email,
        name: user.name.unwrap_or(user.login),
        groups: teams_in(&teams, &org),
    }))
}

/// One call, its answer read as JSON; any status but 200 is an error.
fn call<T: for<'a> Deserialize<'a>>(
    method: &str,
    url: &str,
    token: Option<&str>,
    body: String,
) -> Result<T, String> {
    let mut headers = vec![
        ("accept".to_string(), "application/json".to_string()),
        ("user-agent".to_string(), "appos-sign-in".to_string()),
    ];
    if let Some(t) = token {
        headers.push(("authorization".into(), format!("Bearer {t}")));
        headers.push(("x-github-api-version".into(), "2022-11-28".into()));
    }
    if !body.is_empty() {
        headers.push((
            "content-type".into(),
            "application/x-www-form-urlencoded".into(),
        ));
    }
    let answer = http::fetch(&http::Request {
        method: method.into(),
        url: url.into(),
        headers,
        body,
    })?;
    if answer.status != 200 {
        return Err(format!("{url} answered {}", answer.status));
    }
    serde_json::from_str(&answer.body).map_err(|e| format!("{url}: {e}"))
}

#[derive(Deserialize)]
struct Token {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct User {
    id: u64,
    login: String,
    name: Option<String>,
}

#[derive(Deserialize)]
struct Email {
    email: String,
    primary: bool,
    verified: bool,
}

#[derive(Deserialize)]
struct Membership {
    state: String,
}

#[derive(Deserialize)]
struct Team {
    slug: String,
    organization: Org,
}

#[derive(Deserialize)]
struct Org {
    login: String,
}

fn primary_email(emails: &[Email]) -> Option<String> {
    emails
        .iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email.clone())
}

/// The slugs of the teams in `org`.
fn teams_in(teams: &[Team], org: &str) -> Vec<String> {
    teams
        .iter()
        .filter(|t| t.organization.login.eq_ignore_ascii_case(org))
        .map(|t| t.slug.clone())
        .collect()
}

/// `a=1&b=2`, form-encoded.
fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            method: "GET".into(),
            path: "/_auth/sign-in".into(),
            query: vec![],
            form: vec![],
            origin: "https://apps.kitewell.com".into(),
            callback: "https://apps.kitewell.com/_auth/callback".into(),
            state: "s+1".into(),
            nonce: "n".into(),
            verifier: "v".into(),
            challenge: "c_1".into(),
        }
    }

    #[test]
    fn start_sends_the_state_the_callback_and_the_challenge() {
        let url = authorize_url("Iv1.abc", &request());
        assert_eq!(
            url,
            "https://github.com/login/oauth/authorize?client_id=Iv1.abc&redirect_uri=https%3A%2F%2Fapps.kitewell.com%2F_auth%2Fcallback\
             &state=s%2B1&scope=read%3Auser%20user%3Aemail%20read%3Aorg&code_challenge=c_1&code_challenge_method=S256&allow_signup=false"
        );
    }

    #[test]
    fn the_email_is_the_verified_primary_one() {
        let emails: Vec<Email> = serde_json::from_str(
            r#"[{"email":"old@kitewell.com","primary":false,"verified":true},
                {"email":"ann@kitewell.com","primary":true,"verified":true}]"#,
        )
        .unwrap();
        assert_eq!(primary_email(&emails).as_deref(), Some("ann@kitewell.com"));
        let unverified: Vec<Email> = serde_json::from_str(
            r#"[{"email":"ann@kitewell.com","primary":true,"verified":false}]"#,
        )
        .unwrap();
        assert_eq!(primary_email(&unverified), None);
    }

    #[test]
    fn groups_are_the_teams_in_the_org() {
        let teams: Vec<Team> = serde_json::from_str(
            r#"[{"slug":"marketing","organization":{"login":"Kitewell"}},
                {"slug":"rustaceans","organization":{"login":"elsewhere"}},
                {"slug":"platform","organization":{"login":"kitewell"}}]"#,
        )
        .unwrap();
        assert_eq!(teams_in(&teams, "kitewell"), ["marketing", "platform"]);
    }
}
