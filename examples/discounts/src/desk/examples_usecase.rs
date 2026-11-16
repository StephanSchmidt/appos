//! Usecase: put in the example requests on the first visit, once.

use super::db;
use super::domain::DiscountRequest;
use super::examples::examples;
use crate::appos::log;

pub fn examples_uc(now: &str) {
    examples_u(now, db::has_examples, db::add_examples, log::info);
}

pub fn examples_u(
    now: &str,
    has_examples: impl Fn() -> bool,
    add_examples: impl Fn(&[DiscountRequest]),
    log: impl Fn(&str),
) {
    if has_examples() {
        return;
    }
    let all = examples(now);
    add_examples(&all);
    log(&format!("added {} example requests", all.len()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn only_once() {
        let added = Cell::new(0);
        examples_u(
            "2026-11-13T10:00:00+01:00",
            || true,
            |all| added.set(all.len()),
            |_| {},
        );
        assert_eq!(added.get(), 0);
        examples_u(
            "2026-11-13T10:00:00+01:00",
            || false,
            |all| added.set(all.len()),
            |_| {},
        );
        assert_eq!(added.get(), 7);
    }
}
