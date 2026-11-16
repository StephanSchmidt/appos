//! The desk's IO: every kv call, with the keys. Usecases get these functions
//! as parameters; nothing else touches kv.
//!
//! Keys:
//!   desk/req/00000007   request D-7, with its whole history (one key, so
//!                       an approval reads and writes it in one transaction)
//!   desk/last-number    the last request number given out
//!   desk/examples       set once the example requests are in

use super::domain::DiscountRequest;
use crate::appos::kv;

const REQUESTS: &str = "desk/req/";
const LAST_NUMBER: &str = "desk/last-number";
const EXAMPLES: &str = "desk/examples";

/// The next request number. Two reps submitting at once both read and
/// write this key, so one of them runs again and gets the next number.
/// Unreadable if the last number isn't one: starting again at 1 would
/// overwrite D-1.
pub fn next_number() -> Result<u32, kv::Unreadable> {
    next_number_u(kv::get_json(LAST_NUMBER)?, kv::get, |next| {
        kv::set_json(LAST_NUMBER, &next);
    })
}

fn next_number_u(
    last: Option<u32>,
    get: impl Fn(&str) -> Option<String>,
    save_number: impl Fn(u32),
) -> Result<u32, kv::Unreadable> {
    let invalid = |reason: String| kv::Unreadable {
        key: LAST_NUMBER.into(),
        reason,
    };
    let next = last
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| invalid("request numbers are exhausted".into()))?;
    // Read the raw key: even an unreadable request must never be replaced.
    // The shell checks this read at commit as well as the counter's read.
    if get(&key(next)).is_some() {
        return Err(invalid(format!("D-{next} already exists")));
    }
    save_number(next);
    Ok(next)
}

pub fn save(request: &DiscountRequest) {
    kv::set_json(&key(request.number), request);
}

/// Request D-<number>, if there is one; Unreadable if what is stored there
/// isn't a request.
pub fn load(number: u32) -> Result<Option<DiscountRequest>, kv::Unreadable> {
    kv::get_json(&key(number))
}

/// Every request. One that can't be read is logged and left
/// out, so it doesn't take the lists down.
pub fn load_all() -> Vec<DiscountRequest> {
    kv::scan_all_json(REQUESTS, false)
        .into_iter()
        .map(|(_, r)| r)
        .collect()
}

pub fn has_examples() -> bool {
    kv::get(EXAMPLES).is_some()
}

/// Saves the example requests and marks them as done, so they come once.
pub fn add_examples(requests: &[DiscountRequest]) {
    for r in requests {
        save(r);
    }
    let last = requests.iter().map(|r| r.number).max().unwrap_or(0);
    kv::set_json(LAST_NUMBER, &last);
    kv::set(EXAMPLES, "yes");
}

/// Keep the original eight-digit keys so existing requests remain readable.
/// Numbers above eight digits are sorted by number in list_usecase.
fn key(number: u32) -> String {
    format!("{REQUESTS}{number:08}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_key_format_is_preserved() {
        assert_eq!(key(7), "desk/req/00000007");
        assert!(key(9) < key(10));
    }

    #[test]
    fn exhausted_numbers_do_not_wrap_or_write() {
        let error = next_number_u(
            Some(u32::MAX),
            |_| panic!("read after exhaustion"),
            |_| panic!("counter changed"),
        )
        .unwrap_err();
        assert_eq!(error.key, LAST_NUMBER);
        assert!(error.reason.contains("exhausted"));
    }

    #[test]
    fn a_stale_or_missing_counter_never_overwrites_a_request() {
        for last in [None, Some(0), Some(41)] {
            let next = last.unwrap_or(0) + 1;
            let error = next_number_u(
                last,
                |k| {
                    assert_eq!(k, key(next));
                    Some("even invalid JSON must be preserved".into())
                },
                |_| panic!("counter changed"),
            )
            .unwrap_err();
            assert_eq!(error.key, LAST_NUMBER);
            assert!(error.reason.contains("already exists"));
        }
    }

    #[test]
    fn reserves_the_next_free_number() {
        use std::cell::Cell;
        for last in [None, Some(7), Some(u32::MAX - 1)] {
            let saved = Cell::new(None);
            let next = last.unwrap_or(0) + 1;
            assert_eq!(
                next_number_u(last, |_| None, |n| saved.set(Some(n))),
                Ok(next)
            );
            assert_eq!(saved.get(), Some(next));
        }
    }
}
