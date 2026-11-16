//! Usecase: show one greeting.

use super::db;
use super::domain::{Greeting, GreetingId};
use crate::appos::kv::Unreadable;

/// Why a greeting can't be shown. One error type per usecase: it lists
/// what can go wrong here and nothing else, and the handler matches every
/// variant. These are outcomes the data decides; input that doesn't parse
/// never gets this far.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ShowError {
    #[error("there is no greeting with this id")]
    NotFound,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn show_uc(id: GreetingId) -> Result<Greeting, ShowError> {
    show_u(id, db::load_greeting)
}

pub fn show_u(
    id: GreetingId,
    load_greeting: impl Fn(GreetingId) -> Result<Option<Greeting>, Unreadable>,
) -> Result<Greeting, ShowError> {
    load_greeting(id)?.ok_or(ShowError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_the_greeting() {
        let greeting = show_u(GreetingId::new(), |id| {
            Ok(Some(Greeting {
                id,
                name: "Ann".into(),
                at: Default::default(),
            }))
        });
        assert_eq!(greeting.unwrap().name, "Ann");
    }

    #[test]
    fn an_unknown_id_is_not_found() {
        let result = show_u(GreetingId::new(), |_| Ok(None));
        assert_eq!(result.unwrap_err(), ShowError::NotFound);
    }

    #[test]
    fn a_greeting_that_cant_be_read_is_an_error() {
        let unreadable = Unreadable {
            key: "hello/history/1".into(),
            reason: "expected value".into(),
        };
        let result = show_u(GreetingId::new(), |_| Err(unreadable.clone()));
        assert_eq!(result.unwrap_err(), ShowError::Unreadable(unreadable));
    }
}
