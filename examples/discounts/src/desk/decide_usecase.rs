//! Usecase: an approver approves, rejects or sends a request back.

use super::db;
use super::domain::{DecideRefused, DiscountRequest, Verdict};
use super::people::Person;
use crate::appos::kv::Unreadable;
use crate::appos::log;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum DecideError {
    #[error("there is no request with this number")]
    NotFound,
    #[error("the request isn't waiting for this person")]
    NotYourTurn,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn decide_uc(
    number: u32,
    who: &Person,
    verdict: Verdict,
    now: &str,
) -> Result<DiscountRequest, DecideError> {
    decide_u(number, who, verdict, now, db::load, db::save, log::info)
}

pub fn decide_u(
    number: u32,
    who: &Person,
    verdict: Verdict,
    now: &str,
    load: impl Fn(u32) -> Result<Option<DiscountRequest>, Unreadable>,
    save: impl Fn(&DiscountRequest),
    log: impl Fn(&str),
) -> Result<DiscountRequest, DecideError> {
    let mut request = load(number)?.ok_or(DecideError::NotFound)?;
    request
        .decide(who, verdict, now)
        .map_err(|refused| match refused {
            DecideRefused::NotYourTurn => DecideError::NotYourTurn,
        })?;
    save(&request);
    if let Some(event) = request.history.last() {
        log(&format!(
            "{} {} by {}",
            request.reference(),
            event.action.label(),
            who.id
        ));
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::examples::examples;
    use crate::desk::people;
    use std::cell::RefCell;

    const NOW: &str = "2026-11-13T10:00:00+01:00";

    fn example(n: u32) -> Result<Option<DiscountRequest>, Unreadable> {
        Ok(examples(NOW).into_iter().find(|r| r.number == n))
    }

    #[test]
    fn the_next_approver_decides_and_it_is_saved() {
        let saved = RefCell::new(vec![]);
        let daniel = people::find("daniel").unwrap();
        let r = decide_u(
            7,
            daniel,
            Verdict::Approve(String::new()),
            NOW,
            example,
            |r| saved.borrow_mut().push(r.clone()),
            |_| {},
        )
        .unwrap();
        assert_eq!(saved.borrow().len(), 1);
        assert!(r.decision.unwrap().approved);
    }

    #[test]
    fn others_and_unknown_numbers_are_refused() {
        let marcus = people::find("marcus").unwrap();
        let approve = || Verdict::Approve(String::new());
        assert_eq!(
            decide_u(
                7,
                marcus,
                approve(),
                NOW,
                example,
                |_| panic!("saved"),
                |_| {}
            ),
            Err(DecideError::NotYourTurn)
        );
        assert_eq!(
            decide_u(
                99,
                marcus,
                approve(),
                NOW,
                example,
                |_| panic!("saved"),
                |_| {}
            ),
            Err(DecideError::NotFound)
        );
    }

    #[test]
    fn a_request_that_cant_be_read_is_an_error_and_nothing_is_saved() {
        let daniel = people::find("daniel").unwrap();
        let unreadable = Unreadable {
            key: "desk/req/00000007".into(),
            reason: "expected value".into(),
        };
        let result = decide_u(
            7,
            daniel,
            Verdict::Approve(String::new()),
            NOW,
            |_| Err(unreadable.clone()),
            |_| panic!("saved"),
            |_| panic!("logged"),
        );
        assert_eq!(result, Err(DecideError::Unreadable(unreadable)));
    }
}
