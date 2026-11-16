//! Usecases: the team deletes a person's registration (a test, or someone
//! who asks for their data to be deleted), or a session nobody registered
//! for.

use super::db;
use super::domain::{RegId, Registration, Roster, Session, SessionId};
use crate::appos::kv::Unreadable;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum DeleteRegistrationError {
    #[error("there is no such registration")]
    NotFound,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum DeleteSessionError {
    #[error("there is no such session")]
    NotFound,
    #[error("people registered for the session")]
    HasRegistrations,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn delete_registration_uc(
    session: SessionId,
    id: RegId,
) -> Result<Registration, DeleteRegistrationError> {
    delete_registration_u(
        session,
        id,
        db::load_session,
        db::load_registration,
        db::load_roster,
        |reg, roster| {
            db::delete_registration(reg);
            db::save_roster(reg.session, roster);
        },
    )
}

/// A seat that frees up goes to the first on the waitlist. Data that can't
/// be read is returned before anything is deleted.
pub fn delete_registration_u(
    session_id: SessionId,
    id: RegId,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    load_registration: impl Fn(SessionId, RegId) -> Result<Option<Registration>, Unreadable>,
    load_roster: impl Fn(SessionId) -> Result<Roster, Unreadable>,
    delete: impl Fn(&Registration, &Roster),
) -> Result<Registration, DeleteRegistrationError> {
    let session = load_session(session_id)?.ok_or(DeleteRegistrationError::NotFound)?;
    let reg = load_registration(session_id, id)?.ok_or(DeleteRegistrationError::NotFound)?;
    let mut roster = load_roster(session_id)?;
    roster.remove(id, session.seats);
    delete(&reg, &roster);
    Ok(reg)
}

pub fn delete_session_uc(id: SessionId) -> Result<Session, DeleteSessionError> {
    delete_session_u(id, db::load_session, db::load_roster, db::delete_session)
}

/// Only a session nobody ever registered for: the others are cancelled. A
/// roster that can't be read is an error, not an empty one.
pub fn delete_session_u(
    id: SessionId,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    load_roster: impl Fn(SessionId) -> Result<Roster, Unreadable>,
    delete: impl Fn(SessionId),
) -> Result<Session, DeleteSessionError> {
    let session = load_session(id)?.ok_or(DeleteSessionError::NotFound)?;
    if !load_roster(id)?.is_empty() {
        return Err(DeleteSessionError::HasRegistrations);
    }
    delete(id);
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::{Status, Tracking};
    use crate::events::testing::{NOW, applicant, session_input, unreadable};
    use std::cell::RefCell;

    #[test]
    fn deleting_a_seated_person_seats_the_next() {
        let session = Session::new(session_input(), NOW);
        let a = Registration::new(
            session.id,
            applicant("a@acme.com"),
            Tracking::default(),
            NOW,
        );
        let b = RegId::new();
        let mut roster = Roster::default();
        roster.add(a.id, 1);
        roster.add(b, 1);
        let after = RefCell::new(None);
        let mut one_seat = session.clone();
        one_seat.seats = 1;
        delete_registration_u(
            session.id,
            a.id,
            |_| Ok(Some(one_seat.clone())),
            |_, _| Ok(Some(a.clone())),
            |_| Ok(roster.clone()),
            |_, r| *after.borrow_mut() = Some(r.clone()),
        )
        .unwrap();
        let after = after.into_inner().unwrap();
        assert_eq!(after.status(a.id), None);
        assert_eq!(after.status(b), Some(Status::Seated));
    }

    #[test]
    fn only_empty_sessions_are_deleted() {
        let session = Session::new(session_input(), NOW);
        let mut roster = Roster::default();
        roster.add(RegId::new(), 50);
        let result = delete_session_u(
            session.id,
            |_| Ok(Some(session.clone())),
            |_| Ok(roster.clone()),
            |_| panic!("not deleted"),
        );
        assert_eq!(result.unwrap_err(), DeleteSessionError::HasRegistrations);
        assert!(
            delete_session_u(
                session.id,
                |_| Ok(Some(session.clone())),
                |_| Ok(Roster::default()),
                |_| {}
            )
            .is_ok()
        );
    }

    #[test]
    fn a_registration_that_cant_be_read_is_an_error_and_deletes_nothing() {
        let session = Session::new(session_input(), NOW);
        let bad = unreadable("events/reg/1/2");
        let result = delete_registration_u(
            session.id,
            RegId::new(),
            |_| Ok(Some(session.clone())),
            |_, _| Err(bad.clone()),
            |_| Ok(Roster::default()),
            |_, _| panic!("not deleted"),
        );
        assert_eq!(
            result.unwrap_err(),
            DeleteRegistrationError::Unreadable(bad)
        );
    }

    #[test]
    fn a_roster_that_cant_be_read_is_an_error_and_deletes_nothing() {
        let session = Session::new(session_input(), NOW);
        let bad = unreadable("events/roster/1");
        let result = delete_session_u(
            session.id,
            |_| Ok(Some(session.clone())),
            |_| Err(bad.clone()),
            |_| panic!("not deleted"),
        );
        assert_eq!(result.unwrap_err(), DeleteSessionError::Unreadable(bad));
    }
}
