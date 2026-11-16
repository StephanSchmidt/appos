//! The hello module's handlers: parse the input, call usecases with the
//! parsed values, and turn what they return, errors included, into the
//! page. Errors are handled here and only here (see ERRORS in appos guide).

use askama::Template;

use super::domain::{self, Greeting, GreetingId, Name, NameError};
use super::greet_usecase::greet_uc;
use super::list_usecase::list_uc;
use super::show_usecase::{ShowError, show_uc};
use crate::appos::{self, Request, html, page};

/// The page. `greeting`, `history` and `name_error` are blocks in the
/// template that can also be rendered alone, for htmx to swap in.
#[derive(Template)]
#[template(
    path = "hello/index.html",
    blocks = ["greeting", "history", "name_error"]
)]
struct Index {
    base: String,
    /// Newest first; the first one is the greeting.
    history: Vec<Greeting>,
    /// What was typed into the form, and why it didn't parse.
    name: String,
    error: Option<NameError>,
}

/// One greeting's page.
#[derive(Template)]
#[template(path = "hello/greeting.html")]
struct GreetingPage {
    base: String,
    greeting: Greeting,
}

pub fn index(req: &Request) {
    full_page(req, "", None);
}

pub fn greet(req: &Request) {
    let typed = req.form("name").unwrap_or("");
    let name = match Name::parse(typed) {
        Ok(name) => name,
        Err(error) => return invalid(req, typed, error),
    };
    greet_uc(name, req.now());
    if req.htmx {
        // Only these parts change; the form keeps its text and focus.
        let t = Index {
            base: req.base.clone(),
            history: list_uc(),
            name: String::new(),
            error: None,
        };
        page::set_html("greeting", &html(&t.as_greeting()));
        page::set_html("history", &html(&t.as_history()));
        // Clears an error from an earlier try.
        page::set_html("name_error", &html(&t.as_name_error()));
    } else {
        // Without JavaScript: back to the page, which shows the new state.
        page::redirect(&req.url("/"));
    }
}

pub fn greeting(req: &Request) {
    // Text that isn't an id is as unknown as an id nobody has: 404.
    let Some(id) = req.param("id").and_then(GreetingId::parse) else {
        return appos::not_found();
    };
    // Every variant by name, no `_`: a new kind of error doesn't compile
    // until this handler decides what the visitor sees.
    match show_uc(id) {
        Ok(greeting) => {
            appos::style(include_str!("../../templates/style.css"));
            appos::title(&format!("Hello, {}", greeting.name));
            let t = GreetingPage {
                base: req.base.clone(),
                greeting,
            };
            page::set_html("page", &html(&t));
        }
        Err(ShowError::NotFound) => appos::not_found(),
        Err(ShowError::Unreadable(e)) => appos::unreadable(&e),
    }
}

/// Input that doesn't parse: status 422 and the error next to the field.
/// With htmx only the error changes; without, the whole page comes back
/// with what was typed.
fn invalid(req: &Request, typed: &str, error: NameError) {
    page::set_status(422);
    if req.htmx {
        let t = Index {
            base: req.base.clone(),
            history: vec![],
            name: String::new(),
            error: Some(error),
        };
        page::set_html("name_error", &html(&t.as_name_error()));
    } else {
        full_page(req, typed, Some(error));
    }
}

fn full_page(req: &Request, name: &str, error: Option<NameError>) {
    appos::style(include_str!("../../templates/style.css"));
    appos::title("Hello");
    let t = Index {
        base: req.base.clone(),
        history: list_uc(),
        name: name.into(),
        error,
    };
    page::set_html("page", &html(&t));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appos::time::Time;

    fn now() -> Time {
        Time::parse("2026-11-11T14:05:09+01:00").unwrap()
    }

    fn index(history: Vec<Greeting>, name: &str, error: Option<NameError>) -> Index {
        Index {
            base: "/hello".into(),
            history,
            name: name.into(),
            error,
        }
    }

    fn ann() -> Greeting {
        Greeting::new(Name::parse("Ann").unwrap(), now())
    }

    #[test]
    fn page_before_any_hello() {
        let page = index(vec![], "", None).render().unwrap();
        assert!(page.contains("Hello!"));
        assert!(page.contains("Nobody yet"));
        assert!(page.contains(r#"action="/hello/greet""#));
        assert!(page.contains(r#"maxlength="40""#));
    }

    #[test]
    fn greeting_and_history_show_the_newest_hello() {
        let greeting = ann();
        let link = format!(r#"href="/hello/greetings/{}""#, greeting.id);
        let t = index(vec![greeting], "", None);
        assert!(t.as_greeting().render().unwrap().contains("Hello, Ann!"));
        let history = t.as_history().render().unwrap();
        assert!(history.contains("Ann"));
        assert!(history.contains("14:05"));
        assert!(history.contains(&link));
    }

    #[test]
    fn an_error_shows_next_to_what_was_typed() {
        let error = NameError::TooLong { max: 40 };
        let page = index(vec![], "Annnn", Some(error)).render().unwrap();
        assert!(page.contains(r#"value="Annnn""#));
        assert!(page.contains("keep it to 40 characters"));
    }

    #[test]
    fn no_error_renders_nothing() {
        let t = index(vec![], "", None);
        assert_eq!(t.as_name_error().render().unwrap(), "");
    }

    #[test]
    fn a_greetings_page_shows_when() {
        let t = GreetingPage {
            base: "/hello".into(),
            greeting: ann(),
        };
        let page = t.render().unwrap();
        assert!(page.contains("Hello, Ann!"));
        assert!(page.contains("2026-11-11 at 14:05"));
    }

    #[test]
    fn names_are_escaped() {
        let evil = Greeting::new(Name::parse("<b>Ann</b>").unwrap(), now());
        let page = index(vec![evil], "<b>", None).render().unwrap();
        assert!(!page.contains("<b>"));
    }
}
