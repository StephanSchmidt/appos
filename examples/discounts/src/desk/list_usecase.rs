//! Usecase: all requests, for the lists and the quarter's numbers.

use super::db;
use super::domain::DiscountRequest;

/// Every request, newest first.
pub fn list_uc() -> Vec<DiscountRequest> {
    list_u(db::load_all)
}

pub fn list_u(load_all: impl Fn() -> Vec<DiscountRequest>) -> Vec<DiscountRequest> {
    let mut all = load_all();
    // Eight-digit storage keys cease to sort numerically at D-100000000.
    all.sort_by_key(|r| std::cmp::Reverse(r.number));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::examples::examples;

    #[test]
    fn newest_first() {
        let all = list_u(|| examples("2026-11-13T10:00:00+01:00"));
        assert_eq!(all[0].number, 7);
    }

    #[test]
    fn numbers_above_eight_digits_still_sort_newest_first() {
        let mut all = examples("2026-11-13T10:00:00+01:00");
        all.truncate(3);
        for (r, n) in all.iter_mut().zip([100_000_000, u32::MAX, 99_999_999]) {
            r.number = n;
        }
        let numbers: Vec<_> = list_u(|| all.clone()).iter().map(|r| r.number).collect();
        assert_eq!(numbers, [u32::MAX, 100_000_000, 99_999_999]);
    }
}
