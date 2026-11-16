//! Usecase: someone gives up their seat or their place on the waitlist; the
//! waitlist moves up.

use super::db;
use super::domain::{RegId, Registration, Roster, Session, SessionId};
use crate::appos::kv::Unreadable;
use crate::appos::time::Time;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CancelError {
    #[error("there is no such registration")]
    NotFound,
    #[error("the session has started")]
    Started,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn cancel_uc(session: SessionId, id: RegId, now: Time) -> Result<(), CancelError> {
    cancel_u(
        session,
        id,
        now,
        db::load_session,
        db::load_registration,
        db::load_roster,
        |reg, roster| {
            db::save_registration(reg);
            db::save_roster(reg.session, roster);
        },
    )
}

/// Cancelling twice, or for a cancelled session, changes nothing. Data that
/// can't be read is returned before anything is saved.
pub fn cancel_u(
    session_id: SessionId,
    id: RegId,
    now: Time,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    load_registration: impl Fn(SessionId, RegId) -> Result<Option<Registration>, Unreadable>,
    load_roster: impl Fn(SessionId) -> Result<Roster, Unreadable>,
    save: impl Fn(&Registration, &Roster),
) -> Result<(), CancelError> {
    let session = load_session(session_id)?.ok_or(CancelError::NotFound)?;
    let mut reg = load_registration(session_id, id)?.ok_or(CancelError::NotFound)?;
    if session.is_cancelled() {
        return Ok(());
    }
    if session.has_started(now.unix()) {
        return Err(CancelError::Started);
    }
    let mut roster = load_roster(session_id)?;
    if roster.cancel(id, session.seats) {
        reg.cancelled_at = Some(now);
        save(&reg, &roster);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::{SessionInput, Status, Tracking};
    use crate::events::testing::{NOW, applicant, session_input, unreadable};
    use std::cell::RefCell;

    #[test]
    fn the_first_on_the_waitlist_moves_up() {
        let session = Session::new(
            SessionInput {
                seats: 1,
                ..session_input()
            },
            NOW,
        );
        let a = Registration::new(
            session.id,
            applicant("a@acme.com"),
            Tracking::default(),
            NOW,
        );
        let b = Registration::new(
            session.id,
            applicant("b@acme.com"),
            Tracking::default(),
            NOW,
        );
        let mut start = Roster::default();
        start.add(a.id, 1);
        start.add(b.id, 1);
        let roster = RefCell::new(start);
        let saved = RefCell::new(None);
        let cancel = || {
            cancel_u(
                session.id,
                a.id,
                NOW,
                |_| Ok(Some(session.clone())),
                |_, _| Ok(Some(a.clone())),
                |_| Ok(roster.borrow().clone()),
                |reg, r| {
                    *roster.borrow_mut() = r.clone();
                    *saved.borrow_mut() = Some(reg.cancelled_at);
                },
            )
        };
        assert_eq!(cancel(), Ok(()));
        assert_eq!(roster.borrow().status(a.id), Some(Status::Cancelled));
        assert_eq!(roster.borrow().status(b.id), Some(Status::Seated));
        assert_eq!(*saved.borrow(), Some(Some(NOW)));
        *saved.borrow_mut() = None;
        assert_eq!(cancel(), Ok(()));
        assert_eq!(*saved.borrow(), None, "a second cancel writes nothing");
    }

    #[test]
    fn too_late_once_it_started() {
        let mut session = Session::new(session_input(), NOW);
        session.start = 0;
        let a = Registration::new(
            session.id,
            applicant("a@acme.com"),
            Tracking::default(),
            NOW,
        );
        let result = cancel_u(
            session.id,
            a.id,
            NOW,
            |_| Ok(Some(session.clone())),
            |_, _| Ok(Some(a.clone())),
            |_| Ok(Roster::default()),
            |_, _| {},
        );
        assert_eq!(result, Err(CancelError::Started));
    }

    #[test]
    fn data_that_cant_be_read_is_an_error_and_saves_nothing() {
        let session = Session::new(session_input(), NOW);
        let a = Registration::new(
            session.id,
            applicant("a@acme.com"),
            Tracking::default(),
            NOW,
        );
        let bad = unreadable("events/reg/1/2");
        let result = cancel_u(
            session.id,
            a.id,
            NOW,
            |_| Ok(Some(session.clone())),
            |_, _| Err(bad.clone()),
            |_| Ok(Roster::default()),
            |_, _| panic!("nothing is saved"),
        );
        assert_eq!(result, Err(CancelError::Unreadable(bad)));
        let bad = unreadable("events/roster/1");
        let result = cancel_u(
            session.id,
            a.id,
            NOW,
            |_| Ok(Some(session.clone())),
            |_, _| Ok(Some(a.clone())),
            |_| Err(bad.clone()),
            |_, _| panic!("nothing is saved"),
        );
        assert_eq!(result, Err(CancelError::Unreadable(bad)));
    }
}
