//! Usecase: the secret part of the events team's address. Made on first
//! use and kept in kv, so every installation has its own and none is in
//! the source.

use uuid::Uuid;

use super::db;

/// The team's key, and whether it was made just now.
pub fn team_key_uc() -> (String, bool) {
    team_key_u(db::load_team_key, db::save_team_key, || {
        Uuid::new_v4().simple().to_string()
    })
}

pub fn team_key_u(
    load: impl Fn() -> Option<String>,
    save: impl Fn(&str),
    make: impl Fn() -> String,
) -> (String, bool) {
    match load() {
        Some(key) => (key, false),
        None => {
            let key = make();
            save(&key);
            (key, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn the_first_call_makes_and_keeps_a_key() {
        let saved = RefCell::new(None);
        let key = team_key_u(
            || None,
            |k| *saved.borrow_mut() = Some(k.to_string()),
            || "new".into(),
        );
        assert_eq!(key, ("new".into(), true));
        assert_eq!(saved.into_inner().as_deref(), Some("new"));
    }

    #[test]
    fn a_kept_key_stays() {
        let key = team_key_u(
            || Some("old".into()),
            |_| panic!("must not save"),
            || panic!("must not make"),
        );
        assert_eq!(key, ("old".into(), false));
    }
}
