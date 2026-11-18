//! A draft's quote as the page shows it: dollars, not cents. Plain Rust,
//! no IO. The prices are the platform's (flow.get); the app only formats
//! them.

use crate::appos::kitewell::booking::flow::{Billing, Draft, PaymentTerms};

pub struct QuoteView {
    /// Each line's label and its amount a year.
    pub lines: Vec<(String, String)>,
    pub per_year: String,
    pub years: u8,
    pub contract: String,
    /// How the invoices go out, in words.
    pub invoices: String,
    pub problems: Vec<String>,
}

impl QuoteView {
    pub fn of(draft: &Draft) -> QuoteView {
        let q = &draft.quote;
        let every = match draft.billing {
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
            years: draft.years,
            contract: money(cents(q.contract_cents)),
            invoices: format!(
                "One invoice {every}, {}, payable in {} days.",
                money(cents(q.per_invoice_cents)),
                days(draft.payment_terms)
            ),
            problems: q.problems.clone(),
        }
    }
}

pub fn billing_name(billing: Billing) -> &'static str {
    match billing {
        Billing::Monthly => "Monthly",
        Billing::Quarterly => "Quarterly",
        Billing::Yearly => "Yearly",
    }
}

pub fn days(terms: PaymentTerms) -> u32 {
    match terms {
        PaymentTerms::Net30 => 30,
        PaymentTerms::Net45 => 45,
        PaymentTerms::Net60 => 60,
    }
}

fn cents(c: u64) -> i64 {
    i64::try_from(c).unwrap_or(i64::MAX)
}

/// 725000 as "$7,250", 516250 as "$5,162.50", -1274400 as "-$12,744".
pub fn money(cents: i64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_in_dollars() {
        assert_eq!(money(725_000), "$7,250");
        assert_eq!(money(516_250), "$5,162.50");
        assert_eq!(money(-1_274_400), "-$12,744");
        assert_eq!(money(0), "$0");
        assert_eq!(money(11_611_200), "$116,112");
        assert_eq!(money(5), "$0.05");
    }
}
