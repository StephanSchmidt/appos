//! Usecase: one request.

use super::db;
use super::domain::DiscountRequest;
use crate::appos::kv::Unreadable;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ShowError {
    #[error("there is no request with this number")]
    NotFound,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn show_uc(number: u32) -> Result<DiscountRequest, ShowError> {
    show_u(number, db::load)
}

pub fn show_u(
    number: u32,
    load: impl Fn(u32) -> Result<Option<DiscountRequest>, Unreadable>,
) -> Result<DiscountRequest, ShowError> {
    load(number)?.ok_or(ShowError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_number_is_not_found() {
        assert_eq!(show_u(7, |_| Ok(None)), Err(ShowError::NotFound));
    }

    #[test]
    fn a_request_that_cant_be_read_is_an_error() {
        let unreadable = Unreadable {
            key: "desk/req/00000007".into(),
            reason: "expected value".into(),
        };
        let result = show_u(7, |_| Err(unreadable.clone()));
        assert_eq!(result, Err(ShowError::Unreadable(unreadable)));
    }
}
