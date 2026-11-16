//! The dev provider: anyone signs in as anyone, by typing an email and
//! groups. For trying sign-in and policies on localhost (auth.toml refuses
//! it on any other address).

use super::Identity;
use super::routes::Step;
use crate::web::escape;

/// The form, with what was typed and what is wrong, if anything.
pub fn start(again: Option<(&[(String, String)], &str)>) -> Step {
    let field = |name: &str| again.and_then(|(form, _)| form.iter().find(|(k, _)| k == name)).map_or("", |(_, v)| v.as_str()).to_string();
    let error = again.map_or(String::new(), |(_, why)| format!(r#"<p class="appos-error" role="alert">{}</p>"#, escape(why)));
    Step::Page(format!(
        r#"<h1>Sign in</h1>
<p class="appos-muted">Dev sign-in: anyone can be anyone here. Groups are what the policy names as group:&lt;name&gt;.</p>
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
    fn a_bad_email_shows_the_form_again_with_what_was_typed() {
        let Step::Page(html) = form(&form_of(&[("email", "<ann>"), ("groups", "x")])) else { panic!() };
        assert!(html.contains("is not an email address"));
        assert!(html.contains(r#"value="&lt;ann&gt;""#) && html.contains(r#"value="x""#));
    }
}
