//! Kitewell's own booking pages, at /book. Review is always here: the
//! customer sees the quote, says who they are and books. For every other
//! slot there is a plain page that takes over while the app owning the
//! slot doesn't serve, paused or not deployed, so the flow never breaks:
//! sales's and product's apps are what customers usually see.
//!
//!   GET  /book                  the package; the sales app's page if it serves
//!   POST /book                  starts a draft with the package or an offer
//!   GET  /book/:draft/choose    the plain pages of the slots, and their forms
//!   GET  /book/:draft/configure
//!   GET  /book/:draft/terms
//!   GET  /book/:draft           review; POST books, at once
//!   GET  /book/:draft/setup     the workspace and its members
//!
//! The pages need the grant booking-book (policy.csv: `p, anyone, book,
//! booking-book`). They call the store directly: no app between them.

use appos_shell::kit::{Context, KitRequest, KitRoute, Reply, Respond};
use askama::Template;

use crate::booking::{Booking, PAGES};
use crate::catalog;
use crate::store::looks_like_email;
use crate::types::{Billing, Booking as Booked, Customer, Draft, PaymentTerms, Plan, Slot};

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
        route(true, "/book", choose_post),
        route(false, "/book/:draft/choose", choose),
        route(true, "/book/:draft/choose", choose_post),
        route(false, "/book/:draft/configure", configure),
        route(true, "/book/:draft/configure", configure_post),
        route(false, "/book/:draft/terms", terms),
        route(true, "/book/:draft/terms", terms_post),
        route(false, "/book/:draft", review),
        route(true, "/book/:draft", review_post),
        route(false, "/book/:draft/setup", setup),
        route(true, "/book/:draft/setup", setup_post),
    ]
}

/// A choice on a page: a radio button, a checkbox or an option.
struct Opt {
    value: String,
    label: String,
    note: String,
    checked: bool,
    /// Shown, but not the customer's to change.
    fixed: bool,
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

fn plan_named(name: &str) -> Option<Plan> {
    catalog::catalog()
        .plans
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .map(|p| p.plan)
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
fn open(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Result<Draft, Reply>> {
    let id = req.param("draft").unwrap_or_default();
    if b.store.booking(id)?.is_some() {
        return Ok(Err(Reply::redirect(&b.link(
            &|app| cx.serves(app),
            id,
            Slot::Setup,
        ))));
    }
    Ok(match b.get(id)? {
        Some(draft) => Ok(draft),
        None => Err(gone()?),
    })
}

/// The draft, or the answer to give instead.
macro_rules! open_or_answer {
    ($b:expr, $cx:expr, $req:expr) => {
        match open($b, $cx, $req)? {
            Ok(draft) => draft,
            Err(reply) => return Ok(reply),
        }
    };
}

// Choose -------------------------------------------------------------------

#[derive(Template)]
#[template(path = "book/choose.html")]
struct ChoosePage {
    step: u8,
    action: String,
    users: String,
    plans: Vec<Opt>,
    error: String,
    code: String,
    code_error: String,
}

fn choose_page(
    draft: Option<&Draft>,
    users: &str,
    error: String,
    code: &str,
    code_error: String,
) -> ChoosePage {
    let plans = catalog::catalog()
        .plans
        .into_iter()
        .map(|p| Opt {
            value: p.name.clone(),
            label: format!(
                "{}, {} per user per month",
                p.name,
                money(i64::from(p.monthly_cents))
            ),
            note: if p.billed_minimum > 1 {
                format!(
                    "For {} to {} users, billed for at least {}.",
                    p.min_users, p.max_users, p.billed_minimum
                )
            } else {
                format!("For {} to {} users.", p.min_users, p.max_users)
            },
            checked: draft.and_then(|d| d.plan) == Some(p.plan),
            fixed: false,
        })
        .collect();
    ChoosePage {
        step: 1,
        action: draft.map_or_else(|| "/book".to_string(), |d| format!("/book/{}/choose", d.id)),
        users: users.to_string(),
        plans,
        error,
        code: code.to_string(),
        code_error,
    }
}

/// GET /book: where a booking starts. The sales app's page while it
/// serves; this plain one otherwise.
fn start(b: &Booking, cx: &Context, _: &KitRequest) -> anyhow::Result<Reply> {
    match b.app_page(&|app| cx.serves(app), Slot::Choose) {
        Some(path) => Ok(Reply::redirect(path)),
        None => show(
            "Choose your plan",
            &choose_page(None, "", String::new(), "", String::new()),
        ),
    }
}

fn choose(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, cx, req);
    let users = if draft.users > 0 {
        draft.users.to_string()
    } else {
        String::new()
    };
    show(
        "Choose your plan",
        &choose_page(
            Some(&draft),
            &users,
            String::new(),
            draft.offer.as_deref().unwrap_or(""),
            String::new(),
        ),
    )
}

/// The plan and the team size, or an offer's code: POST /book starts a
/// draft with it, POST /book/:draft/choose changes the draft.
fn choose_post(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let before = match req.param("draft") {
        Some(_) => Some(open_or_answer!(b, cx, req)),
        None => None,
    };
    let code = req.form_value("code").unwrap_or_default().trim();
    let users = req.form_value("users").unwrap_or_default().trim();
    let id = match &before {
        Some(draft) => draft.id.clone(),
        None => b.store.start()?.0,
    };
    let chosen = if !code.is_empty() {
        b.with_offer(&id, code)?
    } else {
        match (
            users.parse::<u32>(),
            req.form_value("plan").and_then(plan_named),
        ) {
            (Err(_), _) => Err("How many people are on your team?".into()),
            (_, None) => Err("Choose a plan".into()),
            (Ok(users), Some(plan)) => b.with_plan(&id, plan, users, "")?,
        }
    };
    match chosen {
        Ok(_) => Ok(Reply::redirect(&b.next(
            &|app| cx.serves(app),
            &id,
            Slot::Choose,
        ))),
        Err(why) => {
            let draft = b.get(&id)?;
            let (error, code_error) = if code.is_empty() {
                (why, String::new())
            } else {
                (String::new(), why)
            };
            Ok(show(
                "Choose your plan",
                &choose_page(draft.as_ref(), users, error, code, code_error),
            )?
            .status(422))
        }
    }
}

// Configure ----------------------------------------------------------------

#[derive(Template)]
#[template(path = "book/configure.html")]
struct ConfigurePage {
    step: u8,
    action: String,
    package: String,
    users: u32,
    add_ons: Vec<Opt>,
    templates: Vec<Opt>,
    error: String,
    back: String,
    quote: QuoteView,
}

fn configure_page(b: &Booking, cx: &Context, draft: &Draft, error: String) -> ConfigurePage {
    let catalog = catalog::catalog();
    let add_ons = catalog
        .add_ons
        .iter()
        .map(|a| {
            let included = draft.plan.is_some_and(|p| a.included_in.contains(&p));
            Opt {
                value: a.id.clone(),
                label: a.name.clone(),
                note: if included {
                    "Included in your plan".into()
                } else {
                    format!("{} per user per month", money(i64::from(a.monthly_cents)))
                },
                checked: included || draft.add_ons.contains(&a.id),
                fixed: included,
            }
        })
        .collect();
    let mut templates: Vec<Opt> = catalog
        .templates
        .iter()
        .map(|t| Opt {
            value: t.id.clone(),
            label: t.name.clone(),
            note: t.description.clone(),
            checked: draft.template.as_deref() == Some(t.id.as_str()),
            fixed: false,
        })
        .collect();
    templates.push(Opt {
        value: String::new(),
        label: "Start empty".into(),
        note: "No project yet.".into(),
        checked: draft.template.is_none(),
        fixed: false,
    });
    ConfigurePage {
        step: 2,
        action: format!("/book/{}/configure", draft.id),
        package: draft.package.clone(),
        users: draft.users,
        add_ons,
        templates,
        error,
        back: b.link(&|app| cx.serves(app), &draft.id, Slot::Choose),
        quote: QuoteView::of(draft),
    }
}

fn configure(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, cx, req);
    if draft.plan.is_none() {
        return Ok(Reply::redirect(&b.link(
            &|app| cx.serves(app),
            &draft.id,
            Slot::Choose,
        )));
    }
    show(
        "Your workspace",
        &configure_page(b, cx, &draft, String::new()),
    )
}

fn configure_post(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, cx, req);
    let template = req.form_value("template").filter(|t| !t.is_empty());
    let mut changed = b.with_template(&draft.id, template)?;
    for add_on in catalog::catalog().add_ons {
        if changed.is_err() || draft.plan.is_some_and(|p| add_on.included_in.contains(&p)) {
            continue;
        }
        let on = req
            .form
            .iter()
            .any(|(name, value)| name == "add-on" && *value == add_on.id);
        changed = b.with_add_on(&draft.id, &add_on.id, on)?;
    }
    match changed {
        Ok(_) => Ok(Reply::redirect(&b.next(
            &|app| cx.serves(app),
            &draft.id,
            Slot::Configure,
        ))),
        Err(why) => Ok(show("Your workspace", &configure_page(b, cx, &draft, why))?.status(422)),
    }
}

// Terms --------------------------------------------------------------------

#[derive(Template)]
#[template(path = "book/terms.html")]
struct TermsPage {
    step: u8,
    action: String,
    package: String,
    users: u32,
    offer: String,
    offer_terms: String,
    billing: Vec<Opt>,
    years: Vec<Opt>,
    error: String,
    next: String,
    back: String,
    quote: QuoteView,
}

fn terms_page(b: &Booking, cx: &Context, draft: &Draft, error: String) -> TermsPage {
    let billing = [
        (
            Billing::Yearly,
            "yearly",
            "Yearly",
            "Twelve months for the price of ten.",
        ),
        (
            Billing::Monthly,
            "monthly",
            "Monthly",
            "For a contract of one year.",
        ),
    ]
    .into_iter()
    .map(|(billing, value, label, note)| Opt {
        value: value.into(),
        label: label.into(),
        note: note.into(),
        checked: draft.billing == billing,
        fixed: false,
    })
    .collect();
    let years = (1..=3)
        .map(|y: u8| Opt {
            value: y.to_string(),
            label: if y == 1 {
                "1 year".into()
            } else {
                format!("{y} years")
            },
            note: String::new(),
            checked: draft.years == y,
            fixed: false,
        })
        .collect();
    let serves = |app: &str| cx.serves(app);
    TermsPage {
        step: 3,
        action: format!("/book/{}/terms", draft.id),
        package: draft.package.clone(),
        users: draft.users,
        offer: draft.offer.clone().unwrap_or_default(),
        offer_terms: terms_in_words(draft),
        billing,
        years,
        error,
        next: b.next(&serves, &draft.id, Slot::Terms),
        back: b.link(&serves, &draft.id, Slot::Configure),
        quote: QuoteView::of(draft),
    }
}

fn terms(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, cx, req);
    if draft.plan.is_none() {
        return Ok(Reply::redirect(&b.link(
            &|app| cx.serves(app),
            &draft.id,
            Slot::Choose,
        )));
    }
    show("How you pay", &terms_page(b, cx, &draft, String::new()))
}

fn terms_post(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, cx, req);
    let billing = if req.form_value("billing") == Some("monthly") {
        Billing::Monthly
    } else {
        Billing::Yearly
    };
    let years = req
        .form_value("years")
        .and_then(|y| y.parse().ok())
        .unwrap_or(1);
    match b.with_terms(&draft.id, billing, years, PaymentTerms::Net30)? {
        Ok(_) => Ok(Reply::redirect(&b.next(
            &|app| cx.serves(app),
            &draft.id,
            Slot::Terms,
        ))),
        Err(why) => Ok(show("How you pay", &terms_page(b, cx, &draft, why))?.status(422)),
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

fn review_page(b: &Booking, cx: &Context, draft: &Draft) -> ReviewPage {
    let serves = |app: &str| cx.serves(app);
    let link = |slot| b.link(&serves, &draft.id, slot);
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

fn review(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, cx, req);
    if draft.plan.is_none() {
        return Ok(Reply::redirect(&b.link(
            &|app| cx.serves(app),
            &draft.id,
            Slot::Choose,
        )));
    }
    show("Review and book", &review_page(b, cx, &draft))
}

/// Books, at once, and goes on to set up.
fn review_post(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let draft = open_or_answer!(b, cx, req);
    let field = |name: &str| req.form_value(name).unwrap_or_default().trim().to_string();
    let mut t = review_page(b, cx, &draft);
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
                return Ok(Reply::redirect(&b.next(
                    &|app| cx.serves(app),
                    &draft.id,
                    Slot::Review,
                )));
            }
            // The quote's problems are on the page already.
            Err(problems) => t.quote.problems = problems,
        }
    }
    Ok(show("Review and book", &t)?.status(422))
}

// Set up -------------------------------------------------------------------

#[derive(Template)]
#[template(path = "book/setup.html")]
struct SetupPage {
    step: u8,
    action: String,
    booking: Booked,
    seats: u32,
    full: bool,
    name: String,
    email: String,
    error: String,
}

fn setup_page(booking: Booked) -> SetupPage {
    let seats = booking.quote.billed_users;
    SetupPage {
        step: 5,
        action: format!("/book/{}/setup", booking.id),
        full: booking.members.len() >= seats as usize,
        seats,
        booking,
        name: String::new(),
        email: String::new(),
        error: String::new(),
    }
}

fn setup(b: &Booking, cx: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let id = req.param("draft").unwrap_or_default();
    match b.store.booking(id)? {
        Some(booking) => show("Your workspace is ready", &setup_page(booking)),
        // Not booked yet: back to review, where it's booked.
        None if b.store.draft(id)?.is_some() => Ok(Reply::redirect(&b.link(
            &|app| cx.serves(app),
            id,
            Slot::Review,
        ))),
        None => gone(),
    }
}

fn setup_post(b: &Booking, _: &Context, req: &KitRequest) -> anyhow::Result<Reply> {
    let id = req.param("draft").unwrap_or_default();
    let (name, email) = (
        req.form_value("name").unwrap_or_default(),
        req.form_value("email").unwrap_or_default(),
    );
    match b.add_member(id, name, email)? {
        Ok(_) => Ok(Reply::redirect(&format!("/book/{id}/setup"))),
        Err(why) => match b.store.booking(id)? {
            Some(booking) => {
                let mut t = setup_page(booking);
                (t.name, t.email, t.error) = (name.trim().into(), email.trim().into(), why);
                Ok(show("Your workspace is ready", &t)?.status(422))
            }
            None => gone(),
        },
    }
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
    fn the_plain_pages_render() {
        let b = booking();
        let (id, _) = b.store.start().unwrap();
        let page = choose_page(None, "", String::new(), "", String::new())
            .render()
            .unwrap();
        assert!(
            page.contains(r#"action="/book""#) && page.contains("Team, $29 per user per month"),
            "{page}"
        );
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

        b.book(&id, &customer(), CONSENT).unwrap().unwrap();
        let page = setup_page(b.store.booking(&id).unwrap().unwrap())
            .render()
            .unwrap();
        assert!(page.contains("1 of 120 seats"), "{page}");
        assert!(page.contains("https://app.kitewell.example/join/"));
    }
}
