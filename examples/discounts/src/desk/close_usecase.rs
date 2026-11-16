//! Usecase: a rep withdraws a request, or marks the deal won or lost.

use super::db;
use super::domain::{CloseRefused, DiscountRequest, Outcome};
use super::people::Person;
use crate::appos::kv::Unreadable;
use crate::appos::log;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CloseError {
    #[error("there is no request with this number")]
    NotFound,
    #[error("the request is another rep's")]
    OtherRep,
    #[error("the request can't take this step any more")]
    NotOpen,
    #[error("the approval has expired")]
    Expired,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn close_uc(
    number: u32,
    who: &Person,
    outcome: Outcome,
    comment: String,
    now: &str,
) -> Result<DiscountRequest, CloseError> {
    close_u(
        number,
        who,
        outcome,
        comment,
        now,
        db::load,
        db::save,
        log::info,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn close_u(
    number: u32,
    who: &Person,
    outcome: Outcome,
    comment: String,
    now: &str,
    load: impl Fn(u32) -> Result<Option<DiscountRequest>, Unreadable>,
    save: impl Fn(&DiscountRequest),
    log: impl Fn(&str),
) -> Result<DiscountRequest, CloseError> {
    let mut request = load(number)?.ok_or(CloseError::NotFound)?;
    request
        .close(who, outcome, comment, now)
        .map_err(|refused| match refused {
            CloseRefused::OtherRep => CloseError::OtherRep,
            CloseRefused::NotOpen => CloseError::NotOpen,
            CloseRefused::Expired => CloseError::Expired,
        })?;
    save(&request);
    log(&format!(
        "{} {:?} by {}",
        request.reference(),
        outcome,
        who.id
    ));
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::examples::examples;
    use crate::desk::people;

    const NOW: &str = "2026-11-13T10:00:00+01:00";

    fn example(n: u32) -> Result<Option<DiscountRequest>, Unreadable> {
        Ok(examples(NOW).into_iter().find(|r| r.number == n))
    }

    #[test]
    fn won_lost_and_withdrawn() {
        let tom = people::find("tom").unwrap();
        let go = |n, outcome| close_u(n, tom, outcome, String::new(), NOW, example, |_| {}, |_| {});
        assert!(go(1, Outcome::Won).is_ok());
        assert_eq!(go(2, Outcome::Won).unwrap_err(), CloseError::OtherRep);
        assert_eq!(go(5, Outcome::Won).unwrap_err(), CloseError::NotOpen);
        assert!(go(5, Outcome::Withdraw).is_ok());
        assert_eq!(go(50, Outcome::Lost).unwrap_err(), CloseError::NotFound);
    }

    #[test]
    fn a_request_that_cant_be_read_is_an_error_and_nothing_is_saved() {
        let tom = people::find("tom").unwrap();
        let unreadable = Unreadable {
            key: "desk/req/00000001".into(),
            reason: "expected value".into(),
        };
        let result = close_u(
            1,
            tom,
            Outcome::Won,
            String::new(),
            NOW,
            |_| Err(unreadable.clone()),
            |_| panic!("saved"),
            |_| panic!("logged"),
        );
        assert_eq!(result, Err(CloseError::Unreadable(unreadable)));
    }
}
