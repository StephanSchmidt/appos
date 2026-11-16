//! Usecase: a rep asks for a discount.

use super::db;
use super::domain::{DiscountRequest, Terms, route_text};
use super::people::Rep;
use crate::appos::kv::Unreadable;
use crate::appos::log;

/// Unreadable if the last number given out can't be read; then nothing is
/// saved.
pub fn submit_uc(rep: Rep, terms: Terms, now: &str) -> Result<DiscountRequest, Unreadable> {
    submit_u(rep, terms, now, db::next_number, db::save, log::info)
}

pub fn submit_u(
    rep: Rep,
    terms: Terms,
    now: &str,
    next_number: impl Fn() -> Result<u32, Unreadable>,
    save: impl Fn(&DiscountRequest),
    log: impl Fn(&str),
) -> Result<DiscountRequest, Unreadable> {
    let request = DiscountRequest::submit(next_number()?, rep, terms, now);
    save(&request);
    log(&format!(
        "{} by {}: {:?}, needs {:?}",
        request.reference(),
        rep.person.id,
        request.terms.summary(),
        route_text(&request.steps)
    ));
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::domain::{Discount, Plan};
    use crate::desk::people;
    use std::cell::RefCell;

    fn terms() -> Terms {
        Terms {
            customer: "Acme".into(),
            plan: Plan::Team,
            seats: 10,
            years: 1,
            discount: Discount::parse("15").unwrap(),
            ..Terms::default()
        }
    }

    #[test]
    fn numbers_saves_and_logs() {
        let saved = RefCell::new(vec![]);
        let logged = RefCell::new(vec![]);
        let rep = Rep::of(people::find("priya").unwrap()).unwrap();
        let r = submit_u(
            rep,
            terms(),
            "2026-11-13T10:00:00+01:00",
            || Ok(42),
            |r| saved.borrow_mut().push(r.number),
            |line| logged.borrow_mut().push(line.to_string()),
        )
        .unwrap();
        assert_eq!(r.reference(), "D-42");
        assert_eq!(*saved.borrow(), [42]);
        assert!(logged.borrow()[0].contains("Daniel Reyes (Manager)"));
    }

    #[test]
    fn a_last_number_that_cant_be_read_is_an_error_and_nothing_is_saved() {
        let rep = Rep::of(people::find("priya").unwrap()).unwrap();
        let unreadable = Unreadable {
            key: "desk/last-number".into(),
            reason: "expected value".into(),
        };
        let result = submit_u(
            rep,
            terms(),
            "2026-11-13T10:00:00+01:00",
            || Err(unreadable.clone()),
            |_| panic!("saved"),
            |_| panic!("logged"),
        );
        assert_eq!(result.unwrap_err(), unreadable);
    }
}
