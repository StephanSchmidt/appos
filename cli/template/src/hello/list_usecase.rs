//! Usecase: list the newest greetings.

use super::db;
use super::domain::{Greeting, HISTORY};

/// The newest greetings, newest first.
pub fn list_uc() -> Vec<Greeting> {
    list_u(db::load_history)
}

pub fn list_u(load_history: impl Fn(usize) -> Vec<Greeting>) -> Vec<Greeting> {
    load_history(HISTORY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_newest_greetings() {
        let history = list_u(|n| Vec::from_fn(n, |_| Greeting::default()));
        assert_eq!(history.len(), HISTORY);
    }
}
