//! Usecase: the team adds a session or changes one. More seats move the
//! waitlist up.

use super::db;
use super::domain::{Roster, Session, SessionId, SessionInput};
use crate::appos::kv::Unreadable;
use crate::appos::time::Time;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SaveError {
    #[error("there is no such session")]
    NotFound,
    #[error("{taken} people already have a seat")]
    SeatsTaken { taken: usize },
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub fn create_uc(input: SessionInput, now: Time) -> Session {
    let session = Session::new(input, now);
    db::save_session(&session);
    session
}

pub fn update_uc(id: SessionId, input: SessionInput, now: Time) -> Result<Session, SaveError> {
    update_u(
        id,
        input,
        now,
        db::load_session,
        db::load_roster,
        |session, roster| {
            db::save_session(session);
            db::save_roster(session.id, roster);
        },
    )
}

/// Data that can't be read is returned before anything is saved.
pub fn update_u(
    id: SessionId,
    input: SessionInput,
    now: Time,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    load_roster: impl Fn(SessionId) -> Result<Roster, Unreadable>,
    save: impl Fn(&Session, &Roster),
) -> Result<Session, SaveError> {
    let mut session = load_session(id)?.ok_or(SaveError::NotFound)?;
    let mut roster = load_roster(id)?;
    if (input.seats as usize) < roster.seated.len() {
        return Err(SaveError::SeatsTaken {
            taken: roster.seated.len(),
        });
    }
    session.apply(input, now);
    if session.is_open(now.unix()) {
        roster.promote(session.seats);
    }
    save(&session, &roster);
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::{RegId, Status};
    use crate::events::testing::{NOW, session_input, unreadable};
    use std::cell::RefCell;

    fn full_session() -> (Session, Roster, Vec<RegId>) {
        let session = Session::new(
            SessionInput {
                seats: 2,
                ..session_input()
            },
            NOW,
        );
        let mut roster = Roster::default();
        let ids = Vec::from_fn(4, |_| RegId::new());
        for id in &ids {
            roster.add(*id, 2);
        }
        (session, roster, ids)
    }

    #[test]
    fn more_seats_move_the_waitlist_up() {
        let (session, roster, ids) = full_session();
        let saved = RefCell::new(None);
        let input = SessionInput {
            seats: 3,
            ..session_input()
        };
        update_u(
            session.id,
            input,
            NOW,
            |_| Ok(Some(session.clone())),
            |_| Ok(roster.clone()),
            |_, r| *saved.borrow_mut() = Some(r.clone()),
        )
        .unwrap();
        let saved = saved.into_inner().unwrap();
        assert_eq!(saved.status(ids[2]), Some(Status::Seated));
        assert_eq!(saved.status(ids[3]), Some(Status::Waitlist(1)));
    }

    #[test]
    fn fewer_seats_than_taken_is_refused() {
        let (session, roster, _) = full_session();
        let input = SessionInput {
            seats: 1,
            ..session_input()
        };
        let result = update_u(
            session.id,
            input,
            NOW,
            |_| Ok(Some(session.clone())),
            |_| Ok(roster.clone()),
            |_, _| panic!("nothing is saved"),
        );
        assert_eq!(result.unwrap_err(), SaveError::SeatsTaken { taken: 2 });
    }

    #[test]
    fn a_roster_that_cant_be_read_is_an_error_and_saves_nothing() {
        let (session, _, _) = full_session();
        let bad = unreadable("events/roster/1");
        let result = update_u(
            session.id,
            session_input(),
            NOW,
            |_| Ok(Some(session.clone())),
            |_| Err(bad.clone()),
            |_, _| panic!("nothing is saved"),
        );
        assert_eq!(result.unwrap_err(), SaveError::Unreadable(bad));
    }
}
