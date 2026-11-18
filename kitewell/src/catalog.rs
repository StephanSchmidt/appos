//! What Kitewell sells and what it costs: the plans, the add-on, the
//! starter templates, the approved offers, and the quote. Prices are
//! finance's, from the price list on the pricing page: yearly billing
//! costs ten months, Business bills at least 10 users, and the AI planning
//! assistant comes with Business. An offer is a contract: its users pay
//! the yearly list price, whatever the billing, less the discount the
//! discount desk approved.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::types::{
    AddOnInfo, Billing, Catalog, Line, PaymentTerms, Plan, PlanInfo, Quote, TemplateInfo,
};

/// More users than the biggest package: Enterprise, sales talks to them.
pub const ENTERPRISE_FROM: u32 = 251;
pub const AI: &str = "ai-assistant";
/// The most an offer may take off, in basis points: 50%.
pub const MAX_DISCOUNT: u32 = 5000;

pub fn catalog() -> Catalog {
    let features = |list: &[&str]| list.iter().map(|f| f.to_string()).collect();
    Catalog {
        plans: vec![
            PlanInfo {
                plan: Plan::Starter,
                name: "Starter".into(),
                monthly_cents: 1200,
                min_users: 1,
                max_users: 10,
                billed_minimum: 1,
                features: features(&[
                    "Unlimited projects",
                    "Calendar and timeline views",
                    "Slack and Google Calendar integration",
                    "Email support",
                ]),
            },
            PlanInfo {
                plan: Plan::Team,
                name: "Team".into(),
                monthly_cents: 2900,
                min_users: 3,
                max_users: 100,
                billed_minimum: 3,
                features: features(&[
                    "Everything in Starter",
                    "Workload view and capacity planning",
                    "Guest access",
                    "Priority support",
                ]),
            },
            PlanInfo {
                plan: Plan::Business,
                name: "Business".into(),
                monthly_cents: 5900,
                min_users: 1,
                max_users: ENTERPRISE_FROM - 1,
                billed_minimum: 10,
                features: features(&[
                    "Everything in Team",
                    "SSO and SCIM",
                    "Audit log",
                    "AI planning assistant included",
                    "Dedicated success manager",
                ]),
            },
        ],
        add_ons: vec![AddOnInfo {
            id: AI.into(),
            name: "AI planning assistant".into(),
            monthly_cents: 800,
            included_in: vec![Plan::Business],
        }],
        templates: vec![
            template(
                "plan-a-quarter",
                "Plan a quarter",
                "Goals, milestones and a realistic timeline for the next three months.",
            ),
            template(
                "agency-capacity",
                "Agency capacity plan",
                "Clients, projects and people, with the workload view set up for billable hours.",
            ),
            template(
                "product-launch",
                "Product launch",
                "From the first draft to launch day, with every team's part on one timeline.",
            ),
        ],
        enterprise_from: ENTERPRISE_FROM,
    }
}

fn template(id: &str, name: &str, description: &str) -> TemplateInfo {
    TemplateInfo {
        id: id.into(),
        name: name.into(),
        description: description.into(),
    }
}

fn plan_info(plan: Plan) -> PlanInfo {
    catalog()
        .plans
        .into_iter()
        .find(|p| p.plan == plan)
        .expect("every plan is in the catalog")
}

pub fn template_name(id: &str) -> Option<String> {
    catalog()
        .templates
        .into_iter()
        .find(|t| t.id == id)
        .map(|t| t.name)
}

/// An offer the discount desk approved for one customer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub code: String,
    pub company: String,
    pub package: String,
    pub plan: Plan,
    pub users: u32,
    pub years: u8,
    pub billing: Billing,
    pub payment_terms: PaymentTerms,
    /// In basis points: 1250 is 12.5%.
    pub discount: u32,
    pub approved_by: String,
    /// The last day it can be booked, like "2026-12-31".
    pub valid_until: String,
}

/// The offers a fresh platform starts with, as the discount desk approved
/// them.
pub fn seed_offers() -> Vec<Offer> {
    vec![
        Offer {
            code: "KW-ACME-2026".into(),
            company: "Acme Analytics".into(),
            package: "Business".into(),
            plan: Plan::Business,
            users: 120,
            years: 2,
            billing: Billing::Yearly,
            payment_terms: PaymentTerms::Net30,
            discount: 1800,
            approved_by: "Daniel Reyes, Marcus Hale".into(),
            valid_until: "2026-12-31".into(),
        },
        Offer {
            code: "KW-NORDLICHT-2026".into(),
            company: "Nordlicht Media".into(),
            package: "Agency package".into(),
            plan: Plan::Business,
            users: 40,
            years: 1,
            billing: Billing::Quarterly,
            payment_terms: PaymentTerms::Net45,
            discount: 1250,
            approved_by: "Grace Kim, Elena Petrova".into(),
            valid_until: "2027-01-31".into(),
        },
        Offer {
            code: "KW-POLAR-2025".into(),
            company: "Polar Foods".into(),
            package: "Team".into(),
            plan: Plan::Team,
            users: 30,
            years: 1,
            billing: Billing::Yearly,
            payment_terms: PaymentTerms::Net30,
            discount: 800,
            approved_by: "Daniel Reyes".into(),
            valid_until: "2025-12-31".into(),
        },
    ]
}

impl Offer {
    /// Why it can't be booked on `today` ("2026-11-18"), if it can't.
    pub fn expired(&self, today: &str) -> Option<String> {
        (today > self.valid_until.as_str())
            .then(|| format!("This offer expired on {}", long_date(&self.valid_until)))
    }
}

/// "2026-12-31" as "31 December 2026".
pub fn long_date(day: &str) -> String {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_or_else(|_| day.to_string(), |d| d.format("%-d %B %Y").to_string())
}

/// What a draft says so far, as the platform keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub package: String,
    pub plan: Option<Plan>,
    pub users: u32,
    pub add_ons: Vec<String>,
    pub template: Option<String>,
    pub billing: Billing,
    pub years: u8,
    pub payment_terms: PaymentTerms,
    pub offer: Option<String>,
}

impl Default for State {
    fn default() -> State {
        State {
            package: String::new(),
            plan: None,
            users: 0,
            add_ons: Vec::new(),
            template: None,
            billing: Billing::Yearly,
            years: 1,
            payment_terms: PaymentTerms::Net30,
            offer: None,
        }
    }
}

/// Whether `users` may book `plan`; the reason in words if not.
pub fn check_plan(plan: Plan, users: u32) -> Result<(), String> {
    let info = plan_info(plan);
    if users == 0 {
        Err("How many people are on your team?".into())
    } else if users >= ENTERPRISE_FROM {
        Err(format!(
            "More than {} users: talk to sales",
            ENTERPRISE_FROM - 1
        ))
    } else if users > info.max_users {
        Err(format!(
            "{} is for up to {} users",
            info.name, info.max_users
        ))
    } else if users < info.min_users {
        Err(format!("{} starts at {} users", info.name, info.min_users))
    } else {
        Ok(())
    }
}

/// Whether the terms are allowed: an offer's own, or without one, Net 30
/// and yearly billing, or monthly for one year.
pub fn check_terms(
    offer: Option<&Offer>,
    billing: Billing,
    years: u8,
    payment_terms: PaymentTerms,
) -> Result<(), String> {
    if let Some(offer) = offer {
        return if (billing, years, payment_terms)
            == (offer.billing, offer.years, offer.payment_terms)
        {
            Ok(())
        } else {
            Err(format!("These terms come with your offer {}", offer.code))
        };
    }
    if !(1..=3).contains(&years) {
        Err("A contract runs 1, 2 or 3 years".into())
    } else if billing == Billing::Quarterly {
        Err("Quarterly billing comes only with an offer".into())
    } else if billing == Billing::Monthly && years > 1 {
        Err("Monthly billing is for 1 year only".into())
    } else if payment_terms != PaymentTerms::Net30 {
        Err("Payment is Net 30 without an offer".into())
    } else {
        Ok(())
    }
}

/// The quote for a draft on `today`, with its offer if it has one (none
/// for an offer that no longer exists: that's a problem then).
pub fn quote(state: &State, offer: Option<&Offer>, today: &str) -> Quote {
    let mut problems = Vec::new();
    let Some(plan) = state.plan else {
        return Quote {
            problems: vec!["Choose a package".into()],
            ..Quote::default()
        };
    };
    if let Err(why) = check_plan(plan, state.users) {
        problems.push(why);
    }
    match (&state.offer, offer) {
        (Some(code), None) => problems.push(format!("There is no offer {code} any more")),
        (_, Some(offer)) => problems.extend(offer.expired(today)),
        (None, None) => {}
    }
    if let Err(why) = check_terms(offer, state.billing, state.years, state.payment_terms) {
        problems.push(why);
    }
    let info = plan_info(plan);
    let billed = state.users.max(info.billed_minimum);
    // A contract (an offer) pays the yearly list price; without one,
    // monthly billing pays twelve months, yearly ten.
    let months = if offer.is_none() && state.billing == Billing::Monthly {
        12
    } else {
        10
    };
    let per_user_year = |monthly: u32| i64::from(monthly) * months;
    let billing = match state.billing {
        Billing::Monthly => "monthly",
        Billing::Quarterly => "quarterly",
        Billing::Yearly => "yearly",
    };
    let users = if billed > state.users {
        format!("billed for {billed} users (the minimum)")
    } else {
        format!("{billed} users")
    };
    let mut lines = vec![Line {
        label: format!("{}, {users}, {billing}", info.name),
        cents: per_user_year(info.monthly_cents) * i64::from(billed),
    }];
    for add_on in catalog()
        .add_ons
        .iter()
        .filter(|a| state.add_ons.contains(&a.id) || a.included_in.contains(&plan))
    {
        if add_on.included_in.contains(&plan) {
            lines.push(Line {
                label: format!("{}, included", add_on.name),
                cents: 0,
            });
        } else {
            lines.push(Line {
                label: format!("{}, {billed} users", add_on.name),
                cents: per_user_year(add_on.monthly_cents) * i64::from(billed),
            });
        }
    }
    if let Some(offer) = offer {
        let list: i64 = lines.iter().map(|l| l.cents).sum();
        let discount = offer.discount.min(MAX_DISCOUNT);
        lines.push(Line {
            label: format!("Offer {}, {} off", offer.code, percent(discount)),
            cents: -((list * i64::from(discount) + 5000) / 10_000),
        });
    }
    let per_year = u64::try_from(lines.iter().map(|l| l.cents).sum::<i64>()).unwrap_or(0);
    let invoices = match state.billing {
        Billing::Monthly => 12,
        Billing::Quarterly => 4,
        Billing::Yearly => 1,
    };
    Quote {
        lines,
        billed_users: billed,
        per_year_cents: per_year,
        contract_cents: per_year * u64::from(state.years),
        per_invoice_cents: (per_year + invoices / 2) / invoices,
        problems,
    }
}

/// 1800 as "18%", 1250 as "12.5%".
fn percent(basis_points: u32) -> String {
    if basis_points.is_multiple_of(100) {
        format!("{}%", basis_points / 100)
    } else {
        format!("{}%", f64::from(basis_points) / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(code: &str) -> Offer {
        seed_offers().into_iter().find(|o| o.code == code).unwrap()
    }

    fn state(plan: Plan, users: u32, billing: Billing) -> State {
        State {
            plan: Some(plan),
            users,
            billing,
            ..State::default()
        }
    }

    #[test]
    fn the_discount_desks_example() {
        // 120 seats of Business, 2 years, 18% off: list $70,800, discount
        // $12,744 a year, net ARR $58,056, TCV $116,112.
        let acme = offer("KW-ACME-2026");
        let s = State {
            offer: Some(acme.code.clone()),
            years: 2,
            ..state(Plan::Business, 120, Billing::Yearly)
        };
        let q = quote(&s, Some(&acme), "2026-11-18");
        assert_eq!(q.problems, Vec::<String>::new());
        assert_eq!(q.lines[0].cents, 7_080_000);
        assert_eq!(q.lines.last().unwrap().cents, -1_274_400);
        assert_eq!(
            (q.per_year_cents, q.contract_cents),
            (5_805_600, 11_611_200)
        );
    }

    #[test]
    fn yearly_is_ten_months_and_business_bills_ten() {
        // The pricing page: Team, 25 users, yearly is $7,250 a year.
        let q = quote(&state(Plan::Team, 25, Billing::Yearly), None, "2026-11-18");
        assert_eq!(q.per_year_cents, 725_000);
        let q = quote(&state(Plan::Team, 25, Billing::Monthly), None, "2026-11-18");
        assert_eq!((q.per_year_cents, q.per_invoice_cents), (870_000, 72_500));
        let q = quote(
            &state(Plan::Business, 6, Billing::Yearly),
            None,
            "2026-11-18",
        );
        assert_eq!(q.billed_users, 10);
        assert_eq!(
            q.lines[0].label,
            "Business, billed for 10 users (the minimum), yearly"
        );
        assert_eq!(
            q.lines[1],
            Line {
                label: "AI planning assistant, included".into(),
                cents: 0
            }
        );
    }

    #[test]
    fn the_ai_assistant_costs_eight_dollars_outside_business() {
        let s = State {
            add_ons: vec![AI.into()],
            ..state(Plan::Starter, 4, Billing::Yearly)
        };
        let q = quote(&s, None, "2026-11-18");
        assert_eq!(q.lines[1].cents, 800 * 10 * 4);
        assert_eq!(q.per_year_cents, (1200 + 800) * 10 * 4);
    }

    #[test]
    fn what_cant_be_booked_says_why() {
        assert_eq!(
            check_plan(Plan::Starter, 11),
            Err("Starter is for up to 10 users".into())
        );
        assert_eq!(
            check_plan(Plan::Team, 2),
            Err("Team starts at 3 users".into())
        );
        assert_eq!(
            check_plan(Plan::Business, 251),
            Err("More than 250 users: talk to sales".into())
        );
        assert_eq!(
            check_terms(None, Billing::Monthly, 2, PaymentTerms::Net30),
            Err("Monthly billing is for 1 year only".into())
        );
        assert_eq!(
            check_terms(None, Billing::Yearly, 1, PaymentTerms::Net60),
            Err("Payment is Net 30 without an offer".into())
        );
        let nordlicht = offer("KW-NORDLICHT-2026");
        assert!(check_terms(Some(&nordlicht), Billing::Quarterly, 1, PaymentTerms::Net45).is_ok());
        assert_eq!(
            check_terms(Some(&nordlicht), Billing::Yearly, 1, PaymentTerms::Net45),
            Err("These terms come with your offer KW-NORDLICHT-2026".into())
        );
        let polar = offer("KW-POLAR-2025");
        assert_eq!(
            polar.expired("2026-11-18"),
            Some("This offer expired on 31 December 2025".into())
        );
        assert_eq!(
            quote(&State::default(), None, "2026-11-18").problems,
            ["Choose a package"]
        );
    }

    #[test]
    fn an_offer_pays_the_yearly_list_price_in_quarters() {
        let nordlicht = offer("KW-NORDLICHT-2026");
        let s = State {
            offer: Some(nordlicht.code.clone()),
            billing: Billing::Quarterly,
            payment_terms: PaymentTerms::Net45,
            ..state(Plan::Business, 40, Billing::Quarterly)
        };
        let q = quote(&s, Some(&nordlicht), "2026-11-18");
        assert_eq!(q.problems, Vec::<String>::new());
        // 40 × $590 = $23,600, 12.5% off = $20,650 a year, $5,162.50 a quarter.
        assert_eq!(
            (q.per_year_cents, q.per_invoice_cents),
            (2_065_000, 516_250)
        );
        assert_eq!(
            q.lines.last().unwrap().label,
            "Offer KW-NORDLICHT-2026, 12.5% off"
        );
    }
}
