//! The dev provider: anyone signs in as anyone, by typing an email and
//! groups, a test user filled in. For trying sign-in and policies on
//! localhost (auth.toml refuses it on any other address), and in
//! `appos dev`, to try pages signed in and anonymous.

use super::Identity;
use super::routes::Step;
use crate::web::escape;

/// Who the form signs in unless changed.
const TEST_EMAIL: &str = "test@example.com";
const TEST_NAME: &str = "Test User";

/// The form, with what was typed and what is wrong, if anything; at first
/// the test user.
pub fn start(again: Option<(&[(String, String)], &str)>) -> Step {
    let field = |name: &str| match again {
        Some((form, _)) => form.iter().find(|(k, _)| k == name).map_or("", |(_, v)| v.as_str()).to_string(),
        None => [("email", TEST_EMAIL), ("name", TEST_NAME)].iter().find(|(k, _)| *k == name).map_or("", |(_, v)| *v).to_string(),
    };
    let error = again.map_or(String::new(), |(_, why)| format!(r#"<p class="appos-error" role="alert">{}</p>"#, escape(why)));
    Step::Page(format!(
        r#"<h1>Sign in</h1>
<p class="appos-muted">Dev sign-in: be the test user, or anyone you type. Groups are what the policy names as group:&lt;name&gt;.</p>
{error}<form class="appos-form" method="post" action="/_auth/sign-in">
<label>Email <input name="email" type="email" required autocomplete="email" value="{email}"></label>
<label>Name <input name="name" autocomplete="name" value="{name}"></label>
<label>Groups <input name="groups" placeholder="sales, platform" value="{groups}"></label>
<button>Sign in</button>
</form>"#,
        email = escape(&field("email")),
        name = escape(&field("name")),
        groups = escape(&field("groups")),
    ))
}

pub fn form(form: &[(String, String)]) -> Step {
    let field = |name: &str| form.iter().find(|(k, _)| k == name).map_or("", |(_, v)| v.as_str());
    let groups = field("groups").split(',').map(str::to_string);
    match Identity::new("", field("email"), field("name"), groups) {
        Ok(id) => Step::Done(id),
        Err(why) => start(Some((form, &why))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form_of(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn the_form_signs_in_with_email_name_and_groups() {
        let Step::Done(id) = form(&form_of(&[("email", "ann@acme.com"), ("name", "Ann"), ("groups", "sales, platform")])) else { panic!() };
        assert_eq!((id.email.as_str(), id.name.as_str()), ("ann@acme.com", "Ann"));
        assert_eq!(id.groups, ["sales", "platform"]);
    }

    #[test]
    fn the_form_starts_with_the_test_user() {
        let Step::Page(html) = start(None) else { panic!() };
        assert!(html.contains(r#"value="test@example.com""#) && html.contains(r#"value="Test User""#), "{html}");
        let Step::Done(id) = form(&form_of(&[("email", TEST_EMAIL), ("name", "Test User"), ("groups", "")])) else { panic!() };
        assert_eq!((id.email.as_str(), id.name.as_str(), id.groups.len()), ("test@example.com", "Test User", 0));
    }

    #[tokio::test]
    async fn appos_dev_tries_pages_as_the_test_user_and_anonymous() {
        use super::super::routes::tests::{cookies, get, post, send};
        let dir = tempfile::tempdir().unwrap();
        let auth = super::super::Auth::dev("http://localhost", dir.path()).unwrap();
        let parts = crate::testing::parts(dir.path().join("data"), Default::default());
        let shell = crate::shell::Shell::new(crate::shell::Parts { auth: Some(auth), ..parts }).unwrap();
        let who = crate::bundle::Bundle {
            manifest: "[app]\nid = 'test'\nname = 'Test'\nversion = '1'\nowner = 'T'\ngroup = 'T'\n[capabilities]\ngrant = ['user-read']\n".into(),
            wasm: include_bytes!("../testing-user.wat").to_vec(),
            icon: None,
            deps: None,
            kit: None,
        };
        crate::deploy::deploy(&shell, &who.pack().unwrap(), crate::platform::Actor::Cli).await.unwrap();
        let app = crate::web::router(shell);
        let (_, _, body) = send(&app, get("/test", "")).await;
        assert!(body.contains("nobody") && body.contains(">Sign in</a>"), "anonymous at first: {body}");
        let (_, headers, body) = send(&app, get("/_auth/sign-in?to=/test", "")).await;
        assert!(body.contains(r#"value="test@example.com""#), "{body}");
        let (_, headers, _) = send(&app, post("/_auth/sign-in", &cookies(&headers), "email=test%40example.com&name=Test+User&groups=")).await;
        let session = cookies(&headers).split("; ").find(|c| c.starts_with("appos-session=")).unwrap().to_string();
        let (_, _, body) = send(&app, get("/test", &session)).await;
        assert!(body.contains("test@example.com") && body.contains("Sign out"), "the test user: {body}");
    }

    #[test]
    fn a_bad_email_shows_the_form_again_with_what_was_typed() {
        let Step::Page(html) = form(&form_of(&[("email", "<ann>"), ("groups", "x")])) else { panic!() };
        assert!(html.contains("is not an email address"));
        assert!(html.contains(r#"value="&lt;ann&gt;""#) && html.contains(r#"value="x""#));
    }
}
