//! Example requests, so the desk doesn't look empty on day one: one
//! approved automatically, one approved, one rejected, one sent back, one
//! waiting for Marcus and two waiting for Daniel (one for over 24 hours).
//! Made-up customers. They are played through the same rules as real
//! requests, at times relative to the first visit.

use super::domain::{Billing, Discount, DiscountRequest, Payment, Plan, Terms, Verdict};
use super::people::{self, Rep};
use super::time::{self, DAY, HOUR};

struct Deal {
    rep: &'static str,
    customer: &'static str,
    plan: Plan,
    seats: u32,
    years: u32,
    discount: &'static str,
    billing: Billing,
    competitor: &'static str,
    close_in_days: i64,
    justification: &'static str,
}

/// Plays `actions` (hours ago, who, what) on a new request made `hours`
/// ago.
fn play(
    number: u32,
    now: &str,
    hours: i64,
    deal: Deal,
    actions: &[(i64, &str, Verdict)],
) -> DiscountRequest {
    let rep = Rep::of(people::find(deal.rep).expect("example reps exist"))
        .expect("example reps are reps");
    let terms = Terms {
        customer: deal.customer.into(),
        link: format!(
            "https://kitewell.lightning.force.com/lightning/r/Opportunity/0068d00000{number:05}AbC/view"
        ),
        plan: deal.plan,
        seats: deal.seats,
        years: deal.years,
        discount: Discount::parse(deal.discount).expect("example discounts are valid"),
        billing: deal.billing,
        payment: Payment::Net30,
        competitor: deal.competitor.into(),
        close_date: time::date(&time::add(now, deal.close_in_days * DAY)).into(),
        justification: deal.justification.into(),
    };
    let ago = |h: i64| time::add(now, -h * HOUR);
    let mut request = DiscountRequest::submit(number, rep, terms, &ago(hours));
    for (h, who, verdict) in actions {
        let who = people::find(who).expect("example approvers exist");
        request
            .decide(who, verdict.clone(), &ago(*h))
            .expect("examples follow the policy");
    }
    request
}

pub fn examples(now: &str) -> Vec<DiscountRequest> {
    vec![
        play(
            1,
            now,
            6 * 24,
            Deal {
                rep: "tom",
                customer: "Fjordline Logistics",
                plan: Plan::Team,
                seats: 40,
                years: 1,
                discount: "8",
                billing: Billing::Annual,
                competitor: "",
                close_in_days: 12,
                justification: "Early renewal two months before their term ends; 8% is what we give for renewing early and paying upfront.",
            },
            &[],
        ),
        play(
            2,
            now,
            5 * 24,
            Deal {
                rep: "sofia",
                customer: "Brightwater Health",
                plan: Plan::Business,
                seats: 120,
                years: 2,
                discount: "18",
                billing: Billing::Annual,
                competitor: "Clarion",
                close_in_days: 16,
                justification: "Clarion quoted them 25% under our list price. With 18% and a two-year term we stay ahead without matching Clarion.",
            },
            &[(4 * 24 + 20, "grace", Verdict::Approve("Fine with the two-year term.".into()))],
        ),
        play(
            3,
            now,
            4 * 24,
            Deal {
                rep: "ken",
                customer: "Copperleaf Retail",
                plan: Plan::Team,
                seats: 60,
                years: 1,
                discount: "35",
                billing: Billing::Monthly,
                competitor: "Taskara",
                close_in_days: 20,
                justification: "Taskara is free for their first year. They want monthly billing because their budget resets every month.",
            },
            &[
                (3 * 24 + 22, "grace", Verdict::Approve("OK from me if Marcus agrees.".into())),
                (
                    3 * 24 + 1,
                    "marcus",
                    Verdict::Reject("35% on a one-year Team deal with monthly billing is too much. Come back with 20% and annual billing.".into()),
                ),
            ],
        ),
        play(
            4,
            now,
            2 * 24,
            Deal {
                rep: "priya",
                customer: "Halden & Co.",
                plan: Plan::Business,
                seats: 80,
                years: 1,
                discount: "25",
                billing: Billing::Annual,
                competitor: "Brightdesk",
                close_in_days: 25,
                justification: "Brightdesk offered them a 30% launch discount. Halden is the reference customer we need in Nordic legal.",
            },
            &[(27, "daniel", Verdict::SendBack("Go to 15% with a two-year term and I'll sign it.".into()))],
        ),
        play(
            5,
            now,
            31,
            Deal {
                rep: "tom",
                customer: "Meridian Freight",
                plan: Plan::Business,
                seats: 200,
                years: 3,
                discount: "15",
                billing: Billing::Quarterly,
                competitor: "Clarion",
                close_in_days: 18,
                justification: "Three-year commitment for 200 seats. Their finance team only pays quarterly, and Clarion offered the same terms.",
            },
            &[],
        ),
        play(
            6,
            now,
            30,
            Deal {
                rep: "sofia",
                customer: "Lumen Labs",
                plan: Plan::Business,
                seats: 150,
                years: 2,
                discount: "22",
                billing: Billing::Annual,
                competitor: "Taskara",
                close_in_days: 10,
                justification: "Lumen is moving its whole research division from Taskara and will publish a case study with us.",
            },
            &[(7, "grace", Verdict::Approve("Strategic logo for the West Coast.".into()))],
        ),
        play(
            7,
            now,
            3,
            Deal {
                rep: "priya",
                customer: "Oakridge Software",
                plan: Plan::Team,
                seats: 50,
                years: 2,
                discount: "12.5",
                billing: Billing::Annual,
                competitor: "",
                close_in_days: 21,
                justification: "Second team at Oakridge; the first one pays 12.5% off and they want the same price for the same product.",
            },
            &[],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::domain::Status;

    const NOW: &str = "2026-11-13T10:00:00+01:00";

    #[test]
    fn one_of_each_as_asked() {
        let now = time::secs(NOW);
        let all = examples(NOW);
        let status: Vec<Status> = all.iter().map(|r| r.status(now)).collect();
        use Status::*;
        assert_eq!(
            status,
            [
                Approved, Approved, Rejected, SentBack, Waiting, Waiting, Waiting
            ]
        );
        assert!(all[0].decision.as_ref().unwrap().by_policy);
        assert!(!all[1].decision.as_ref().unwrap().by_policy);
        assert!(all[4].is_waiting_for("daniel"));
        assert!(all[4].is_overdue(now));
        assert_eq!(all[4].steps.len(), 3);
        assert!(all[5].is_waiting_for("marcus"));
        assert!(all[6].is_waiting_for("daniel"));
        assert!(!all[6].is_overdue(now));
    }

    #[test]
    fn numbers_and_times_go_up_together() {
        let all = examples(NOW);
        for pair in all.windows(2) {
            assert!(time::secs(&pair[0].created_at) < time::secs(&pair[1].created_at));
        }
    }
}
