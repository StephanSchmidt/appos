//! Usecase: greet someone.

use super::db;
use super::domain::{Greeting, Name};
use crate::appos::log;
use crate::appos::time::Time;

/// Greets `name` at `now`, adds the greeting to the history and logs it.
/// It takes a `Name`, not text: the handler has checked the input already.
pub fn greet_uc(name: Name, now: Time) -> Greeting {
    greet_u(name, now, db::add_greeting, log::info)
}

pub fn greet_u(
    name: Name,
    now: Time,
    add_greeting: impl Fn(&Greeting),
    log: impl Fn(&str),
) -> Greeting {
    let greeting = Greeting::new(name, now);
    add_greeting(&greeting);
    // {:?} quotes the name, so a name with a line break can't fake a log line.
    log(&format!("hello: {:?}", greeting.name));
    greeting
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn greets_adds_and_logs() {
        let added = RefCell::new(vec![]);
        let logged = RefCell::new(vec![]);

        let greeting = greet_u(
            Name::parse("Ann").unwrap(),
            Time::parse("2026-11-11T14:05:09+01:00").unwrap(),
            |greeting| added.borrow_mut().push(greeting.name.clone()),
            |line| logged.borrow_mut().push(line.to_string()),
        );

        assert_eq!(greeting.time(), "14:05");
        assert_eq!(*added.borrow(), ["Ann"]);
        assert_eq!(*logged.borrow(), [r#"hello: "Ann""#]);
    }
}
