//! The token sign-in: `appos-shell` started without `--auth`, `--policy`
//! and `--no-auth`. It prints a link with a token, new at every start;
//! whoever opens it (or types the token) signs in as `admin@localhost`, who
//! may do everything (`policy`). Nobody else may do anything. For trying the
//! shell out without setting up sign-in, and still closed to everyone else.

use super::Identity;
use super::routes::Step;
use crate::web::escape;

/// Who the token signs in.
pub const EMAIL: &str = "admin@localhost";

pub struct Token {
    secret: String,
}

impl Token {
    pub fn new(secret: String) -> Token {
        Token { secret }
    }

    /// The link (`?token=`) signs in at once; without one, a form for it.
    pub fn start(&self, query: &[(String, String)]) -> Step {
        match query.iter().find(|(k, _)| k == "token") {
            Some((_, token)) => self.check(token),
            None => page(None),
        }
    }

    /// The form's token.
    pub fn form(&self, form: &[(String, String)]) -> Step {
        let token = form.iter().find(|(k, _)| k == "token").map_or("", |(_, v)| v.as_str());
        self.check(token)
    }

    fn check(&self, token: &str) -> Step {
        if !same(token.trim(), &self.secret) {
            return page(Some("That is not this start's token: it changes each time appos-shell starts."));
        }
        match Identity::new("token", EMAIL, "Admin", []) {
            Ok(id) => Step::Done(id),
            Err(e) => Step::Refuse(e),
        }
    }
}

fn page(error: Option<&str>) -> Step {
    let error = error.map_or(String::new(), |why| format!(r#"<p class="appos-error" role="alert">{}</p>"#, escape(why)));
    Step::Page(format!(
        r#"<h1>Sign in</h1>
<p class="appos-muted">Open the link appos-shell printed when it started, or give its token. It signs you in as admin, who may do everything here.</p>
{error}<form class="appos-form" method="post" action="/_auth/sign-in">
<label>Token <input name="token" type="password" required autocomplete="off"></label>
<button>Sign in</button>
</form>"#
    ))
}

/// The same, in the same time whatever differs.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// The policy that goes with it: the token's admin may use every grant
/// in every app, also a Wasm kit's deployed later, and the admin screen.
pub fn policy() -> String {
    format!("p, {EMAIL}, *, *\n")
}

#[cfg(test)]
mod tests {
    use axum::http::{StatusCode, header};

    use super::super::routes::tests::{cookies, get, post, send};
    use super::*;
    use crate::kit::{ADMIN_READ, ADMIN_WRITE};

    fn session(headers: &axum::http::HeaderMap) -> Option<String> {
        cookies(headers).split("; ").find(|c| c.starts_with("appos-session=") && c.len() > "appos-session=".len()).map(str::to_string)
    }

    #[tokio::test]
    async fn the_link_signs_in_as_admin_and_nobody_else_may_do_anything() {
        let (_dir, shell) = crate::testing::shell_with_token("s3cret-token").await;
        let app = crate::web::router(shell.clone()).merge(crate::web::platform_router(shell.clone(), None));

        // Without it: the app's read is denied, the admin screen sends to sign in.
        let (status, _, _) = send(&app, get("/test", "")).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let (status, headers, _) = send(&app, get("/admin", "")).await;
        assert_eq!((status, &headers[header::LOCATION]), (StatusCode::SEE_OTHER, &"/_auth/sign-in?to=%2Fadmin".parse::<axum::http::HeaderValue>().unwrap()));
        let (status, _, body) = send(&app, get("/_auth/sign-in", "")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#"name="token""#), "{body}");

        // A wrong token: the form again, no session.
        let (status, headers, body) = send(&app, get("/_auth/sign-in?token=guess", "")).await;
        assert!(status == StatusCode::OK && body.contains("not this start&#39;s token") && session(&headers).is_none(), "{body}");

        // The link.
        let (status, headers, _) = send(&app, get("/_auth/sign-in?token=s3cret-token&to=/admin", "")).await;
        assert_eq!((status, &headers[header::LOCATION]), (StatusCode::SEE_OTHER, &"/admin".parse::<axum::http::HeaderValue>().unwrap()));
        let signed_in = session(&headers).unwrap();
        assert_eq!(send(&app, get("/admin", &signed_in)).await.0, StatusCode::OK);
        let (status, _, body) = send(&app, post("/test", &signed_in, "")).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // The form takes the token too.
        let (_, headers, _) = send(&app, get("/_auth/sign-in", "")).await;
        let (status, headers, _) = send(&app, post("/_auth/sign-in", &cookies(&headers), "token=s3cret-token")).await;
        assert!(status == StatusCode::SEE_OTHER && session(&headers).is_some());
    }

    #[tokio::test]
    async fn its_policy_is_every_grant_and_admin_for_the_token_only() {
        let mut grants = crate::kit::Kits::builtin().names();
        let p = crate::access::Policies::built_in(&policy(), grants.clone()).await.unwrap().current();
        grants.push("scoring-read".into());
        let admin = super::super::Who::signed_in(Identity::new("token", EMAIL, "Admin", []).unwrap());
        assert!(p.may(&admin, ADMIN_READ) && p.may(&admin, ADMIN_WRITE));
        for g in grants {
            assert!(p.access(&admin, "any", "Any", std::slice::from_ref(&g)).allows(0), "{g}");
            assert!(!p.access(&super::super::Who::default(), "any", "Any", std::slice::from_ref(&g)).allows(0), "{g}");
        }
        assert!(!p.may(&super::super::Who::default(), ADMIN_READ));
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "abcd"));
    }
}
