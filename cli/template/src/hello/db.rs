//! The hello module's IO: every kv call, with the keys. Usecases get these
//! functions as parameters; nothing else touches kv. All keys start with
//! "hello/", so they can't collide with another module's.

use super::domain::{Greeting, GreetingId};
use crate::appos::kv;

const HISTORY: &str = "hello/history/";

/// Adds a greeting to the history under a key of its own. Requests that
/// greet at the same time write different keys, so neither has to run
/// again (see STORAGE in appos guide). A count of all hellos in one key
/// would be written by every greeting.
pub fn add_greeting(greeting: &Greeting) {
    kv::set_json(&key(greeting.id), greeting);
}

/// The greeting with this id, if there is one; Unreadable if what is
/// stored there isn't a greeting (see ERRORS in appos guide).
pub fn load_greeting(id: GreetingId) -> Result<Option<Greeting>, kv::Unreadable> {
    kv::get_json(&key(id))
}

/// The newest `n` greetings, newest first: a reverse scan starts at the
/// last key. One scan reads them all; one kv::get each would cost more of
/// the request's budget. An unreadable greeting is logged and left out, so
/// one bad entry doesn't take the whole list down.
pub fn load_history(n: usize) -> Vec<Greeting> {
    kv::scan_json(HISTORY, None, n as u32, true)
        .into_iter()
        .map(|(_, greeting)| greeting)
        .collect()
}

/// Ids sort by time, so the key order (what kv::scan returns) is the order
/// the greetings came in.
fn key(id: GreetingId) -> String {
    format!("{HISTORY}{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_the_prefix_and_the_id() {
        let id = GreetingId::new();
        assert_eq!(key(id), format!("hello/history/{id}"));
    }
}
