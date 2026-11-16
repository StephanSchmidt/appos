//! Usecase: a rep changes a request that was sent back and sends it again.

use super::db;
use super::domain::{DiscountRequest, ResubmitRefused, Terms};
use super::people::Rep;
use crate::appos::kv::Unreadable;
use crate::appos::log;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ResubmitError {
    #[error("there is no request with this number")]
    NotFound,
    #[error("the request is another rep's")]
    OtherRep,
    #[error("the request wasn't sent back")]
    NotSentBack,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn resubmit_uc(
    number: u32,
    rep: Rep,
    terms: Terms,
    comment: String,
    now: &str,
) -> Result<DiscountRequest, ResubmitError> {
    resubmit_u(
        number,
        rep,
        terms,
        comment,
        now,
        db::load,
        db::save,
        log::info,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn resubmit_u(
    number: u32,
    rep: Rep,
    terms: Terms,
    comment: String,
    now: &str,
    load: impl Fn(u32) -> Result<Option<DiscountRequest>, Unreadable>,
    save: impl Fn(&DiscountRequest),
    log: impl Fn(&str),
) -> Result<DiscountRequest, ResubmitError> {
    let mut request = load(number)?.ok_or(ResubmitError::NotFound)?;
    request
        .resubmit(rep, terms, comment, now)
        .map_err(|refused| match refused {
            ResubmitRefused::OtherRep => ResubmitError::OtherRep,
            ResubmitRefused::NotSentBack => ResubmitError::NotSentBack,
        })?;
    save(&request);
    log(&format!(
        "{} sent again by {}: {:?}",
        request.reference(),
        rep.person.id,
        request.terms.summary()
    ));
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::domain::Discount;
    use crate::desk::examples::examples;
    use crate::desk::people;

    const NOW: &str = "2026-11-13T10:00:00+01:00";

    fn example(n: u32) -> Result<Option<DiscountRequest>, Unreadable> {
        Ok(examples(NOW).into_iter().find(|r| r.number == n))
    }

    #[test]
    fn only_the_rep_and_only_when_sent_back() {
        let priya = Rep::of(people::find("priya").unwrap()).unwrap();
        let tom = Rep::of(people::find("tom").unwrap()).unwrap();
        let mut terms = example(4).unwrap().unwrap().terms;
        terms.discount = Discount::parse("15").unwrap();
        terms.years = 2;
        let go = |n, rep| {
            resubmit_u(
                n,
                rep,
                terms.clone(),
                String::new(),
                NOW,
                example,
                |_| {},
                |_| {},
            )
        };
        assert_eq!(go(4, tom).unwrap_err(), ResubmitError::OtherRep);
        assert_eq!(go(7, priya).unwrap_err(), ResubmitError::NotSentBack);
        assert_eq!(go(70, priya).unwrap_err(), ResubmitError::NotFound);
        assert!(go(4, priya).unwrap().is_waiting_for("daniel"));
    }

    #[test]
    fn a_request_that_cant_be_read_is_an_error_and_nothing_is_saved() {
        let priya = Rep::of(people::find("priya").unwrap()).unwrap();
        let terms = example(4).unwrap().unwrap().terms;
        let unreadable = Unreadable {
            key: "desk/req/00000004".into(),
            reason: "expected value".into(),
        };
        let result = resubmit_u(
            4,
            priya,
            terms,
            String::new(),
            NOW,
            |_| Err(unreadable.clone()),
            |_| panic!("saved"),
            |_| panic!("logged"),
        );
        assert_eq!(result, Err(ResubmitError::Unreadable(unreadable)));
    }
}
