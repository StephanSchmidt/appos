//! The package page and the terms page: parse what the customer sent, call
//! the platform's booking flow, show what it answers. Errors end here: the
//! platform's reasons ("Starter is for up to 10 users") go on the page as
//! they are.

use askama::Template;

use super::domain::{self, Card, Mark, SALES, Slider};
use crate::appos::kitewell::booking::flow::{self, Billing, Draft, PaymentTerms, Slot};
use crate::appos::kitewell::booking::{choose, terms as kit_terms};
use crate::appos::{self, Request, html, log, page};
use crate::quote::{QuoteView, billing_name, days};

#[derive(Template)]
#[template(path = "packages/index.html", blocks = ["cards", "users_box", "slider_box"])]
struct Index {
    base: String,
    step: u8,
    /// The draft's id; empty until the customer chooses something.
    draft: String,
    /// The team size as typed.
    users: String,
    /// The slider's stop for that size, its last stop and its labels.
    slider: u32,
    slider_last: u32,
    marks: Vec<Mark>,
    cards: Vec<Card>,
    /// More users than any package: the cards say so, and where to write.
    largest: Option<u32>,
    sales: &'static str,
    error: String,
    code: String,
    code_error: String,
}

#[derive(Template)]
#[template(path = "packages/terms.html", blocks = ["quote_block", "terms_error"])]
struct Terms {
    base: String,
    step: u8,
    draft: String,
    package: String,
    users: u32,
    /// The offer's code; empty without one.
    offer: String,
    /// The offer's terms in words.
    offer_terms: String,
    monthly: bool,
    years: Vec<Year>,
    error: String,
    next: String,
    back: String,
    quote: QuoteView,
}

struct Year {
    value: u8,
    label: &'static str,
    selected: bool,
}

/// The team size the page starts with, as on the pricing page.
const FIRST_SIZE: &str = "10";

pub fn index(req: &Request) {
    let draft = open(req.query("draft"));
    // The slider sends its stop as `slider` and leaves the box out; the box
    // sends what was typed as `users`.
    let slider = Slider::new(&flow::catalog());
    let slid = req
        .query("slider")
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|s| *s <= slider.last());
    let typed = match (slid, req.query("users")) {
        (Some(stop), _) => slider.users(stop).to_string(),
        (None, Some(typed)) => typed.to_string(),
        (None, None) => draft
            .as_ref()
            .filter(|d| d.users > 0)
            .map_or_else(|| FIRST_SIZE.to_string(), |d| d.users.to_string()),
    };
    let t = index_page(req, draft.as_ref(), &typed);
    if req.htmx {
        page::set_html("cards", &html(&t.as_cards()));
        // Each control moves the other, never itself: a replaced slider
        // stops the drag, a replaced box loses what is being typed.
        if slid.is_some() {
            page::set_html("users_box", &html(&t.as_users_box()));
        } else if t.error.is_empty() {
            page::set_html("slider_box", &html(&t.as_slider_box()));
        }
    } else {
        show(&t, "Choose your package");
    }
}

pub fn choose(req: &Request) {
    let before = open(req.form("draft"));
    let typed = req.form("users").unwrap_or("");
    let Some(package) = req.form("package").and_then(domain::package) else {
        return again(req, before.as_ref(), typed, "Choose one of the packages");
    };
    let users = match domain::users(typed) {
        Ok(users) => users,
        Err(why) => return again(req, before.as_ref(), typed, why),
    };
    let catalog = flow::catalog();
    let plan = catalog.plans.iter().find(|p| p.plan == package.plan);
    if let Some(why) =
        plan.and_then(|p| domain::unavailable(package, p, users, catalog.enterprise_from))
    {
        return again(req, before.as_ref(), typed, &why);
    }
    let id = before
        .as_ref()
        .map_or_else(|| choose::start().id, |d| d.id.clone());
    if let Err(why) = choose::with_plan(&id, package.plan, users, package.name) {
        return again(req, flow::get(&id).as_ref(), typed, &why);
    }
    // A package starts with its own contract length: two years for the
    // Agency package. The customer changes it on the terms page.
    if before.as_ref().is_none_or(|d| d.package != package.name)
        && let Err(why) =
            kit_terms::with_terms(&id, Billing::Yearly, package.years, PaymentTerms::Net30)
    {
        log::warn(&format!("the {} package's terms: {why}", package.name));
    }
    page::redirect(&flow::next(&id, Slot::Choose));
}

pub fn offer(req: &Request) {
    let before = open(req.form("draft"));
    let code = req.form("code").unwrap_or("").trim();
    if code.is_empty() {
        return offer_again(
            req,
            before.as_ref(),
            code,
            "Type the code from your offer, like KW-ACME-2026",
        );
    }
    let id = before
        .as_ref()
        .map_or_else(|| choose::start().id, |d| d.id.clone());
    match choose::with_offer(&id, code) {
        Ok(_) => page::redirect(&flow::next(&id, Slot::Choose)),
        Err(why) => offer_again(req, flow::get(&id).as_ref(), code, &why),
    }
}

pub fn terms(req: &Request) {
    let Some(draft) = open(req.query("draft")) else {
        return gone(req);
    };
    if draft.plan.is_none() {
        return page::redirect(&flow::link(&draft.id, Slot::Choose));
    }
    show(&terms_page(req, &draft), "How you pay");
}

pub fn set_terms(req: &Request) {
    let Some(draft) = open(req.form("draft")) else {
        return gone(req);
    };
    let billing = match req.form("billing") {
        Some("monthly") => Billing::Monthly,
        _ => Billing::Yearly,
    };
    let years = req
        .form("years")
        .and_then(|y| y.parse::<u8>().ok())
        .unwrap_or(1);
    match kit_terms::with_terms(&draft.id, billing, years, PaymentTerms::Net30) {
        // A change on the page: the new price, and no error any more.
        Ok(changed) if req.htmx => {
            let t = terms_page(req, &changed);
            page::set_html("quote", &html(&t.as_quote_block()));
            page::set_html("terms_error", "");
        }
        Ok(_) => page::redirect(&flow::next(&draft.id, Slot::Terms)),
        Err(why) => {
            page::set_status(422);
            if req.htmx {
                page::set_html("terms_error", &appos::escape(&why));
            } else {
                let mut t = terms_page(req, &draft);
                t.error = why;
                t.monthly = billing == Billing::Monthly;
                show(&t, "How you pay");
            }
        }
    }
}

/// The draft with this id while it can still change; none for no id, an
/// unknown one, or one that is booked: the customer starts again.
fn open(id: Option<&str>) -> Option<Draft> {
    let id = id?.trim();
    if id.is_empty() || flow::booked(id).is_some() {
        return None;
    }
    flow::get(id)
}

fn index_page(req: &Request, draft: Option<&Draft>, typed: &str) -> Index {
    let catalog = flow::catalog();
    let (users, error) = match typed.trim() {
        "" => (0, ""),
        typed => match domain::users(typed) {
            Ok(users) => (users, ""),
            Err(why) => (0, why),
        },
    };
    let slider = Slider::new(&catalog);
    Index {
        base: req.base.clone(),
        step: 1,
        draft: draft.map(|d| d.id.clone()).unwrap_or_default(),
        users: typed.trim().to_string(),
        slider: slider.stop(if users == 0 { 10 } else { users }),
        slider_last: slider.last(),
        marks: slider.marks(&catalog),
        cards: domain::cards(&catalog, users, draft.map_or("", |d| d.package.as_str())),
        largest: (users >= catalog.enterprise_from).then(|| catalog.enterprise_from - 1),
        sales: SALES,
        error: error.to_string(),
        code: draft.and_then(|d| d.offer.clone()).unwrap_or_default(),
        code_error: String::new(),
    }
}

/// The package page again, with why the choice didn't go through.
fn again(req: &Request, draft: Option<&Draft>, typed: &str, why: &str) {
    page::set_status(422);
    let mut t = index_page(req, draft, typed);
    t.error = why.to_string();
    show(&t, "Choose your package");
}

fn offer_again(req: &Request, draft: Option<&Draft>, code: &str, why: &str) {
    page::set_status(422);
    let typed = draft
        .filter(|d| d.users > 0)
        .map(|d| d.users.to_string())
        .unwrap_or_default();
    let mut t = index_page(req, draft, &typed);
    (t.code, t.code_error) = (code.to_string(), why.to_string());
    show(&t, "Choose your package");
}

fn terms_page(req: &Request, draft: &Draft) -> Terms {
    let offer = draft.offer.clone().unwrap_or_default();
    let offer_terms = format!(
        "Billed {}, for {} year{}, invoices payable in {} days.",
        billing_name(draft.billing).to_lowercase(),
        draft.years,
        if draft.years == 1 { "" } else { "s" },
        days(draft.payment_terms)
    );
    let years = [(1, "1 year"), (2, "2 years"), (3, "3 years")]
        .into_iter()
        .map(|(value, label)| Year {
            value,
            label,
            selected: value == draft.years,
        })
        .collect();
    Terms {
        base: req.base.clone(),
        step: 3,
        draft: draft.id.clone(),
        package: draft.package.clone(),
        users: draft.users,
        offer,
        offer_terms,
        monthly: draft.billing == Billing::Monthly,
        years,
        error: String::new(),
        next: flow::next(&draft.id, Slot::Terms),
        back: flow::link(&draft.id, Slot::Configure),
        quote: QuoteView::of(draft),
    }
}

#[derive(Template)]
#[template(path = "packages/gone.html")]
struct Gone {
    base: String,
}

/// A link to a draft that isn't open any more: booked, or never there.
fn gone(req: &Request) {
    page::set_status(404);
    show(
        &Gone {
            base: req.base.clone(),
        },
        "Start again",
    );
}

fn show(t: &impl Template, title: &str) {
    appos::style(include_str!("../../templates/style.css"));
    appos::title(title);
    page::set_html("page", &html(t));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appos::kitewell::booking::flow::{Line, Plan, Quote};

    fn draft(offer: Option<&str>) -> Draft {
        Draft {
            id: "d1".into(),
            package: "Agency package".into(),
            plan: Some(Plan::Business),
            users: 40,
            add_ons: vec![],
            template: None,
            billing: Billing::Quarterly,
            years: 2,
            payment_terms: PaymentTerms::Net45,
            offer: offer.map(str::to_string),
            quote: Quote {
                lines: vec![Line {
                    label: "Business, 40 users, quarterly".into(),
                    cents: 2_360_000,
                }],
                billed_users: 40,
                per_year_cents: 2_065_000,
                contract_cents: 4_130_000,
                per_invoice_cents: 516_250,
                problems: vec![],
            },
        }
    }

    fn terms(draft: &Draft) -> Terms {
        Terms {
            base: "/packages".into(),
            step: 3,
            draft: draft.id.clone(),
            package: draft.package.clone(),
            users: draft.users,
            offer: draft.offer.clone().unwrap_or_default(),
            offer_terms: "Billed quarterly, for 2 years, invoices payable in 45 days.".into(),
            monthly: false,
            years: vec![Year {
                value: 2,
                label: "2 years",
                selected: true,
            }],
            error: String::new(),
            next: "/book?draft=d1".into(),
            back: "/workspace/configure?draft=d1".into(),
            quote: QuoteView::of(draft),
        }
    }

    #[test]
    fn an_offers_terms_are_read_only() {
        let page = terms(&draft(Some("KW-NORDLICHT-2026"))).render().unwrap();
        assert!(page.contains("These terms come with your offer KW-NORDLICHT-2026"));
        assert!(page.contains(r#"href="/book?draft=d1""#));
        assert!(!page.contains(r#"name="billing""#));
        assert!(page.contains("One invoice a quarter, $5,162.50, payable in 45 days."));
        assert!(page.contains("$41,300"), "the two years");
    }

    #[test]
    fn without_an_offer_the_customer_picks() {
        let page = terms(&draft(None)).render().unwrap();
        assert!(page.contains(r#"name="billing" value="yearly" checked"#));
        assert!(page.contains(
            r#"<input type="radio" name="years" value="2" checked><span>2 years</span>"#
        ));
    }
}
