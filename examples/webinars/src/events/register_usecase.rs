//! Usecase: someone registers for a session and gets a seat or a place on
//! the waitlist.

use super::db;
use super::domain::{Applicant, RegId, Registration, Roster, Session, SessionId, Status, Tracking};
use crate::appos::kv::Unreadable;
use crate::appos::time::Time;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum RegisterError {
    #[error("there is no such session")]
    NotFound,
    #[error("the session was cancelled")]
    Cancelled,
    #[error("the session has started")]
    Closed,
    #[error("this email is already registered for the session")]
    AlreadyRegistered,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn register_uc(
    session: SessionId,
    who: Applicant,
    tracking: Tracking,
    now: Time,
) -> Result<(Registration, Status), RegisterError> {
    register_u(
        session,
        who,
        tracking,
        now,
        db::load_session,
        db::load_roster,
        db::find_email,
        |reg, roster| {
            db::save_registration(reg);
            db::save_roster(reg.session, roster);
            db::save_email(reg.session, &reg.email_key(), reg.id);
        },
    )
}

/// The roster and the email entry are read with kv::get: if someone else
/// takes a seat or registers the same email at the same moment, the shell
/// runs this again with their change, so seats never run over. Data that
/// can't be read is returned before anything is saved.
#[allow(clippy::too_many_arguments)]
pub fn register_u(
    id: SessionId,
    who: Applicant,
    tracking: Tracking,
    now: Time,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    load_roster: impl Fn(SessionId) -> Result<Roster, Unreadable>,
    find_email: impl Fn(SessionId, &str) -> Option<RegId>,
    save: impl Fn(&Registration, &Roster),
) -> Result<(Registration, Status), RegisterError> {
    let session = load_session(id)?.ok_or(RegisterError::NotFound)?;
    if session.is_cancelled() {
        return Err(RegisterError::Cancelled);
    }
    if session.has_started(now.unix()) {
        return Err(RegisterError::Closed);
    }
    let mut roster = load_roster(id)?;
    if let Some(earlier) = find_email(id, &who.email.key())
        && roster.status(earlier).is_some_and(|s| s.is_active())
    {
        return Err(RegisterError::AlreadyRegistered);
    }
    let reg = Registration::new(id, who, tracking, now);
    let status = roster.add(reg.id, session.seats);
    save(&reg, &roster);
    Ok((reg, status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::{ApplicantForm, SessionInput};
    use crate::events::testing::{NOW, applicant, session_input, unreadable};
    use std::cell::RefCell;

    struct Store {
        session: Session,
        roster: RefCell<Roster>,
        emails: RefCell<Vec<(String, RegId)>>,
    }

    impl Store {
        fn new(seats: u32) -> Store {
            let input = SessionInput {
                seats,
                ..session_input()
            };
            Store {
                session: Session::new(input, NOW),
                roster: RefCell::default(),
                emails: RefCell::default(),
            }
        }

        fn register(&self, email: &str) -> Result<Status, RegisterError> {
            register_u(
                self.session.id,
                applicant(email),
                Tracking::parse("linkedin", "q4"),
                NOW,
                |_| Ok(Some(self.session.clone())),
                |_| Ok(self.roster.borrow().clone()),
                |_, key| {
                    self.emails
                        .borrow()
                        .iter()
                        .rev()
                        .find(|(e, _)| e == key)
                        .map(|(_, id)| *id)
                },
                |reg, roster| {
                    *self.roster.borrow_mut() = roster.clone();
                    self.emails.borrow_mut().push((reg.email_key(), reg.id));
                },
            )
            .map(|(_, status)| status)
        }
    }

    #[test]
    fn seats_then_waitlist() {
        let store = Store::new(2);
        assert_eq!(store.register("a@acme.com"), Ok(Status::Seated));
        assert_eq!(store.register("b@acme.com"), Ok(Status::Seated));
        assert_eq!(store.register("c@acme.com"), Ok(Status::Waitlist(1)));
        assert_eq!(store.register("d@acme.com"), Ok(Status::Waitlist(2)));
    }

    #[test]
    fn the_same_email_twice_gets_no_second_seat() {
        let store = Store::new(2);
        store.register("a@acme.com").unwrap();
        assert_eq!(
            store.register(" A@ACME.com "),
            Err(RegisterError::AlreadyRegistered)
        );
        assert_eq!(store.roster.borrow().seated.len(), 1);
    }

    #[test]
    fn after_cancelling_one_can_register_again() {
        let store = Store::new(2);
        store.register("a@acme.com").unwrap();
        let first = store.roster.borrow().seated[0];
        store.roster.borrow_mut().cancel(first, 2);
        assert_eq!(store.register("a@acme.com"), Ok(Status::Seated));
    }

    #[test]
    fn registration_closes_when_the_session_starts() {
        let mut store = Store::new(2);
        store.session.start = NOW.unix();
        assert_eq!(store.register("a@acme.com"), Err(RegisterError::Closed));
        store.session.cancelled_at = Some(NOW);
        assert_eq!(store.register("a@acme.com"), Err(RegisterError::Cancelled));
    }

    #[test]
    fn unknown_sessions_are_not_found() {
        let result = register_u(
            SessionId::new(),
            Applicant::parse(&ApplicantForm::default()).unwrap_or_default(),
            Tracking::default(),
            NOW,
            |_| Ok(None),
            |_| Ok(Roster::default()),
            |_, _| None,
            |_, _| {},
        );
        assert_eq!(result.unwrap_err(), RegisterError::NotFound);
    }

    #[test]
    fn a_roster_that_cant_be_read_is_an_error_and_saves_nothing() {
        let session = Session::new(session_input(), NOW);
        let bad = unreadable("events/roster/1");
        let result = register_u(
            session.id,
            applicant("a@acme.com"),
            Tracking::default(),
            NOW,
            |_| Ok(Some(session.clone())),
            |_| Err(bad.clone()),
            |_, _| None,
            |_, _| panic!("nothing is saved"),
        );
        assert_eq!(result.unwrap_err(), RegisterError::Unreadable(bad));
    }
}
