//! Greetings: an AppOS kit. A kit offers functions to the platform's apps
//! (api/), behind grants (kit.toml), and has a page of its own, where this
//! one lets you try its function. Change it into yours;
//! `appos guide kits` explains kits.
//!
//!   appos dev       runs the page at http://127.0.0.1:8080/<kit-id>;
//!                   changes rebuild and reload
//!   make check      formatting, lints, tests
//!   appos deploy    builds it and puts it on the current platform
//!
//! wit/ is the platform's contract: leave it to `appos sync`. api/ is the
//! kit's own.

// What the kit calls (import) and offers (export). To call another kit,
// add its grant to kit.toml's [capabilities] and its interface here, like
// `import acme:crm/customers@0.1.0;`; `appos sync` brings its WIT into
// wit/deps/ and a fake of it into fakes/, which appos dev runs.
wit_bindgen::generate!({
    inline: "package local:kit;
        world kit {
            import appos:framework/registry@0.1.0;
            import appos:framework/page@0.1.0;
            export appos:framework/handler@0.1.0;
            export kit:hello/greetings@0.1.0;
        }",
    path: ["api/hello.wit", "wit"],
    generate_all,
});

use appos::framework::types::Request;
use appos::framework::{page, registry};
use exports::kit::hello::greetings::Guest as Offered;

struct Kit;

/// What apps call. Each call runs in a fresh sandbox, like a request.
impl Offered for Kit {
    fn greet(name: String) -> String {
        greeting(&name)
    }
}

/// The kit's page, below /<kit-id>, built as an app builds its pages.
impl exports::appos::framework::handler::Guest for Kit {
    fn init() {
        registry::add_route("GET", "/", "try");
    }

    fn start(_handler: String, req: Request) {
        let name = req
            .query
            .iter()
            .find(|(key, _)| key == "name")
            .map_or("", |(_, value)| value.as_str());
        if req.htmx {
            // Only the answer changes.
            page::set_html("answer", &answer(name));
        } else {
            page::add_header("<title>Greetings</title>");
            page::add_header(&format!("<style>{STYLE}</style>"));
            page::set_html("page", &view(&req.base, name));
        }
    }
}

export!(Kit);

/// The greeting for `name`, as apps get it.
fn greeting(name: &str) -> String {
    match name.trim() {
        "" => "Hello!".to_string(),
        name => format!("Hello, {name}!"),
    }
}

/// The page: the call as a form, and what it returns. Without JavaScript
/// the form loads the page again; with htmx only the answer changes.
fn view(base: &str, name: &str) -> String {
    let base = escape(base);
    format!(
        "<div class=\"kit\">\
         <h1>Greetings</h1>\
         <p>Apps on this platform call <code>greet</code> with a name and \
         show what it returns. Try one.</p>\
         <form class=\"call\" method=\"get\" action=\"{base}/\" hx-get=\"{base}/\" hx-swap=\"none\">\
         <span class=\"fn\" aria-hidden=\"true\">greet(</span>\
         <input name=\"name\" value=\"{name}\" placeholder=\"Ada\" aria-label=\"Name\" autocomplete=\"off\">\
         <span aria-hidden=\"true\">)</span>\
         <button>Call</button>\
         </form>\
         <p class=\"answer\" id=\"answer\" aria-live=\"polite\">{answer}</p>\
         </div>",
        name = escape(name),
        answer = answer(name),
    )
}

/// What greet returns, as a string literal.
fn answer(name: &str) -> String {
    format!("\"{}\"", escape(&greeting(name)))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The page's design. The shell adds its top bar above it.
const STYLE: &str = "
.kit { max-width: 40rem; margin: 0 auto; padding: 4rem 1.25rem;
  font: 17px/1.55 ui-sans-serif, system-ui, -apple-system, 'Segoe UI', sans-serif; color: #0f1222; }
.kit h1 { font-size: 2.4rem; letter-spacing: -.025em; line-height: 1.1; margin: 0 0 .75rem; }
.kit p { color: #4a4f63; margin: 0 0 2.25rem; }
.kit code, .call, .kit .answer { font-family: ui-monospace, 'SF Mono', Menlo, Consolas, monospace; }
.kit code { font-size: .95em; color: #0f1222; }
.call { display: flex; align-items: center; flex-wrap: wrap; gap: .5rem .35rem; margin: 0;
  padding: 1.1rem 1.25rem; font-size: 1.3rem; background: #fff;
  border: 2px solid #0f1222; border-radius: 6px; box-shadow: 6px 6px 0 #ffd23f; }
.call .fn { font-weight: 600; }
.call input { flex: 1 1 6ch; min-width: 6ch; max-width: 24ch; font: inherit; color: inherit; background: transparent;
  border: 0; border-bottom: 2px dashed #9aa0b4; padding: .1rem .2rem; }
.call input:focus { outline: none; border-bottom: 2px solid #0f1222; }
.call button { margin-left: auto; cursor: pointer; font: 600 1rem/1 ui-sans-serif, system-ui, sans-serif;
  padding: .65rem 1.2rem; color: #0f1222; background: #ffd23f; border: 2px solid #0f1222; border-radius: 6px; }
.call button:focus-visible { outline: 3px solid #6d5efc; outline-offset: 2px; }
.kit .answer { margin: 1.75rem 0 0; font-size: 1.3rem; color: #0f1222; overflow-wrap: anywhere; }
.kit .answer::before { content: 'returns '; color: #7a8096; font-size: .8em; }
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_makes_the_greeting() {
        assert_eq!(greeting("Ada"), "Hello, Ada!");
        assert_eq!(greeting("  Ada "), "Hello, Ada!");
        assert_eq!(greeting(""), "Hello!");
    }

    #[test]
    fn what_visitors_type_stays_text() {
        assert_eq!(answer("<b>"), "\"Hello, &lt;b&gt;!\"");
        let page = view("/hello", "\"><script>");
        assert!(!page.contains("<script>"));
        assert!(page.contains("value=\"&quot;&gt;&lt;script&gt;\""));
    }
}
