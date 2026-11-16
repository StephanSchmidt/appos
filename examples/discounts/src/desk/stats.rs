//! The quarter's numbers for Hannah and Marcus. Plain Rust, no IO.

use super::domain::{DiscountRequest, Status};
use super::people::{self, Person};
use super::time;

#[derive(Debug, Default, PartialEq)]
pub struct Tally {
    /// Requests made in the quarter.
    pub count: usize,
    /// The sum of their discounts, in tenths of a percent.
    discount_sum: u64,
    /// Approved in the end (by people or the policy), even if later won,
    /// lost, expired or withdrawn.
    pub approved: usize,
    /// Approved or rejected.
    pub decided: usize,
    /// Discount over the contract term on deals still approved or won,
    /// in cents.
    pub given: u64,
    /// The part of `given` on won deals.
    pub given_won: u64,
    /// Seconds from request to final decision, for the ones people decided.
    decision_secs: i64,
    pub decided_by_people: usize,
}

impl Tally {
    fn add(&mut self, r: &DiscountRequest, now: i64) {
        self.count += 1;
        self.discount_sum += u64::from(r.terms.discount.tenths());
        if let Some(d) = &r.decision {
            self.decided += 1;
            if d.approved {
                self.approved += 1;
            }
            if !d.by_policy {
                self.decided_by_people += 1;
                self.decision_secs += (time::secs(&d.at) - time::secs(&r.created_at)).max(0);
            }
        }
        match r.status(now) {
            Status::Approved => self.given += r.numbers.discount_total,
            Status::Won => {
                self.given += r.numbers.discount_total;
                self.given_won += r.numbers.discount_total;
            }
            Status::Waiting
            | Status::SentBack
            | Status::Expired
            | Status::Lost
            | Status::Rejected
            | Status::Withdrawn => {}
        }
    }

    /// The average discount in tenths of a percent.
    pub fn avg_discount(&self) -> Option<u64> {
        (self.count > 0).then(|| (self.discount_sum + self.count as u64 / 2) / self.count as u64)
    }

    /// Approved of decided, in whole percent.
    pub fn approval_rate(&self) -> Option<u64> {
        (self.decided > 0)
            .then(|| (self.approved as u64 * 100 + self.decided as u64 / 2) / self.decided as u64)
    }

    pub fn avg_decision_secs(&self) -> Option<i64> {
        (self.decided_by_people > 0).then(|| self.decision_secs / self.decided_by_people as i64)
    }
}

#[derive(Debug)]
pub struct QuarterStats {
    pub year: i64,
    pub quarter: i64,
    pub all: Tally,
    /// Every rep, in the order of PEOPLE, also those without requests.
    pub reps: Vec<(&'static Person, Tally)>,
}

/// The numbers of requests made in `quarter` of `year`.
pub fn quarter_stats(
    requests: &[DiscountRequest],
    year: i64,
    quarter: i64,
    now: i64,
) -> QuarterStats {
    let mut all = Tally::default();
    let mut reps: Vec<(&'static Person, Tally)> =
        people::reps().map(|p| (p, Tally::default())).collect();
    for r in requests
        .iter()
        .filter(|r| time::quarter(&r.created_at) == Some((year, quarter)))
    {
        all.add(r, now);
        if let Some((_, tally)) = reps.iter_mut().find(|(p, _)| p.id == r.rep) {
            tally.add(r, now);
        }
    }
    QuarterStats {
        year,
        quarter,
        all,
        reps,
    }
}

/// The quarter before or after: (2026, 4) + 1 = (2027, 1).
pub fn shift(year: i64, quarter: i64, by: i64) -> (i64, i64) {
    let n = year * 4 + (quarter - 1) + by;
    (n.div_euclid(4), n.rem_euclid(4) + 1)
}

/// Parses "2026-q4".
pub fn parse_quarter(text: &str) -> Option<(i64, i64)> {
    let (y, q) = text.split_once("-q")?;
    let y: i64 = y.parse().ok()?;
    let q: i64 = q.parse().ok()?;
    ((2000..=2100).contains(&y) && (1..=4).contains(&q)).then_some((y, q))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::domain::{Discount, Outcome, Plan, Terms, Verdict};
    use crate::desk::people::Rep;

    const NOW: &str = "2026-11-13T10:00:00+01:00";

    fn submit(n: u32, rep: &str, discount: &str, at: &str) -> DiscountRequest {
        let terms = Terms {
            customer: "Acme".into(),
            plan: Plan::Team,
            seats: 100,
            years: 2,
            discount: Discount::parse(discount).unwrap(),
            close_date: "2026-12-15".into(),
            justification: "A real sentence with more than eight words in it.".into(),
            ..Terms::default()
        };
        DiscountRequest::submit(n, Rep::of(people::find(rep).unwrap()).unwrap(), terms, at)
    }

    #[test]
    fn counts_the_quarter() {
        let auto = submit(1, "tom", "10", "2026-10-20T09:00:00+01:00");
        let mut approved = submit(2, "tom", "20", "2026-11-01T09:00:00+01:00");
        approved
            .decide(
                people::find("daniel").unwrap(),
                Verdict::Approve(String::new()),
                "2026-11-02T09:00:00+01:00",
            )
            .unwrap();
        approved
            .close(
                people::find("tom").unwrap(),
                Outcome::Won,
                String::new(),
                NOW,
            )
            .unwrap();
        let mut rejected = submit(3, "ken", "25", "2026-11-03T09:00:00+01:00");
        rejected
            .decide(
                people::find("grace").unwrap(),
                Verdict::Reject("No.".into()),
                "2026-11-03T21:00:00+01:00",
            )
            .unwrap();
        let waiting = submit(4, "priya", "15", "2026-11-10T09:00:00+01:00");
        let last_quarter = submit(5, "priya", "45", "2026-09-30T09:00:00+01:00");

        let s = quarter_stats(
            &[auto, approved, rejected, waiting, last_quarter],
            2026,
            4,
            time::secs(NOW),
        );
        assert_eq!(s.all.count, 4);
        // (10 + 20 + 25 + 15) / 4 = 17.5%
        assert_eq!(s.all.avg_discount(), Some(175));
        assert_eq!((s.all.approved, s.all.decided), (2, 3));
        assert_eq!(s.all.approval_rate(), Some(67));
        // 24 h and 12 h; the automatic approval doesn't count.
        assert_eq!(s.all.avg_decision_secs(), Some(18 * time::HOUR));
        // Team 100 seats = $29,000 a year: 10% and 20% for 2 years.
        assert_eq!(s.all.given, (290_000 + 580_000) * 2);
        assert_eq!(s.all.given_won, 580_000 * 2);

        let tom = &s.reps.iter().find(|(p, _)| p.id == "tom").unwrap().1;
        assert_eq!(tom.count, 2);
        assert_eq!(tom.avg_discount(), Some(150));
        let sofia = &s.reps.iter().find(|(p, _)| p.id == "sofia").unwrap().1;
        assert_eq!(sofia.count, 0);
        assert_eq!(sofia.avg_discount(), None);
        assert_eq!(sofia.approval_rate(), None);
    }

    #[test]
    fn quarters_move() {
        assert_eq!(shift(2026, 4, 1), (2027, 1));
        assert_eq!(shift(2026, 1, -1), (2025, 4));
        assert_eq!(parse_quarter("2026-q4"), Some((2026, 4)));
        assert_eq!(parse_quarter("2026-q5"), None);
        assert_eq!(parse_quarter("q4"), None);
    }
}
