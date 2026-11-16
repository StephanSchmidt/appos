//! Usecase: the public list of sessions, and one session's public page.

use std::collections::HashMap;

use super::db;
use super::domain::{Roster, Session, SessionId};
use crate::appos::kv::Unreadable;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SessionError {
    #[error("there is no such session")]
    NotFound,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

/// Sessions that haven't started and aren't cancelled, soonest first.
pub fn upcoming_uc(now: i64) -> Vec<(Session, Roster)> {
    upcoming_u(now, db::load_sessions, db::load_rosters)
}

pub fn upcoming_u(
    now: i64,
    load_sessions: impl Fn() -> Vec<Session>,
    load_rosters: impl Fn() -> HashMap<SessionId, Roster>,
) -> Vec<(Session, Roster)> {
    let mut rosters = load_rosters();
    let mut sessions: Vec<(Session, Roster)> = load_sessions()
        .into_iter()
        .filter(|s| s.is_open(now))
        .map(|s| {
            let roster = rosters.remove(&s.id).unwrap_or_default();
            (s, roster)
        })
        .collect();
    sessions.sort_by_key(|(s, _)| s.start);
    sessions
}

/// One session, open or not: its page says when registration has closed.
pub fn session_uc(id: SessionId) -> Result<(Session, Roster), SessionError> {
    session_u(id, db::load_session, db::load_roster)
}

pub fn session_u(
    id: SessionId,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    load_roster: impl Fn(SessionId) -> Result<Roster, Unreadable>,
) -> Result<(Session, Roster), SessionError> {
    let session = load_session(id)?.ok_or(SessionError::NotFound)?;
    Ok((session, load_roster(id)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::{NOW, session_input, unreadable};

    #[test]
    fn only_upcoming_soonest_first() {
        let at = |start: i64| {
            let mut s = Session::new(session_input(), NOW);
            s.start = start;
            s
        };
        let mut cancelled = at(500);
        cancelled.cancelled_at = Some(NOW);
        let sessions = vec![at(300), at(100), at(50), cancelled, at(200)];
        let list = upcoming_u(100, || sessions.clone(), HashMap::new);
        let starts: Vec<i64> = list.iter().map(|(s, _)| s.start).collect();
        assert_eq!(starts, [200, 300]);
    }

    #[test]
    fn a_session_that_cant_be_read_is_an_error() {
        let bad = unreadable("events/session/1");
        let result = session_u(
            SessionId::new(),
            |_| Err(bad.clone()),
            |_| Ok(Roster::default()),
        );
        assert_eq!(result.err(), Some(SessionError::Unreadable(bad)));
    }
}
