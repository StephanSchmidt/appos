//! Customer scores for Acme: a Wasm kit (kit.toml) on acme-platform. It
//! calls the CRM kit (acme:crm/customers, grant crm-read), offers apps a
//! score per customer (api/scoring.wit, grant scoring-read), and shows the
//! scores at /scoring. Apps call it like any kit: `appos sync` against the
//! platform brings its WIT once it is deployed.
//!
//!   make check      # formatting, lints, tests
//!   appos deploy    # builds it and deploys it to the platform you are
//!                   # logged in to, as for an app
//!
//! It keeps nothing: the CRM has the customers and their notes.

wit_bindgen::generate!({
    inline: "package acme:scoring-kit;
        world kit {
            import acme:crm/customers@0.1.0;
            import appos:framework/registry@0.1.0;
            import appos:framework/page@0.1.0;
            export appos:framework/handler@0.1.0;
            export acme:scoring/score@0.1.0;
        }",
    path: ["api/scoring.wit", "wit"],
    generate_all,
});

use acme::crm::customers::{self, Customer};
use appos::framework::types::Request;
use appos::framework::{page, registry};

struct Scoring;

impl exports::acme::scoring::score::Guest for Scoring {
    fn score(customer: String) -> Option<u32> {
        customers::get(&customer).map(|c| score(&c))
    }
}

impl exports::appos::framework::handler::Guest for Scoring {
    fn init() {
        registry::add_route("GET", "/", "list");
    }

    fn start(_handler: String, req: Request) {
        let q = req
            .query
            .iter()
            .find(|(k, _)| k == "q")
            .map_or("", |(_, v)| v.trim());
        page::add_header("<title>Customer scores</title>");
        page::set_html("page", &list(&req.base, q));
    }
}

export!(Scoring);

fn score(c: &Customer) -> u32 {
    revenue_points(c.revenue) + note_points(customers::notes(&c.id).len())
}

/// For revenue in the last 12 months, in euros.
fn revenue_points(revenue: u32) -> u32 {
    match revenue {
        1_000_000.. => 60,
        250_000.. => 45,
        50_000.. => 30,
        _ => 15,
    }
}

fn note_points(notes: usize) -> u32 {
    u32::try_from(notes)
        .unwrap_or(u32::MAX)
        .saturating_mul(10)
        .min(40)
}

/// The customers whose name has `q`, best score first.
fn list(base: &str, q: &str) -> String {
    let mut scored: Vec<(u32, Customer)> = customers::search(q)
        .into_iter()
        .map(|c| (score(&c), c))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let rows: String = scored
        .iter()
        .map(|(score, c)| {
            format!(
                "<tr><td>{}</td><td>{}</td><td class=\"n\">{}</td><td class=\"n\"><b>{score}</b></td></tr>",
                escape(&c.name),
                escape(&c.owner),
                euros(c.revenue)
            )
        })
        .collect();
    let rows = if rows.is_empty() {
        "<tr><td colspan=\"4\">No customer has that in its name.</td></tr>".to_string()
    } else {
        rows
    };
    format!(
        "<h1>Customer scores</h1>\
         <p>Revenue gives up to 60 points, notes in the CRM up to 40.</p>\
         <form method=\"get\" action=\"{}\"><input name=\"q\" value=\"{}\" placeholder=\"Customer name\"> <button>Find</button></form>\
         <table><thead><tr><th>Customer</th><th>Owner</th><th class=\"n\">Revenue</th><th class=\"n\">Score</th></tr></thead><tbody>{rows}</tbody></table>",
        escape(base),
        escape(q)
    )
}

/// Like "1,250,000 €".
fn euros(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, d) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(d);
    }
    out + " €"
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revenue_and_notes_make_the_score() {
        assert_eq!(
            [12_000, 50_000, 480_000, 1_250_000].map(revenue_points),
            [15, 30, 45, 60]
        );
        assert_eq!([0, 1, 4, 9].map(note_points), [0, 10, 40, 40]);
    }

    #[test]
    fn numbers_and_text_read_well() {
        assert_eq!(
            [7, 12_000, 1_250_000].map(euros),
            ["7 €", "12,000 €", "1,250,000 €"]
        );
        assert_eq!(
            escape("<a href=\"x\">&</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;"
        );
    }
}
