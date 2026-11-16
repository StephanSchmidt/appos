//! Usecase: someone opens their own registration page.

use super::db;
use super::domain::{RegId, Registration, Roster, Session, SessionId, Status};
use crate::appos::kv::Unreadable;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum TicketError {
    #[error("there is no such registration")]
    NotFound,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

pub struct Ticket {
    pub session: Session,
    pub roster: Roster,
    pub reg: Registration,
    pub status: Status,
}

pub fn ticket_uc(session: SessionId, id: RegId) -> Result<Ticket, TicketError> {
    ticket_u(
        session,
        id,
        db::load_session,
        db::load_registration,
        db::load_roster,
    )
}

pub fn ticket_u(
    session_id: SessionId,
    id: RegId,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    load_registration: impl Fn(SessionId, RegId) -> Result<Option<Registration>, Unreadable>,
    load_roster: impl Fn(SessionId) -> Result<Roster, Unreadable>,
) -> Result<Ticket, TicketError> {
    let session = load_session(session_id)?.ok_or(TicketError::NotFound)?;
    let reg = load_registration(session_id, id)?.ok_or(TicketError::NotFound)?;
    let roster = load_roster(session_id)?;
    // Deleted by the team: as unknown as an id nobody has.
    let status = roster.status(id).ok_or(TicketError::NotFound)?;
    Ok(Ticket {
        session,
        roster,
        reg,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::Tracking;
    use crate::events::testing::{NOW, applicant, session_input, unreadable};

    #[test]
    fn shows_the_current_status() {
        let session = Session::new(session_input(), NOW);
        let reg = Registration::new(
            session.id,
            applicant("a@acme.com"),
            Tracking::default(),
            NOW,
        );
        let mut roster = Roster::default();
        roster.add(reg.id, 50);
        let ticket = ticket_u(
            session.id,
            reg.id,
            |_| Ok(Some(session.clone())),
            |_, _| Ok(Some(reg.clone())),
            |_| Ok(roster.clone()),
        )
        .unwrap();
        assert_eq!(ticket.status, Status::Seated);
        let gone = ticket_u(
            session.id,
            reg.id,
            |_| Ok(Some(session.clone())),
            |_, _| Ok(Some(reg.clone())),
            |_| Ok(Roster::default()),
        );
        assert_eq!(gone.err(), Some(TicketError::NotFound));
    }

    #[test]
    fn a_registration_that_cant_be_read_is_an_error() {
        let session = Session::new(session_input(), NOW);
        let bad = unreadable("events/reg/1/2");
        let result = ticket_u(
            session.id,
            RegId::new(),
            |_| Ok(Some(session.clone())),
            |_, _| Err(bad.clone()),
            |_| Ok(Roster::default()),
        );
        assert_eq!(result.err(), Some(TicketError::Unreadable(bad)));
    }
}
