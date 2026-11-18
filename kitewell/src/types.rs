//! The records and enums of wit/booking.wit, as the kit passes them to
//! apps. Names are WIT's: kebab-case, renamed field by field.

use appos_shell::kit::{ComponentType, Lift, Lower};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ComponentType, Lift, Lower)]
#[component(enum)]
#[repr(u8)]
#[serde(rename_all = "kebab-case")]
pub enum Plan {
    #[component(name = "starter")]
    Starter,
    #[component(name = "team")]
    Team,
    #[component(name = "business")]
    Business,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ComponentType, Lift, Lower)]
#[component(enum)]
#[repr(u8)]
#[serde(rename_all = "kebab-case")]
pub enum Billing {
    #[component(name = "monthly")]
    Monthly,
    #[component(name = "quarterly")]
    Quarterly,
    #[component(name = "yearly")]
    Yearly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ComponentType, Lift, Lower)]
#[component(enum)]
#[repr(u8)]
#[serde(rename_all = "kebab-case")]
pub enum PaymentTerms {
    #[component(name = "net30")]
    Net30,
    #[component(name = "net45")]
    Net45,
    #[component(name = "net60")]
    Net60,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ComponentType, Lift, Lower)]
#[component(enum)]
#[repr(u8)]
pub enum Slot {
    #[component(name = "choose")]
    Choose,
    #[component(name = "configure")]
    Configure,
    #[component(name = "terms")]
    Terms,
    #[component(name = "review")]
    Review,
    #[component(name = "setup")]
    Setup,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct PlanInfo {
    pub plan: Plan,
    pub name: String,
    #[component(name = "monthly-cents")]
    pub monthly_cents: u32,
    #[component(name = "min-users")]
    pub min_users: u32,
    #[component(name = "max-users")]
    pub max_users: u32,
    #[component(name = "billed-minimum")]
    pub billed_minimum: u32,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct AddOnInfo {
    pub id: String,
    pub name: String,
    #[component(name = "monthly-cents")]
    pub monthly_cents: u32,
    #[component(name = "included-in")]
    pub included_in: Vec<Plan>,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct Catalog {
    pub plans: Vec<PlanInfo>,
    #[component(name = "add-ons")]
    pub add_ons: Vec<AddOnInfo>,
    pub templates: Vec<TemplateInfo>,
    #[component(name = "enterprise-from")]
    pub enterprise_from: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ComponentType, Lift, Lower)]
#[component(record)]
pub struct Line {
    pub label: String,
    pub cents: i64,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ComponentType, Lift, Lower,
)]
#[component(record)]
pub struct Quote {
    pub lines: Vec<Line>,
    #[component(name = "billed-users")]
    pub billed_users: u32,
    #[component(name = "per-year-cents")]
    pub per_year_cents: u64,
    #[component(name = "contract-cents")]
    pub contract_cents: u64,
    #[component(name = "per-invoice-cents")]
    pub per_invoice_cents: u64,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct Draft {
    pub id: String,
    pub package: String,
    pub plan: Option<Plan>,
    pub users: u32,
    #[component(name = "add-ons")]
    pub add_ons: Vec<String>,
    pub template: Option<String>,
    pub billing: Billing,
    pub years: u8,
    #[component(name = "payment-terms")]
    pub payment_terms: PaymentTerms,
    pub offer: Option<String>,
    pub quote: Quote,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct Member {
    pub name: String,
    pub email: String,
    pub invite: String,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct Booking {
    pub id: String,
    pub company: String,
    pub package: String,
    pub users: u32,
    pub workspace: String,
    pub members: Vec<Member>,
    pub project: Option<String>,
    pub quote: Quote,
}

/// Who books: what the review page asks. Not in the WIT; only the
/// platform's own page books.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Customer {
    pub company: String,
    pub name: String,
    pub email: String,
}
