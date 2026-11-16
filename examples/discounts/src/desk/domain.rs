//! The desk's data and its rules: the terms a rep asks for, the numbers,
//! the approval policy, and how a request moves from submitted to won.
//! Plain Rust, no IO.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::money::dollars;
use super::people::{self, Person, Rep};
use super::time;
use crate::appos::time::Date;

// ---- The policy ---------------------------------------------------------

/// Up to 10%: approved automatically.
pub const AUTO_UP_TO: Discount = Discount(100);
/// Over 10%: the rep's manager.
pub const MANAGER_ABOVE: Discount = Discount(100);
/// Over 20%: also the VP.
pub const VP_ABOVE: Discount = Discount(200);
/// Over 30%: also the CFO.
pub const CFO_ABOVE: Discount = Discount(300);
/// Over 50%: not possible.
pub const MAX_DISCOUNT: Discount = Discount(500);
/// A deal over this net ARR (in cents) needs the VP, whatever the discount.
pub const VP_ABOVE_NET_ARR: u64 = 100_000 * 100;
/// An approval is valid this long; then it expires by itself.
pub const VALID_DAYS: i64 = 30;
/// Waiting longer than this stands out.
pub const OVERDUE_AFTER: i64 = 24 * time::HOUR;

// ---- What a rep can type ------------------------------------------------

pub const CUSTOMER_MAX: usize = 120;
pub const LINK_MAX: usize = 300;
pub const SEATS_MAX: u32 = 100_000;
pub const COMPETITOR_MAX: usize = 80;
pub const JUSTIFICATION_MIN_WORDS: usize = 8;
pub const JUSTIFICATION_MAX: usize = 2000;
pub const COMMENT_MAX: usize = 1000;
/// How far ahead a close date may be.
pub const CLOSE_WITHIN_DAYS: i64 = 3 * 366;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    #[default]
    Team,
    Business,
}

impl Plan {
    pub const ALL: [Plan; 2] = [Plan::Team, Plan::Business];

    /// The list price per user per year, in cents.
    pub fn price(self) -> u64 {
        match self {
            Plan::Team => 290 * 100,
            Plan::Business => 590 * 100,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Plan::Team => "team",
            Plan::Business => "business",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Plan::Team => "Team",
            Plan::Business => "Business",
        }
    }

    pub fn parse(text: &str) -> Option<Plan> {
        Plan::ALL.into_iter().find(|p| p.key() == text)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Billing {
    #[default]
    Annual,
    Quarterly,
    Monthly,
}

impl Billing {
    pub const ALL: [Billing; 3] = [Billing::Annual, Billing::Quarterly, Billing::Monthly];

    pub fn key(self) -> &'static str {
        match self {
            Billing::Annual => "annual",
            Billing::Quarterly => "quarterly",
            Billing::Monthly => "monthly",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Billing::Annual => "Annually upfront",
            Billing::Quarterly => "Quarterly",
            Billing::Monthly => "Monthly",
        }
    }

    pub fn parse(text: &str) -> Option<Billing> {
        Billing::ALL.into_iter().find(|b| b.key() == text)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Payment {
    #[default]
    Net30,
    Net45,
    Net60,
}

impl Payment {
    pub const ALL: [Payment; 3] = [Payment::Net30, Payment::Net45, Payment::Net60];

    pub fn key(self) -> &'static str {
        match self {
            Payment::Net30 => "net30",
            Payment::Net45 => "net45",
            Payment::Net60 => "net60",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Payment::Net30 => "Net 30",
            Payment::Net45 => "Net 45",
            Payment::Net60 => "Net 60",
        }
    }

    pub fn parse(text: &str) -> Option<Payment> {
        Payment::ALL.into_iter().find(|p| p.key() == text)
    }
}

/// A discount in tenths of a percent: 125 is 12.5%. Only whole numbers and
/// halves, 0 to 50%: `Discount::parse` makes no other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Discount(u32);

impl Discount {
    pub fn tenths(self) -> u32 {
        self.0
    }

    /// "12.5", "18": what goes back into the form.
    pub fn number(self) -> String {
        match self.0 % 10 {
            0 => format!("{}", self.0 / 10),
            r => format!("{}.{r}", self.0 / 10),
        }
    }

    /// Parses "12", "12.5", "12,5", "12.5 %".
    pub fn parse(text: &str) -> Result<Discount, FieldError> {
        let text = text.trim().trim_end_matches('%').trim().replace(',', ".");
        if text.is_empty() {
            return Err(FieldError::DiscountMissing);
        }
        let (whole, frac) = text.split_once('.').unwrap_or((&text, ""));
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if !digits(whole)
            || !digits(frac)
            || whole.len() > 3
            || frac.len() > 2
            || (whole.is_empty() && frac.is_empty())
        {
            return Err(FieldError::DiscountInvalid);
        }
        let whole: u32 = if whole.is_empty() {
            0
        } else {
            whole.parse().map_err(|_| FieldError::DiscountInvalid)?
        };
        let frac: u32 = format!("{frac:0<2}")
            .parse()
            .map_err(|_| FieldError::DiscountInvalid)?;
        let hundredths = whole * 100 + frac;
        if !hundredths.is_multiple_of(50) {
            return Err(FieldError::DiscountNotHalf);
        }
        let discount = Discount(hundredths / 10);
        if discount > MAX_DISCOUNT {
            return Err(FieldError::DiscountTooHigh);
        }
        Ok(discount)
    }
}

impl fmt::Display for Discount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.number())
    }
}

/// What a rep asks for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terms {
    pub customer: String,
    /// The Salesforce opportunity; empty when there is none.
    pub link: String,
    pub plan: Plan,
    pub seats: u32,
    pub years: u32,
    pub discount: Discount,
    pub billing: Billing,
    pub payment: Payment,
    /// Empty when there is none.
    pub competitor: String,
    /// "2026-12-15".
    pub close_date: String,
    pub justification: String,
}

/// The numbers of a deal, in cents; per year unless the name says otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Numbers {
    pub list: u64,
    pub discount: u64,
    pub net_arr: u64,
    /// Total contract value: net ARR times years.
    pub tcv: u64,
    /// The discount over the whole contract.
    pub discount_total: u64,
}

impl Numbers {
    pub fn of(plan: Plan, seats: u32, years: u32, discount: Discount) -> Numbers {
        let list = plan.price() * u64::from(seats);
        // Rounded to the cent; with today's prices it is always exact.
        let off = (list * u64::from(discount.tenths()) + 500) / 1000;
        let net_arr = list - off;
        Numbers {
            list,
            discount: off,
            net_arr,
            tcv: net_arr * u64::from(years),
            discount_total: off * u64::from(years),
        }
    }
}

impl Terms {
    pub fn numbers(&self) -> Numbers {
        Numbers::of(self.plan, self.seats, self.years, self.discount)
    }

    /// "Business, 120 seats, 2 years, 18% off, annually upfront, Net 30".
    pub fn summary(&self) -> String {
        format!(
            "{}, {} seats, {}, {} off, {} billing, {}",
            self.plan.label(),
            super::money::group(u64::from(self.seats)),
            years(self.years),
            self.discount,
            self.billing.label().to_lowercase(),
            self.payment.label()
        )
    }
}

/// "1 year", "3 years".
pub fn years(n: u32) -> String {
    if n == 1 {
        "1 year".into()
    } else {
        format!("{n} years")
    }
}

// ---- Parsing the form ---------------------------------------------------

/// The form's text, as typed. `Terms::parse` turns it into `Terms` or says,
/// per field, what's wrong.
#[derive(Clone, Debug, Default)]
pub struct TermsInput {
    pub customer: String,
    pub link: String,
    pub plan: String,
    pub seats: String,
    pub years: String,
    pub discount: String,
    pub billing: String,
    pub payment: String,
    pub competitor: String,
    pub close_date: String,
    pub justification: String,
}

impl TermsInput {
    /// The form filled in with existing terms, to change them.
    pub fn of(terms: &Terms) -> TermsInput {
        TermsInput {
            customer: terms.customer.clone(),
            link: terms.link.clone(),
            plan: terms.plan.key().into(),
            seats: terms.seats.to_string(),
            years: terms.years.to_string(),
            discount: terms.discount.number(),
            billing: terms.billing.key().into(),
            payment: terms.payment.key().into(),
            competitor: terms.competitor.clone(),
            close_date: terms.close_date.clone(),
            justification: terms.justification.clone(),
        }
    }

    /// A new form: standard billing and terms preselected.
    pub fn blank() -> TermsInput {
        TermsInput {
            years: "1".into(),
            billing: Billing::Annual.key().into(),
            payment: Payment::Net30.key().into(),
            ..TermsInput::default()
        }
    }
}

/// Why a field doesn't parse. The text is for the rep.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum FieldError {
    #[error("Enter the customer's name.")]
    CustomerMissing,
    #[error("Keep the customer's name to {CUSTOMER_MAX} characters.")]
    CustomerTooLong,
    #[error("Paste the whole link to the opportunity: https://… on salesforce.com or force.com.")]
    LinkInvalid,
    #[error("Pick Team or Business. Starter is never discounted.")]
    PlanMissing,
    #[error("Enter the seats as a whole number, 1 to 100,000.")]
    SeatsInvalid,
    #[error("Pick 1, 2 or 3 years.")]
    YearsInvalid,
    #[error("Enter the discount in percent.")]
    DiscountMissing,
    #[error("Enter the discount as a number, like 12 or 12.5.")]
    DiscountInvalid,
    #[error("Use whole numbers or halves, like 12 or 12.5.")]
    DiscountNotHalf,
    #[error("More than 50% is not possible.")]
    DiscountTooHigh,
    #[error("Pick how the customer is billed.")]
    BillingMissing,
    #[error("Pick the payment terms.")]
    PaymentMissing,
    #[error("Keep the competitor to {COMPETITOR_MAX} characters.")]
    CompetitorTooLong,
    #[error("Pick the expected close date.")]
    CloseDateMissing,
    #[error("Enter the date as YYYY-MM-DD, like 2026-12-15.")]
    CloseDateInvalid,
    #[error("That date has passed. When do you expect to close now?")]
    CloseDatePast,
    #[error("That's more than three years away.")]
    CloseDateTooFar,
    #[error(
        "Write at least one real sentence ({JUSTIFICATION_MIN_WORDS} words or more): why does this deal need this discount?"
    )]
    JustificationTooShort,
    #[error("Keep the justification to {JUSTIFICATION_MAX} characters.")]
    JustificationTooLong,
}

/// The fields that didn't parse, by the form field's name.
pub type FormErrors = crate::appos::FormErrors<FieldError>;

impl Terms {
    /// Parses the whole form; `today` is "2026-11-13", for the close date.
    pub fn parse(input: &TermsInput, today: &str) -> Result<Terms, FormErrors> {
        let mut e = FormErrors::default();
        let customer = e.keep("customer", parse_customer(&input.customer));
        let link = e.keep("link", parse_link(&input.link));
        let plan = e.keep(
            "plan",
            Plan::parse(input.plan.trim()).ok_or(FieldError::PlanMissing),
        );
        let seats = e.keep("seats", parse_seats(&input.seats));
        let years = e.keep("years", parse_years(&input.years));
        let discount = e.keep("discount", Discount::parse(&input.discount));
        let billing = e.keep(
            "billing",
            Billing::parse(input.billing.trim()).ok_or(FieldError::BillingMissing),
        );
        let payment = e.keep(
            "payment",
            Payment::parse(input.payment.trim()).ok_or(FieldError::PaymentMissing),
        );
        let competitor = e.keep("competitor", parse_competitor(&input.competitor));
        let close_date = e.keep("close_date", parse_close_date(&input.close_date, today));
        let justification = e.keep("justification", parse_justification(&input.justification));
        match (
            customer,
            link,
            plan,
            seats,
            years,
            discount,
            billing,
            payment,
            competitor,
            close_date,
            justification,
        ) {
            (
                Some(customer),
                Some(link),
                Some(plan),
                Some(seats),
                Some(years),
                Some(discount),
                Some(billing),
                Some(payment),
                Some(competitor),
                Some(close_date),
                Some(justification),
            ) => Ok(Terms {
                customer,
                link,
                plan,
                seats,
                years,
                discount,
                billing,
                payment,
                competitor,
                close_date,
                justification,
            }),
            _ => Err(e),
        }
    }
}

/// Collapses runs of whitespace, so names and lines stay tidy.
fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_customer(text: &str) -> Result<String, FieldError> {
    let name = tidy(text);
    match name.chars().count() {
        0 => Err(FieldError::CustomerMissing),
        n if n > CUSTOMER_MAX => Err(FieldError::CustomerTooLong),
        _ => Ok(name),
    }
}

/// Empty, or an https link to Salesforce.
fn parse_link(text: &str) -> Result<String, FieldError> {
    let link = text.trim();
    if link.is_empty() {
        return Ok(String::new());
    }
    let rest = link
        .strip_prefix("https://")
        .ok_or(FieldError::LinkInvalid)?;
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let salesforce = host.ends_with(".salesforce.com") || host.ends_with(".force.com");
    let clean =
        link.len() <= LINK_MAX && !link.chars().any(|c| c.is_whitespace() || c.is_control());
    let plain_host = host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if salesforce && clean && plain_host {
        Ok(link.to_string())
    } else {
        Err(FieldError::LinkInvalid)
    }
}

fn parse_seats(text: &str) -> Result<u32, FieldError> {
    let text = text.trim();
    if let Some(separator) = text.chars().find(|c| matches!(c, ',' | ' ' | '.' | '\'')) {
        let mut groups = text.split(separator);
        let first = groups.next().unwrap_or("");
        if first.is_empty()
            || first.len() > 3
            || !first.bytes().all(|b| b.is_ascii_digit())
            || !groups.all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(FieldError::SeatsInvalid);
        }
    }
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    if !text
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ',' | ' ' | '.' | '\''))
    {
        return Err(FieldError::SeatsInvalid);
    }
    if digits.is_empty() || digits.len() > 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(FieldError::SeatsInvalid);
    }
    match digits.parse::<u32>() {
        Ok(n) if (1..=SEATS_MAX).contains(&n) => Ok(n),
        _ => Err(FieldError::SeatsInvalid),
    }
}

fn parse_years(text: &str) -> Result<u32, FieldError> {
    match text.trim() {
        "1" => Ok(1),
        "2" => Ok(2),
        "3" => Ok(3),
        _ => Err(FieldError::YearsInvalid),
    }
}

fn parse_competitor(text: &str) -> Result<String, FieldError> {
    let name = tidy(text);
    if name.chars().count() > COMPETITOR_MAX {
        return Err(FieldError::CompetitorTooLong);
    }
    Ok(name)
}

fn parse_close_date(text: &str, today: &str) -> Result<String, FieldError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(FieldError::CloseDateMissing);
    }
    let day = Date::parse(text)
        .ok_or(FieldError::CloseDateInvalid)?
        .days();
    let today = Date::parse(today).map_or(day, Date::days);
    if day < today {
        return Err(FieldError::CloseDatePast);
    }
    if day > today + CLOSE_WITHIN_DAYS {
        return Err(FieldError::CloseDateTooFar);
    }
    Ok(text.to_string())
}

fn parse_justification(text: &str) -> Result<String, FieldError> {
    let text = text.trim();
    if text.chars().count() > JUSTIFICATION_MAX {
        return Err(FieldError::JustificationTooLong);
    }
    if text.split_whitespace().count() < JUSTIFICATION_MIN_WORDS {
        return Err(FieldError::JustificationTooShort);
    }
    Ok(text.to_string())
}

/// What the form shows while the rep types: the numbers once plan, seats,
/// years and discount parse, and who approves once billing and payment
/// terms do too.
#[derive(Debug, Default)]
pub struct Preview {
    pub numbers: Option<Numbers>,
    pub steps: Option<Vec<Step>>,
    /// Why the discount doesn't parse, once something is typed.
    pub discount_error: Option<FieldError>,
}

pub fn preview(rep: Rep, input: &TermsInput) -> Preview {
    let plan = Plan::parse(input.plan.trim());
    let seats = parse_seats(&input.seats).ok();
    let years = parse_years(&input.years).ok();
    let discount = Discount::parse(&input.discount);
    let billing = Billing::parse(input.billing.trim());
    let payment = Payment::parse(input.payment.trim());
    let (numbers, steps) = match (plan, seats, years, discount.clone().ok()) {
        (Some(plan), Some(seats), Some(years), Some(discount)) => {
            let steps = match (billing, payment) {
                (Some(billing), Some(payment)) => {
                    let terms = Terms {
                        plan,
                        seats,
                        years,
                        discount,
                        billing,
                        payment,
                        ..Terms::default()
                    };
                    Some(route(rep, &terms))
                }
                _ => None,
            };
            (Some(Numbers::of(plan, seats, years, discount)), steps)
        }
        _ => (None, None),
    };
    Preview {
        numbers,
        steps,
        discount_error: discount.err().filter(|e| *e != FieldError::DiscountMissing),
    }
}

// ---- Who approves -------------------------------------------------------

/// The approvers, in the order they approve.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approver {
    #[default]
    Manager,
    Vp,
    Cfo,
}

impl Approver {
    pub const ORDER: [Approver; 3] = [Approver::Manager, Approver::Vp, Approver::Cfo];

    pub fn label(self) -> &'static str {
        match self {
            Approver::Manager => "Manager",
            Approver::Vp => "VP",
            Approver::Cfo => "CFO",
        }
    }

    fn person(self, rep: Rep) -> &'static Person {
        match self {
            Approver::Manager => rep.manager,
            Approver::Vp => people::vp(),
            Approver::Cfo => people::cfo(),
        }
    }
}

/// One approval a request needs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub approver: Approver,
    /// The person's id.
    pub person: String,
    /// Why the policy needs this approver: "discount over 20%".
    pub reasons: Vec<String>,
    /// When they approved in this round; empty until then.
    pub approved_at: String,
}

/// The approvals the policy needs for these terms, in order: manager, VP,
/// CFO, each at most once, whatever number of rules need them.
pub fn route(rep: Rep, terms: &Terms) -> Vec<Step> {
    let d = terms.discount;
    let numbers = terms.numbers();
    let mut needs: Vec<(Approver, String)> = vec![];
    if d > MANAGER_ABOVE {
        needs.push((Approver::Manager, format!("discount over {MANAGER_ABOVE}")));
    }
    if d > VP_ABOVE {
        needs.push((Approver::Vp, format!("discount over {VP_ABOVE}")));
    }
    if numbers.net_arr > VP_ABOVE_NET_ARR {
        needs.push((
            Approver::Vp,
            format!("net ARR over {}", dollars(VP_ABOVE_NET_ARR)),
        ));
    }
    if d > CFO_ABOVE {
        needs.push((Approver::Cfo, format!("discount over {CFO_ABOVE}")));
    }
    if terms.billing != Billing::Annual {
        needs.push((
            Approver::Cfo,
            format!("{} billing", terms.billing.label().to_lowercase()),
        ));
    }
    if terms.payment != Payment::Net30 {
        needs.push((
            Approver::Cfo,
            format!("{} payment terms", terms.payment.label()),
        ));
    }
    Approver::ORDER
        .into_iter()
        .filter_map(|approver| {
            let reasons: Vec<String> = needs
                .iter()
                .filter(|(a, _)| *a == approver)
                .map(|(_, r)| r.clone())
                .collect();
            (!reasons.is_empty()).then(|| Step {
                approver,
                person: approver.person(rep).id.into(),
                reasons,
                approved_at: String::new(),
            })
        })
        .collect()
}

/// "Daniel Reyes (manager), then Marcus Hale (VP)".
pub fn route_text(steps: &[Step]) -> String {
    steps
        .iter()
        .map(|s| format!("{} ({})", people::name(&s.person), s.approver.label()))
        .collect::<Vec<_>>()
        .join(", then ")
}

// ---- A request and how it moves -----------------------------------------

/// Where a request is, as stored. Expired isn't stored: an approval expires
/// by itself, so `status` works it out from the time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum State {
    /// Waiting for steps[step] since `since`.
    Waiting {
        step: usize,
        since: String,
    },
    Approved {
        until: String,
    },
    Rejected,
    SentBack,
    Withdrawn,
    Won,
    Lost,
}

/// Where a request is, for people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Waiting,
    SentBack,
    Approved,
    Expired,
    Won,
    Lost,
    Rejected,
    Withdrawn,
}

impl Status {
    pub const ALL: [Status; 8] = [
        Status::Waiting,
        Status::SentBack,
        Status::Approved,
        Status::Expired,
        Status::Won,
        Status::Lost,
        Status::Rejected,
        Status::Withdrawn,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Status::Waiting => "waiting",
            Status::SentBack => "sent-back",
            Status::Approved => "approved",
            Status::Expired => "expired",
            Status::Won => "won",
            Status::Lost => "lost",
            Status::Rejected => "rejected",
            Status::Withdrawn => "withdrawn",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Waiting => "Waiting",
            Status::SentBack => "Sent back",
            Status::Approved => "Approved",
            Status::Expired => "Expired",
            Status::Won => "Won",
            Status::Lost => "Lost",
            Status::Rejected => "Rejected",
            Status::Withdrawn => "Withdrawn",
        }
    }

    pub fn parse(text: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|s| s.key() == text)
    }
}

/// The final decision of the current round: the last approval (or the
/// policy's) or a rejection. A send-back isn't final.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub at: String,
    pub approved: bool,
    pub by_policy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Submitted,
    Resubmitted,
    AutoApproved,
    Approved,
    Rejected,
    SentBack,
    Withdrawn,
    Won,
    Lost,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Submitted => "Submitted",
            Action::Resubmitted => "Changed and sent again",
            Action::AutoApproved => "Approved automatically",
            Action::Approved => "Approved",
            Action::Rejected => "Rejected",
            Action::SentBack => "Sent back for changes",
            Action::Withdrawn => "Withdrew the request",
            Action::Won => "Marked the deal won",
            Action::Lost => "Marked the deal lost",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Action::Submitted | Action::Resubmitted => "submitted",
            Action::AutoApproved | Action::Approved | Action::Won => "approved",
            Action::Rejected | Action::Lost => "rejected",
            Action::SentBack => "sent-back",
            Action::Withdrawn => "withdrawn",
        }
    }
}

/// The id `by` has when the policy acted, not a person.
pub const POLICY: &str = "policy";

/// One line of a request's history: who did what and when, with their
/// comment and the terms and numbers at that moment. Only ever appended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub at: String,
    /// A person's id, or POLICY.
    pub by: String,
    pub action: Action,
    pub comment: String,
    /// What the desk adds: who has to approve, until when it's valid.
    pub note: String,
    pub terms: Terms,
    pub numbers: Numbers,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscountRequest {
    /// D-<number>: what people quote.
    pub number: u32,
    /// The rep's id.
    pub rep: String,
    pub created_at: String,
    pub terms: Terms,
    pub numbers: Numbers,
    /// The approvals of the current round.
    pub steps: Vec<Step>,
    pub state: State,
    pub decision: Option<Decision>,
    pub history: Vec<Event>,
}

/// What an approver decides. Rejecting and sending back need a comment.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Approve(String),
    Reject(String),
    SendBack(String),
}

#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
pub enum VerdictError {
    #[error("Pick approve, send back or reject.")]
    Unknown,
    #[error("Tell the rep why you reject it.")]
    RejectNeedsComment,
    #[error("Tell the rep what to change, like \"go to 15% with a 2-year term and I'll sign it\".")]
    SendBackNeedsComment,
    #[error("Keep the comment to {COMMENT_MAX} characters.")]
    TooLong,
}

impl Verdict {
    pub fn parse(choice: &str, comment: &str) -> Result<Verdict, VerdictError> {
        let comment = comment.trim();
        if comment.chars().count() > COMMENT_MAX {
            return Err(VerdictError::TooLong);
        }
        let comment = comment.to_string();
        match choice {
            "approve" => Ok(Verdict::Approve(comment)),
            "reject" if comment.is_empty() => Err(VerdictError::RejectNeedsComment),
            "reject" => Ok(Verdict::Reject(comment)),
            "send_back" if comment.is_empty() => Err(VerdictError::SendBackNeedsComment),
            "send_back" => Ok(Verdict::SendBack(comment)),
            _ => Err(VerdictError::Unknown),
        }
    }
}

/// What a rep does with their request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Withdraw,
    Won,
    Lost,
}

impl Outcome {
    pub fn parse(text: &str) -> Option<Outcome> {
        match text {
            "withdraw" => Some(Outcome::Withdraw),
            "won" => Some(Outcome::Won),
            "lost" => Some(Outcome::Lost),
            _ => None,
        }
    }
}

/// An optional comment, trimmed; None if it's too long.
pub fn parse_comment(text: &str) -> Option<String> {
    let text = text.trim();
    (text.chars().count() <= COMMENT_MAX).then(|| text.to_string())
}

/// Why an approver can't decide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DecideRefused {
    NotYourTurn,
}

/// Why a rep can't resubmit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResubmitRefused {
    OtherRep,
    NotSentBack,
}

/// Why a rep can't withdraw or close.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CloseRefused {
    OtherRep,
    NotOpen,
    Expired,
}

impl DiscountRequest {
    /// A rep's new request. Up to 10% with standard terms, it's approved
    /// right away; otherwise it waits for the first approver.
    pub fn submit(number: u32, rep: Rep, terms: Terms, now: &str) -> DiscountRequest {
        let mut request = DiscountRequest {
            number,
            rep: rep.person.id.into(),
            created_at: now.into(),
            numbers: terms.numbers(),
            terms,
            steps: vec![],
            state: State::SentBack,
            decision: None,
            history: vec![],
        };
        request.start_round(rep, Action::Submitted, String::new(), now);
        request
    }

    /// "D-7".
    pub fn reference(&self) -> String {
        format!("D-{}", self.number)
    }

    pub fn status(&self, now: i64) -> Status {
        match &self.state {
            State::Waiting { .. } => Status::Waiting,
            State::Approved { until } if now >= time::secs(until) => Status::Expired,
            State::Approved { .. } => Status::Approved,
            State::Rejected => Status::Rejected,
            State::SentBack => Status::SentBack,
            State::Withdrawn => Status::Withdrawn,
            State::Won => Status::Won,
            State::Lost => Status::Lost,
        }
    }

    /// The step waiting for a decision, and since when.
    pub fn waiting(&self) -> Option<(&Step, &str)> {
        match &self.state {
            State::Waiting { step, since } => self.steps.get(*step).map(|s| (s, since.as_str())),
            _ => None,
        }
    }

    pub fn is_waiting_for(&self, person: &str) -> bool {
        self.waiting().is_some_and(|(s, _)| s.person == person)
    }

    /// Waiting for a decision for more than 24 hours.
    pub fn is_overdue(&self, now: i64) -> bool {
        self.waiting()
            .is_some_and(|(_, since)| now - time::secs(since) > OVERDUE_AFTER)
    }

    /// When the approval runs out, for approved requests.
    pub fn valid_until(&self) -> Option<&str> {
        match &self.state {
            State::Approved { until } => Some(until),
            _ => None,
        }
    }

    pub fn decide(
        &mut self,
        who: &Person,
        verdict: Verdict,
        now: &str,
    ) -> Result<(), DecideRefused> {
        let index = match &self.state {
            State::Waiting { step, .. }
                if self.steps.get(*step).is_some_and(|s| s.person == who.id) =>
            {
                *step
            }
            _ => return Err(DecideRefused::NotYourTurn),
        };
        let approver = self.steps[index].approver;
        match verdict {
            Verdict::Approve(comment) => {
                self.steps[index].approved_at = now.into();
                match self.steps.get(index + 1) {
                    Some(next) => {
                        let note = format!(
                            "As {}. Next: {}.",
                            approver.label(),
                            route_text(std::slice::from_ref(next))
                        );
                        self.record(who.id, Action::Approved, comment, note, now);
                        self.state = State::Waiting {
                            step: index + 1,
                            since: now.into(),
                        };
                    }
                    None => {
                        let until = time::add(now, VALID_DAYS * time::DAY);
                        let note = format!(
                            "As {}. Final approval, valid until {}.",
                            approver.label(),
                            time::day(&until)
                        );
                        self.record(who.id, Action::Approved, comment, note, now);
                        self.approved(until, false, now);
                    }
                }
            }
            Verdict::Reject(comment) => {
                self.record(
                    who.id,
                    Action::Rejected,
                    comment,
                    format!("As {}.", approver.label()),
                    now,
                );
                self.state = State::Rejected;
                self.decision = Some(Decision {
                    at: now.into(),
                    approved: false,
                    by_policy: false,
                });
            }
            Verdict::SendBack(comment) => {
                self.record(
                    who.id,
                    Action::SentBack,
                    comment,
                    format!("As {}.", approver.label()),
                    now,
                );
                self.state = State::SentBack;
            }
        }
        Ok(())
    }

    /// The rep changed a request that was sent back: approvals start over
    /// from the first approver, for the new numbers.
    pub fn resubmit(
        &mut self,
        rep: Rep,
        terms: Terms,
        comment: String,
        now: &str,
    ) -> Result<(), ResubmitRefused> {
        if self.rep != rep.person.id {
            return Err(ResubmitRefused::OtherRep);
        }
        if self.state != State::SentBack {
            return Err(ResubmitRefused::NotSentBack);
        }
        self.terms = terms;
        self.start_round(rep, Action::Resubmitted, comment, now);
        Ok(())
    }

    /// Withdraw, or mark the deal won or lost.
    pub fn close(
        &mut self,
        who: &Person,
        outcome: Outcome,
        comment: String,
        now: &str,
    ) -> Result<(), CloseRefused> {
        if self.rep != who.id {
            return Err(CloseRefused::OtherRep);
        }
        let status = self.status(time::secs(now));
        let (action, state) = match (outcome, status) {
            (Outcome::Withdraw, Status::Waiting | Status::SentBack | Status::Approved) => {
                (Action::Withdrawn, State::Withdrawn)
            }
            (Outcome::Won, Status::Approved) => (Action::Won, State::Won),
            (Outcome::Won, Status::Expired) => return Err(CloseRefused::Expired),
            (Outcome::Lost, Status::Approved | Status::Expired) => (Action::Lost, State::Lost),
            _ => return Err(CloseRefused::NotOpen),
        };
        self.record(who.id, action, comment, String::new(), now);
        self.state = state;
        Ok(())
    }

    fn start_round(&mut self, rep: Rep, action: Action, comment: String, now: &str) {
        self.numbers = self.terms.numbers();
        self.steps = route(rep, &self.terms);
        self.decision = None;
        if self.steps.is_empty() {
            self.record(
                rep.person.id,
                action,
                comment,
                "No approval needed.".into(),
                now,
            );
            let until = time::add(now, VALID_DAYS * time::DAY);
            let note = format!(
                "Up to {AUTO_UP_TO}, annual billing upfront, Net 30: approved by policy, valid until {}.",
                time::day(&until)
            );
            self.record(POLICY, Action::AutoApproved, String::new(), note, now);
            self.approved(until, true, now);
        } else {
            let note = format!("Needs {}.", route_text(&self.steps));
            self.record(rep.person.id, action, comment, note, now);
            self.state = State::Waiting {
                step: 0,
                since: now.into(),
            };
        }
    }

    fn approved(&mut self, until: String, by_policy: bool, now: &str) {
        self.state = State::Approved { until };
        self.decision = Some(Decision {
            at: now.into(),
            approved: true,
            by_policy,
        });
    }

    fn record(&mut self, by: &str, action: Action, comment: String, note: String, now: &str) {
        self.history.push(Event {
            at: now.into(),
            by: by.into(),
            action,
            comment,
            note,
            terms: self.terms.clone(),
            numbers: self.numbers,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-11-13T10:00:00+01:00";
    const TODAY: &str = "2026-11-13";

    fn rep(id: &str) -> Rep {
        Rep::of(people::find(id).unwrap()).unwrap()
    }

    fn person(id: &str) -> &'static Person {
        people::find(id).unwrap()
    }

    fn terms(plan: Plan, seats: u32, years: u32, discount: &str) -> Terms {
        Terms {
            customer: "Acme".into(),
            plan,
            seats,
            years,
            discount: Discount::parse(discount).unwrap(),
            close_date: "2026-12-15".into(),
            justification: "They have a competing offer from Rival at a much lower price.".into(),
            ..Terms::default()
        }
    }

    fn approvers(steps: &[Step]) -> Vec<(Approver, String)> {
        steps
            .iter()
            .map(|s| (s.approver, s.person.clone()))
            .collect()
    }

    fn input() -> TermsInput {
        TermsInput {
            customer: "  Brightwater   Health ".into(),
            link: "https://kitewell.lightning.force.com/lightning/r/Opportunity/006A/view".into(),
            plan: "business".into(),
            seats: "120".into(),
            years: "2".into(),
            discount: "18".into(),
            billing: "annual".into(),
            payment: "net30".into(),
            competitor: "Rival".into(),
            close_date: "2026-12-15".into(),
            justification: "Rival offered 25% off; we keep the deal with 18% and a 2-year term."
                .into(),
        }
    }

    #[test]
    fn the_example_from_the_request() {
        let n = terms(Plan::Business, 120, 2, "18").numbers();
        assert_eq!(dollars(n.list), "$70,800");
        assert_eq!(dollars(n.discount), "$12,744");
        assert_eq!(dollars(n.net_arr), "$58,056");
        assert_eq!(dollars(n.tcv), "$116,112");
        assert_eq!(dollars(n.discount_total), "$25,488");
    }

    #[test]
    fn halves_are_exact() {
        let n = terms(Plan::Team, 3, 1, "12.5").numbers();
        assert_eq!(dollars(n.list), "$870");
        assert_eq!(dollars(n.discount), "$108.75");
        assert_eq!(dollars(n.net_arr), "$761.25");
    }

    #[test]
    fn discounts_parse() {
        assert_eq!(Discount::parse("12.5").unwrap().tenths(), 125);
        assert_eq!(Discount::parse(" 12,5 % ").unwrap().tenths(), 125);
        assert_eq!(Discount::parse("18").unwrap().to_string(), "18%");
        assert_eq!(Discount::parse("18.50").unwrap().to_string(), "18.5%");
        assert_eq!(Discount::parse(".5").unwrap().tenths(), 5);
        assert_eq!(Discount::parse("0").unwrap().tenths(), 0);
        assert_eq!(Discount::parse("50").unwrap().tenths(), 500);
        assert_eq!(Discount::parse("50.5"), Err(FieldError::DiscountTooHigh));
        assert_eq!(Discount::parse("12.3"), Err(FieldError::DiscountNotHalf));
        assert_eq!(Discount::parse("12.25"), Err(FieldError::DiscountNotHalf));
        assert_eq!(Discount::parse(""), Err(FieldError::DiscountMissing));
        for bad in ["-5", "abc", "1e2", "12.5.5", ".", "1000"] {
            assert_eq!(
                Discount::parse(bad),
                Err(FieldError::DiscountInvalid),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_whole_form_parses() {
        let t = Terms::parse(&input(), TODAY).unwrap();
        assert_eq!(t.customer, "Brightwater Health");
        assert_eq!(t.plan, Plan::Business);
        assert_eq!(t.seats, 120);
        assert_eq!(t.discount.tenths(), 180);
        assert_eq!(
            t.summary(),
            "Business, 120 seats, 2 years, 18% off, annually upfront billing, Net 30"
        );
        assert_eq!(TermsInput::of(&t).discount, "18");
    }

    #[test]
    fn seat_counts_accept_grouping_but_never_turn_decimals_into_more_seats() {
        for grouped in [
            "100000", "100,000", "100.000", "100 000", "100'000", " 120 ",
        ] {
            assert!(parse_seats(grouped).is_ok(), "{grouped}");
        }
        for invalid in [
            "2.5", "2,5", "12.50", "1 2", "1,,000", "1,00,000", "1000000",
        ] {
            assert_eq!(
                parse_seats(invalid),
                Err(FieldError::SeatsInvalid),
                "{invalid}"
            );
        }
    }

    #[test]
    fn every_bad_field_is_named() {
        let bad = TermsInput {
            customer: " ".into(),
            link: "http://evil.example/salesforce.com".into(),
            plan: "starter".into(),
            seats: "0".into(),
            years: "4".into(),
            discount: "51".into(),
            billing: "weekly".into(),
            payment: "net90".into(),
            competitor: "x".repeat(81),
            close_date: "2026-11-12".into(),
            justification: "Customer asked".into(),
        };
        let errors = Terms::parse(&bad, TODAY).unwrap_err();
        assert_eq!(errors.len(), 11);
        assert_eq!(errors.message("discount"), "More than 50% is not possible.");
        assert!(errors.message("justification").contains("real sentence"));
        assert!(errors.message("plan").contains("Starter"));
        assert_eq!(
            errors.message("customer"),
            FieldError::CustomerMissing.to_string()
        );
        assert_eq!(
            errors.iter().next(),
            Some(("customer", &FieldError::CustomerMissing))
        );
    }

    #[test]
    fn the_link_is_optional_but_must_be_salesforce() {
        assert_eq!(parse_link(""), Ok(String::new()));
        assert!(parse_link("https://kitewell.my.salesforce.com/006A").is_ok());
        for bad in [
            "kitewell.my.salesforce.com/006A",
            "https://salesforce.com.evil.example/x",
            "https://user@evil.example/.salesforce.com",
            "javascript:alert(1)//.salesforce.com",
            "https://kitewell.my.salesforce.com/a b",
        ] {
            assert_eq!(parse_link(bad), Err(FieldError::LinkInvalid), "{bad}");
        }
    }

    #[test]
    fn close_dates() {
        assert!(parse_close_date("2026-11-13", TODAY).is_ok());
        assert_eq!(
            parse_close_date("2026-11-12", TODAY),
            Err(FieldError::CloseDatePast)
        );
        assert_eq!(
            parse_close_date("13.11.2026", TODAY),
            Err(FieldError::CloseDateInvalid)
        );
        assert_eq!(
            parse_close_date("2030-01-01", TODAY),
            Err(FieldError::CloseDateTooFar)
        );
    }

    #[test]
    fn the_preview_shows_what_it_can() {
        let mut i = input();
        let p = preview(rep("priya"), &i);
        assert_eq!(dollars(p.numbers.unwrap().net_arr), "$58,056");
        assert_eq!(
            approvers(&p.steps.unwrap()),
            [(Approver::Manager, "daniel".to_string())]
        );
        i.payment.clear();
        let p = preview(rep("priya"), &i);
        assert!(p.numbers.is_some() && p.steps.is_none());
        i.discount = "60".into();
        let p = preview(rep("priya"), &i);
        assert!(p.numbers.is_none());
        assert_eq!(p.discount_error, Some(FieldError::DiscountTooHigh));
        i.discount.clear();
        assert_eq!(preview(rep("priya"), &i).discount_error, None);
    }

    #[test]
    fn up_to_10_percent_needs_nobody() {
        assert!(route(rep("priya"), &terms(Plan::Team, 10, 1, "10")).is_empty());
        assert!(route(rep("priya"), &terms(Plan::Team, 10, 1, "0")).is_empty());
    }

    #[test]
    fn the_discount_decides_who_approves() {
        use Approver::*;
        let r = |d| {
            route(rep("tom"), &terms(Plan::Team, 10, 1, d))
                .iter()
                .map(|s| s.approver)
                .collect::<Vec<_>>()
        };
        assert_eq!(r("10.5"), [Manager]);
        assert_eq!(r("20"), [Manager]);
        assert_eq!(r("20.5"), [Manager, Vp]);
        assert_eq!(r("30"), [Manager, Vp]);
        assert_eq!(r("30.5"), [Manager, Vp, Cfo]);
        assert_eq!(r("50"), [Manager, Vp, Cfo]);
        let people: Vec<String> = route(rep("tom"), &terms(Plan::Team, 10, 1, "50"))
            .into_iter()
            .map(|s| s.person)
            .collect();
        assert_eq!(people, ["daniel", "marcus", "elena"]);
    }

    #[test]
    fn the_manager_is_the_reps_manager() {
        let steps = route(rep("ken"), &terms(Plan::Team, 10, 1, "15"));
        assert_eq!(steps[0].person, "grace");
    }

    #[test]
    fn billing_and_payment_terms_need_the_cfo_even_at_5_percent() {
        let mut t = terms(Plan::Team, 10, 1, "5");
        t.billing = Billing::Monthly;
        let steps = route(rep("tom"), &t);
        assert_eq!(approvers(&steps), [(Approver::Cfo, "elena".to_string())]);
        assert_eq!(steps[0].reasons, ["monthly billing"]);

        let mut t = terms(Plan::Team, 10, 1, "5");
        t.payment = Payment::Net45;
        assert_eq!(route(rep("tom"), &t)[0].reasons, ["Net 45 payment terms"]);
    }

    #[test]
    fn big_deals_need_the_vp_whatever_the_discount() {
        // 200 Business seats at 5% off: $112,100 net ARR.
        let steps = route(rep("sofia"), &terms(Plan::Business, 200, 1, "5"));
        assert_eq!(approvers(&steps), [(Approver::Vp, "marcus".to_string())]);
        assert_eq!(steps[0].reasons, ["net ARR over $100,000"]);
        // 169 Business seats: $99,710 net ARR, under the line.
        assert!(route(rep("sofia"), &terms(Plan::Business, 169, 3, "0")).is_empty());
    }

    #[test]
    fn nobody_approves_twice_and_the_order_holds() {
        // 35%, monthly, Net 60, over $100k: the CFO for three reasons, the
        // VP for two, still one step each, manager first.
        let mut t = terms(Plan::Business, 400, 3, "35");
        t.billing = Billing::Monthly;
        t.payment = Payment::Net60;
        let steps = route(rep("priya"), &t);
        assert_eq!(
            steps.iter().map(|s| s.approver).collect::<Vec<_>>(),
            Approver::ORDER
        );
        assert_eq!(steps[1].reasons.len(), 2);
        assert_eq!(steps[2].reasons.len(), 3);
        // The CFO alone on top of a manager: no VP in between.
        let mut t = terms(Plan::Team, 10, 1, "15");
        t.billing = Billing::Quarterly;
        let order: Vec<_> = route(rep("priya"), &t).iter().map(|s| s.approver).collect();
        assert_eq!(order, [Approver::Manager, Approver::Cfo]);
    }

    #[test]
    fn small_requests_are_approved_automatically_and_on_record() {
        let r = DiscountRequest::submit(1, rep("tom"), terms(Plan::Team, 40, 1, "8"), NOW);
        assert_eq!(r.status(time::secs(NOW)), Status::Approved);
        assert_eq!(r.valid_until(), Some("2026-12-13T10:00:00+01:00"));
        let actions: Vec<_> = r.history.iter().map(|e| e.action).collect();
        assert_eq!(actions, [Action::Submitted, Action::AutoApproved]);
        assert_eq!(r.history[1].by, POLICY);
        assert!(r.decision.as_ref().unwrap().by_policy);
    }

    #[test]
    fn approvals_go_in_order_and_only_the_next_one_can_act() {
        let mut r = DiscountRequest::submit(2, rep("priya"), terms(Plan::Team, 10, 1, "35"), NOW);
        assert!(r.is_waiting_for("daniel"));
        let ok = Verdict::Approve(String::new());
        assert_eq!(
            r.decide(person("marcus"), ok.clone(), NOW),
            Err(DecideRefused::NotYourTurn)
        );
        assert_eq!(
            r.decide(person("grace"), ok.clone(), NOW),
            Err(DecideRefused::NotYourTurn)
        );
        r.decide(person("daniel"), ok.clone(), NOW).unwrap();
        assert!(r.is_waiting_for("marcus"));
        assert_eq!(
            r.decide(person("daniel"), ok.clone(), NOW),
            Err(DecideRefused::NotYourTurn)
        );
        r.decide(person("marcus"), ok.clone(), NOW).unwrap();
        r.decide(
            person("elena"),
            Verdict::Approve("Fine.".into()),
            "2026-11-14T09:00:00+01:00",
        )
        .unwrap();
        assert_eq!(r.status(time::secs(NOW)), Status::Approved);
        assert_eq!(r.decision.as_ref().unwrap().at, "2026-11-14T09:00:00+01:00");
        assert_eq!(r.history.last().unwrap().comment, "Fine.");
        assert!(
            r.history
                .last()
                .unwrap()
                .note
                .contains("valid until 14 Dec 2026")
        );
        assert_eq!(
            r.decide(person("elena"), ok, NOW),
            Err(DecideRefused::NotYourTurn)
        );
    }

    #[test]
    fn approvals_expire_after_30_days() {
        let r = DiscountRequest::submit(3, rep("tom"), terms(Plan::Team, 40, 1, "8"), NOW);
        let later = time::secs(NOW) + 30 * time::DAY;
        assert_eq!(r.status(later - 1), Status::Approved);
        assert_eq!(r.status(later), Status::Expired);
    }

    #[test]
    fn waiting_over_24_hours_is_overdue() {
        let r = DiscountRequest::submit(4, rep("tom"), terms(Plan::Team, 10, 1, "15"), NOW);
        let now = time::secs(NOW);
        assert!(!r.is_overdue(now + 24 * time::HOUR));
        assert!(r.is_overdue(now + 24 * time::HOUR + 1));
    }

    #[test]
    fn rejecting_and_sending_back_need_a_comment() {
        assert_eq!(
            Verdict::parse("reject", "  "),
            Err(VerdictError::RejectNeedsComment)
        );
        assert_eq!(
            Verdict::parse("send_back", ""),
            Err(VerdictError::SendBackNeedsComment)
        );
        assert_eq!(
            Verdict::parse("approve", ""),
            Ok(Verdict::Approve(String::new()))
        );
        assert_eq!(
            Verdict::parse("approve", &"x".repeat(1001)),
            Err(VerdictError::TooLong)
        );
        assert_eq!(Verdict::parse("delete", "x"), Err(VerdictError::Unknown));
    }

    #[test]
    fn a_rejection_is_final() {
        let mut r = DiscountRequest::submit(5, rep("ken"), terms(Plan::Team, 10, 1, "25"), NOW);
        r.decide(
            person("grace"),
            Verdict::Reject("Too much for a 1-year deal.".into()),
            NOW,
        )
        .unwrap();
        assert_eq!(r.status(0), Status::Rejected);
        assert!(!r.decision.as_ref().unwrap().approved);
        let mut again = r.clone();
        assert_eq!(
            again.close(person("ken"), Outcome::Won, String::new(), NOW),
            Err(CloseRefused::NotOpen)
        );
        assert_eq!(
            again.resubmit(rep("ken"), r.terms.clone(), String::new(), NOW),
            Err(ResubmitRefused::NotSentBack)
        );
    }

    #[test]
    fn sent_back_and_changed_starts_over_with_the_new_numbers() {
        let mut r =
            DiscountRequest::submit(6, rep("priya"), terms(Plan::Business, 80, 1, "25"), NOW);
        r.decide(person("daniel"), Verdict::Approve(String::new()), NOW)
            .unwrap();
        r.decide(
            person("marcus"),
            Verdict::SendBack("Go to 15% with 2 years and I'll sign.".into()),
            NOW,
        )
        .unwrap();
        assert_eq!(r.status(0), Status::SentBack);
        assert_eq!(
            r.decide(person("marcus"), Verdict::Approve(String::new()), NOW),
            Err(DecideRefused::NotYourTurn)
        );

        let changed = terms(Plan::Business, 80, 2, "15");
        assert_eq!(
            r.resubmit(rep("tom"), changed.clone(), String::new(), NOW),
            Err(ResubmitRefused::OtherRep)
        );
        r.resubmit(
            rep("priya"),
            changed,
            "15% for 2 years, as asked.".into(),
            NOW,
        )
        .unwrap();
        // Starts over at the manager; the VP isn't needed at 15% any more.
        assert!(r.is_waiting_for("daniel"));
        assert_eq!(r.steps.len(), 1);
        assert!(r.steps[0].approved_at.is_empty());
        assert_eq!(dollars(r.numbers.tcv), "$80,240");
        // The history keeps the numbers of each moment.
        let first = &r.history[0];
        let last = r.history.last().unwrap();
        assert_eq!(first.terms.discount.to_string(), "25%");
        assert_eq!(last.terms.discount.to_string(), "15%");
        assert_eq!(last.action, Action::Resubmitted);
        assert_ne!(first.numbers, last.numbers);
    }

    #[test]
    fn sent_back_and_changed_to_10_percent_is_approved_automatically() {
        let mut r = DiscountRequest::submit(7, rep("priya"), terms(Plan::Team, 10, 1, "15"), NOW);
        r.decide(person("daniel"), Verdict::SendBack("10% max.".into()), NOW)
            .unwrap();
        r.resubmit(
            rep("priya"),
            terms(Plan::Team, 10, 1, "10"),
            String::new(),
            NOW,
        )
        .unwrap();
        assert_eq!(r.status(0), Status::Approved);
    }

    #[test]
    fn reps_withdraw_and_close_their_own_requests() {
        let mut r = DiscountRequest::submit(8, rep("tom"), terms(Plan::Team, 40, 1, "8"), NOW);
        assert_eq!(
            r.close(person("priya"), Outcome::Won, String::new(), NOW),
            Err(CloseRefused::OtherRep)
        );
        let mut lost = r.clone();
        r.close(person("tom"), Outcome::Won, String::new(), NOW)
            .unwrap();
        assert_eq!(r.status(0), Status::Won);
        assert_eq!(
            r.close(person("tom"), Outcome::Lost, String::new(), NOW),
            Err(CloseRefused::NotOpen)
        );

        let expired = time::add(NOW, 31 * time::DAY);
        assert_eq!(
            lost.close(person("tom"), Outcome::Won, String::new(), &expired),
            Err(CloseRefused::Expired)
        );
        lost.close(person("tom"), Outcome::Lost, String::new(), &expired)
            .unwrap();
        assert_eq!(lost.status(0), Status::Lost);

        let mut waiting =
            DiscountRequest::submit(9, rep("tom"), terms(Plan::Team, 10, 1, "15"), NOW);
        assert_eq!(
            waiting.close(person("tom"), Outcome::Won, String::new(), NOW),
            Err(CloseRefused::NotOpen)
        );
        waiting
            .close(person("tom"), Outcome::Withdraw, "Deal moved.".into(), NOW)
            .unwrap();
        assert_eq!(waiting.status(0), Status::Withdrawn);
        assert!(!waiting.is_waiting_for("daniel"));
    }

    #[test]
    fn stored_requests_read_back() {
        let r = DiscountRequest::submit(10, rep("tom"), terms(Plan::Team, 10, 1, "35"), NOW);
        let json = serde_json::to_string(&r).unwrap();
        let back: DiscountRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}
