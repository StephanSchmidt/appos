//! Kitewell's own booking page, at /book: review. The customer sees the
//! quote, says who they are and books. The other slots are the pages of
//! sales's and product's apps.
//!
//!   GET  /book          where a booking starts: the sales app's package page
//!   GET  /book/:draft   review; POST books, at once
//!
//! The page needs the grant booking-book (policy.csv: `p, anyone, book,
//! booking-book`). It calls the store directly: no app between them.

use appos_shell::kit::{Context, KitRequest, KitRoute, Reply, Respond};
use askama::Template;

use crate::booking::{Booking, PAGES};
use crate::catalog;
use crate::store::looks_like_email;
use crate::types::{Billing, Customer, Draft, PaymentTerms, Slot};

const STYLE: &str = include_str!("../templates/style.css");

/// The sentence a customer agrees to before booking, as legal wrote it.
pub const CONSENT: &str =
    "I agree to Kitewell's terms of service and order terms, and I may book for my company.";

type Page = fn(&Booking, &Context, &KitRequest) -> anyhow::Result<Reply>;

pub fn routes(b: &Booking) -> Vec<KitRoute> {
    let route = |post: bool, path: &str, page: Page| {
        let b = b.clone();
        let respond = move |cx: Context, req: KitRequest| -> Respond {
            let reply = page(&b, &cx, &req);
            Box::pin(async move { reply })
        };
        if post {
            KitRoute::post(path, PAGES, respond)
        } else {
            KitRoute::get(path, PAGES, respond)
        }
    };
    vec![
        route(false, "/book", start),
        route(false, "/book/:draft", review),
        route(true, "/book/:draft", review_post),
    ]
}

/// A draft's quote in dollars.
struct QuoteView {
    lines: Vec<(String, String)>,
    per_year: String,
    years: u8,
    contract: String,
    invoices: String,
    problems: Vec<String>,
}

impl QuoteView {
    fn of(d: &Draft) -> QuoteView {
        let q = &d.quote;
        let every = match d.billing {
            Billing::Monthly => "a month",
            Billing::Quarterly => "a quarter",
            Billing::Yearly => "a year",
        };
        QuoteView {
            lines: q
                .lines
                .iter()
                .map(|l| (l.label.clone(), money(l.cents)))
                .collect(),
            per_year: money(cents(q.per_year_cents)),
            years: d.years,
            contract: money(cents(q.contract_cents)),
            invoices: format!(
                "One invoice {every}, {}, payable in {} days.",
                money(cents(q.per_invoice_cents)),
                days(d.payment_terms)
            ),
            problems: q.problems.clone(),
        }
    }
}

fn cents(c: u64) -> i64 {
    i64::try_from(c).unwrap_or(i64::MAX)
}

fn days(terms: PaymentTerms) -> u32 {
    match terms {
        PaymentTerms::Net30 => 30,
        PaymentTerms::Net45 => 45,
        PaymentTerms::Net60 => 60,
    }
}

/// 725000 as "$7,250", 516250 as "$5,162.50", -1274400 as "-$12,744".
fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.unsigned_abs();
    let dollars = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in dollars.chars().enumerate() {
        if i > 0 && (dollars.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match cents % 100 {
        0 => format!("{sign}${grouped}"),
        c => format!("{sign}${grouped}.{c:02}"),
    }
}

fn terms_in_words(d: &Draft) -> String {
    let billing = match d.billing {
        Billing::Monthly => "monthly",
        Billing::Quarterly => "quarterly",
        Billing::Yearly => "yearly",
    };
    let s = if d.years == 1 { "" } else { "s" };
    format!(
        "Billed {billing}, for {} year{s}, invoices payable in {} days",
        d.years,
        days(d.payment_terms)
    )
}

fn show(title: &str, t: &impl Template) -> anyhow::Result<Reply> {
    Ok(Reply::page(title, t.render()?).head(format!("<style>{STYLE}</style>")))
}

#[derive(Template)]
#[template(path = "book/gone.html")]
struct Gone;

fn gone() -> anyhow::Result<Reply> {
    Ok(show("Start again", &Gone)?.status(404))
}

/// The draft at /book/:draft while it can change. Otherwise the answer
/// instead: a booked one goes on to set up, an unknown one is gone.
fn open(b: &Booking, req: &KitRequest) -> anyhow::Result<Result<Draft, Reply>> {
    let id = req.param("draft").unwrap_or_default();
    if b.store.booking(id)?.is_some() {
        return Ok(Err(Reply::redirect(&b.link(id, Slot::Setup))));
    }
    Ok(match b.get(id)? {
        Some(draft) => Ok(draft),
        None => Err(gone()?),
    })
}

/// The draft, or the answer to give instead.
macro_rules! open_or_answer {
    ($b:expr, $req:expr) => {
        match open($b, $req)? {
            Ok(draft) => draft,
            Err(reply) => return Ok(reply),
        }
    };
}

/// GET /book: where a booking starts, the sales app's package page.
fn start(b: &Booking, _: &Context, _: &KitRequest) -> anyhow::Result<Reply> {
    match b.app_page(Slot::Choose) {
        Some(path) => Ok(Reply::redirect(path)),
        None => gone(),
    }
}

// Review -------------------------------------------------------------------

#[derive(Template)]
#[template(path = "book/review.html")]
struct ReviewPage {
    step: u8,
    action: String,
    /// What the draft says: label, value and where to change it.
    rows: Vec<(String, String, String)>,
    quote: QuoteView,
    company: String,
    name: String,
    email: String,
    company_error: String,
    name_error: String,
    email_error: String,
    consent: &'static str,
    consent_error: String,
}

fn review_page(b: &Booking, draft: &Draft) -> ReviewPage {
    let link = |slot| b.link(&draft.id, slot);
    let catalog = catalog::catalog();
    let add_ons: Vec<String> = catalog
        .add_ons
        .iter()
        .filter(|a| {
            draft.add_ons.contains(&a.id) || draft.plan.is_some_and(|p| a.included_in.contains(&p))
        })
        .map(|a| a.name.clone())
        .collect();
    let project = draft
        .template
        .as_deref()
        .and_then(catalog::template_name)
        .unwrap_or_else(|| "None, it starts empty".into());
    let mut terms = terms_in_words(draft);
    if let Some(offer) = &draft.offer {
        terms = format!("{terms}, from your offer {offer}");
    }
    ReviewPage {
        step: 4,
        action: format!("/book/{}", draft.id),
        rows: vec![
            (
                "Package".into(),
                format!("{} for {} people", draft.package, draft.users),
                link(Slot::Choose),
            ),
            (
                "Add-ons".into(),
                if add_ons.is_empty() {
                    "None".into()
                } else {
                    add_ons.join(", ")
                },
                link(Slot::Configure),
            ),
            ("First project".into(), project, link(Slot::Configure)),
            ("Terms".into(), terms, link(Slot::Terms)),
        ],
        quote: QuoteView::of(draft),
        company: String::new(),
        name: String::new(),
        email: String::new(),
        company_error: String::new(),
        name_error: String::new(),
        email_error: String::new(),
        consent: CONSENT,
        consent_error: String::new(),
    }
}

fn review(b: &Booking, _: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, req);
    if draft.plan.is_none() {
        return Ok(Reply::redirect(&b.link(&draft.id, Slot::Choose)));
    }
    show("Review and book", &review_page(b, &draft))
}

/// Books, at once, and goes on to set up.
fn review_post(b: &Booking, _: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, req);
    let field = |name: &str| req.form_value(name).unwrap_or_default().trim().to_string();
    let mut t = review_page(b, &draft);
    (t.company, t.name, t.email) = (field("company"), field("name"), field("email"));
    if t.company.is_empty() {
        t.company_error = "Your company's name, please".into();
    }
    if t.name.is_empty() {
        t.name_error = "Your name, please".into();
    }
    if !looks_like_email(&t.email) {
        t.email_error = "An email address like ann@acme.com, please".into();
    }
    if req.form_value("consent") != Some("yes") {
        t.consent_error = "Please agree to the terms to book".into();
    }
    let fine = [
        &t.company_error,
        &t.name_error,
        &t.email_error,
        &t.consent_error,
    ]
    .iter()
    .all(|e| e.is_empty());
    if fine {
        let customer = Customer {
            company: t.company.clone(),
            name: t.name.clone(),
            email: t.email.clone(),
        };
        match b.book(&draft.id, &customer, CONSENT)? {
            Ok(()) => {
                return Ok(Reply::redirect(&b.next(&draft.id, Slot::Review)));
            }
            // The quote's problems are on the page already.
            Err(problems) => t.quote.problems = problems,
        }
    }
    Ok(show("Review and book", &t)?.status(422))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::booking::tests::{booking, customer};

    #[test]
    fn money_in_dollars() {
        assert_eq!(money(725_000), "$7,250");
        assert_eq!(money(516_250), "$5,162.50");
        assert_eq!(money(-1_274_400), "-$12,744");
        assert_eq!(money(11_611_200), "$116,112");
    }

    #[test]
    fn the_review_page_renders() {
        let b = booking();
        let (id, _) = b.store.start().unwrap();
        let draft = b.with_offer(&id, "KW-ACME-2026").unwrap().unwrap();
        assert_eq!(
            terms_in_words(&draft),
            "Billed yearly, for 2 years, invoices payable in 30 days"
        );
        let quote = QuoteView::of(&draft);
        assert_eq!(
            (quote.per_year.as_str(), quote.contract.as_str()),
            ("$58,056", "$116,112")
        );
        let page = review_page(&b, &draft).render().unwrap();
        assert!(page.contains(&format!("/packages?draft={id}")), "{page}");
        assert!(page.contains("$116,112"), "{page}");

        b.book(&id, &customer(), CONSENT).unwrap().unwrap();
        assert!(b.store.booking(&id).unwrap().is_some());
    }
}
