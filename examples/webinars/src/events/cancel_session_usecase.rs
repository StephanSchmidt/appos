//! Usecase: the team cancels a session. It leaves the public list; its
//! people's pages say it was cancelled.

use super::db;
use super::domain::{Session, SessionId};
use crate::appos::kv::Unreadable;
use crate::appos::time::Time;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CancelSessionError {
    #[error("there is no such session")]
    NotFound,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn cancel_session_uc(id: SessionId, now: Time) -> Result<Session, CancelSessionError> {
    cancel_session_u(id, now, db::load_session, db::save_session)
}

pub fn cancel_session_u(
    id: SessionId,
    now: Time,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    save: impl Fn(&Session),
) -> Result<Session, CancelSessionError> {
    let mut session = load_session(id)?.ok_or(CancelSessionError::NotFound)?;
    if session.cancelled_at.is_none() {
        session.cancelled_at = Some(now);
        save(&session);
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::{NOW, session_input, unreadable};

    #[test]
    fn cancels_once() {
        let session = Session::new(session_input(), NOW);
        let cancelled =
            cancel_session_u(session.id, NOW, |_| Ok(Some(session.clone())), |_| {}).unwrap();
        assert!(cancelled.is_cancelled());
        cancel_session_u(
            session.id,
            NOW.plus(60),
            |_| Ok(Some(cancelled.clone())),
            |_| panic!("already cancelled"),
        )
        .unwrap();
    }

    #[test]
    fn a_session_that_cant_be_read_is_an_error_and_saves_nothing() {
        let bad = unreadable("events/session/1");
        let result = cancel_session_u(
            SessionId::new(),
            NOW,
            |_| Err(bad.clone()),
            |_| panic!("nothing is saved"),
        );
        assert_eq!(result.unwrap_err(), CancelSessionError::Unreadable(bad));
    }
}
