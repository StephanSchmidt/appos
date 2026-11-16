//! The hello module's data and its rules: plain Rust, no IO.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::appos::time::Time;

/// How many of the newest greetings the page shows.
pub const HISTORY: usize = 10;

/// The longest name, in characters. The form says the same (maxlength), so
/// the browser stops typing there; the server checks it anyway.
pub const NAME_MAX: usize = 40;

/// A name to greet. Parse, don't validate: the only way to make one is
/// `Name::parse`, so code that takes a `Name` never sees an invalid one.
/// Input is checked once, where it comes in, and the rest of the app relies
/// on the type.
#[derive(Debug)]
pub struct Name(String);

/// Why text is not a `Name`. Code and tests match on the variant; its text
/// (`#[error]`) is the message for the visitor.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum NameError {
    #[error("That's a long name! Please keep it to {max} characters.")]
    TooLong { max: usize },
}

impl Name {
    /// Parses what a visitor typed: trimmed, at most NAME_MAX characters;
    /// empty means the world. A form with more fields parses each field
    /// like this, into its own type.
    pub fn parse(input: &str) -> Result<Name, NameError> {
        let name = input.trim();
        if name.chars().count() > NAME_MAX {
            return Err(NameError::TooLong { max: NAME_MAX });
        }
        Ok(Name(if name.is_empty() { "world" } else { name }.into()))
    }
}

/// A greeting's id: a UUID v7. It starts with the time, so ids sort in the
/// order the greetings came in. The default (all zeros) is the id of data
/// saved before greetings had ids.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GreetingId(Uuid);

impl GreetingId {
    pub fn new() -> GreetingId {
        GreetingId(Uuid::now_v7())
    }

    /// Parses an id from a URL. None if it isn't one; the handler answers
    /// that like an id nobody has: not found.
    pub fn parse(text: &str) -> Option<GreetingId> {
        Uuid::try_parse(text).ok().map(GreetingId)
    }
}

impl fmt::Display for GreetingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One hello. Stored data outlives the code: `#[serde(default)]` lets data
/// saved before a new field existed still load.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Greeting {
    pub id: GreetingId,
    pub name: String,
    /// When, on the platform's clock. Stored as RFC 3339 text:
    /// "2026-11-11T14:05:09+01:00".
    pub at: Time,
}

impl Greeting {
    /// A new greeting, with a new id.
    pub fn new(name: Name, at: Time) -> Greeting {
        Greeting {
            id: GreetingId::new(),
            name: name.0,
            at,
        }
    }

    /// The date, "2026-11-11".
    pub fn date(&self) -> String {
        self.at.date().to_string()
    }

    /// The time of day, "14:05".
    pub fn time(&self) -> String {
        let (hour, minute, _) = self.at.clock();
        format!("{hour:02}:{minute:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Time {
        Time::parse("2026-11-11T14:05:09+01:00").unwrap()
    }

    fn name(input: &str) -> String {
        Name::parse(input).unwrap().0
    }

    #[test]
    fn names_are_trimmed() {
        assert_eq!(name("  Ann "), "Ann");
    }

    #[test]
    fn no_name_greets_the_world() {
        assert_eq!(name("   "), "world");
    }

    #[test]
    fn long_names_are_refused() {
        let max = "é".repeat(NAME_MAX);
        assert_eq!(name(&max), max);
        let error = Name::parse(&format!("{max}x")).unwrap_err();
        assert_eq!(error, NameError::TooLong { max: NAME_MAX });
        assert!(error.to_string().contains("40 characters"));
    }

    #[test]
    fn ids_sort_in_the_order_they_came() {
        let first = GreetingId::new();
        assert!(first < GreetingId::new());
        assert!(first.to_string() < GreetingId::new().to_string());
    }

    #[test]
    fn ids_parse_from_what_they_print() {
        let id = GreetingId::new();
        assert_eq!(GreetingId::parse(&id.to_string()), Some(id));
        assert_eq!(GreetingId::parse("42"), None);
    }

    #[test]
    fn a_greeting_knows_its_date_and_time() {
        let greeting = Greeting::new(Name::parse("Ann").unwrap(), now());
        assert_eq!(greeting.name, "Ann");
        assert_eq!(greeting.date(), "2026-11-11");
        assert_eq!(greeting.time(), "14:05");
    }

    #[test]
    fn old_data_without_a_field_still_loads() {
        let greeting: Greeting = serde_json::from_str(r#"{"name": "Ann"}"#).unwrap();
        assert_eq!(greeting.name, "Ann");
        assert_eq!(greeting.id, GreetingId::default());
    }

    #[test]
    fn greetings_store_their_time_as_text() {
        let greeting = Greeting::new(Name::parse("Ann").unwrap(), now());
        let json = serde_json::to_string(&greeting).unwrap();
        assert!(
            json.contains(r#""at":"2026-11-11T14:05:09+01:00""#),
            "{json}"
        );
    }
}
