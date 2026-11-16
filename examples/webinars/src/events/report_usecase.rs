//! Usecases: the team's overview of all sessions, and one session's
//! numbers and attendee list.

use std::collections::{BTreeMap, HashMap};

use super::db;
use super::domain::{Registration, Roster, Session, SessionId, Status};
use crate::appos::kv::Unreadable;
use crate::appos::log;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ReportError {
    #[error("there is no such session")]
    NotFound,
    #[error(transparent)]
    Unreadable(#[from] Unreadable),
}

/// All sessions: upcoming ones first (soonest first), then the rest
/// (latest first).
pub fn overview_uc(now: i64) -> Vec<(Session, Roster)> {
    overview_u(now, db::load_sessions, db::load_rosters)
}

pub fn overview_u(
    now: i64,
    load_sessions: impl Fn() -> Vec<Session>,
    load_rosters: impl Fn() -> HashMap<SessionId, Roster>,
) -> Vec<(Session, Roster)> {
    let mut rosters = load_rosters();
    let mut all: Vec<(Session, Roster)> = load_sessions()
        .into_iter()
        .map(|s| {
            let roster = rosters.remove(&s.id).unwrap_or_default();
            (s, roster)
        })
        .collect();
    all.sort_by_key(|(s, _)| {
        if s.is_open(now) {
            (0, s.start)
        } else {
            (1, -s.start)
        }
    });
    all
}

/// Sign-ups from one source (or campaign).
#[derive(Debug, Default, PartialEq)]
pub struct Count {
    pub name: String,
    pub registered: usize,
    pub waitlist: usize,
    pub cancelled: usize,
}

impl Count {
    pub fn total(&self) -> usize {
        self.registered + self.waitlist + self.cancelled
    }
}

pub struct Report {
    pub session: Session,
    pub roster: Roster,
    /// Seated in the order they came, then the waitlist in its order, then
    /// cancellations.
    pub rows: Vec<(Registration, Status)>,
    pub by_source: Vec<Count>,
    pub by_campaign: Vec<Count>,
}

pub fn report_uc(id: SessionId) -> Result<Report, ReportError> {
    report_u(
        id,
        db::load_session,
        db::load_roster,
        db::load_registrations,
        log::warn,
    )
}

pub fn report_u(
    id: SessionId,
    load_session: impl Fn(SessionId) -> Result<Option<Session>, Unreadable>,
    load_roster: impl Fn(SessionId) -> Result<Roster, Unreadable>,
    load_registrations: impl Fn(SessionId) -> Vec<Registration>,
    warn: impl Fn(&str),
) -> Result<Report, ReportError> {
    let session = load_session(id)?.ok_or(ReportError::NotFound)?;
    let roster = load_roster(id)?;
    let mut rows: Vec<(Registration, Status)> = load_registrations(id)
        .into_iter()
        .filter_map(|reg| match roster.status(reg.id) {
            Some(status) => Some((reg, status)),
            None => {
                warn(&format!(
                    "registration {} is not on the roster of {id}, left out",
                    reg.id
                ));
                None
            }
        })
        .collect();
    rows.sort_by_key(|(reg, status)| match status {
        Status::Seated => (0, 0, reg.id),
        Status::Waitlist(n) => (1, *n, reg.id),
        Status::Cancelled => (2, 0, reg.id),
    });
    let by_source = count(&rows, |r| &r.source);
    let by_campaign = count(&rows, |r| &r.campaign);
    Ok(Report {
        session,
        roster,
        rows,
        by_source,
        by_campaign,
    })
}

/// Most sign-ups first; no source is "direct", no campaign "none".
fn count(rows: &[(Registration, Status)], key: impl Fn(&Registration) -> &String) -> Vec<Count> {
    let mut counts: BTreeMap<String, Count> = BTreeMap::new();
    for (reg, status) in rows {
        let name = key(reg).clone();
        let c = counts.entry(name.clone()).or_insert_with(|| Count {
            name,
            ..Count::default()
        });
        match status {
            Status::Seated => c.registered += 1,
            Status::Waitlist(_) => c.waitlist += 1,
            Status::Cancelled => c.cancelled += 1,
        }
    }
    let mut counts: Vec<Count> = counts.into_values().collect();
    counts.sort_by(|a, b| b.total().cmp(&a.total()).then(a.name.cmp(&b.name)));
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::domain::Tracking;
    use crate::events::testing::{NOW, applicant, session_input, unreadable};

    #[test]
    fn counts_by_source_and_orders_rows() {
        let session = Session::new(session_input(), NOW);
        let regs: Vec<Registration> = [
            ("a@x.com", "linkedin"),
            ("b@x.com", "linkedin"),
            ("c@x.com", ""),
            ("d@x.com", "linkedin"),
        ]
        .iter()
        .map(|(email, source)| {
            Registration::new(
                session.id,
                applicant(email),
                Tracking::parse(source, ""),
                NOW,
            )
        })
        .collect();
        let mut roster = Roster::default();
        for r in &regs {
            roster.add(r.id, 2);
        }
        roster.cancel(regs[0].id, 2);
        let report = report_u(
            session.id,
            |_| Ok(Some(session.clone())),
            |_| Ok(roster.clone()),
            |_| regs.clone(),
            |_| {},
        )
        .unwrap();
        let order: Vec<&str> = report.rows.iter().map(|(r, _)| r.email.as_str()).collect();
        assert_eq!(order, ["b@x.com", "c@x.com", "d@x.com", "a@x.com"]);
        assert_eq!(
            report.by_source[0],
            Count {
                name: "linkedin".into(),
                registered: 1,
                waitlist: 1,
                cancelled: 1
            }
        );
        assert_eq!(
            report.by_source[1],
            Count {
                name: "".into(),
                registered: 1,
                waitlist: 0,
                cancelled: 0
            }
        );
    }

    #[test]
    fn overview_upcoming_first() {
        let at = |start: i64| {
            let mut s = Session::new(session_input(), NOW);
            s.start = start;
            s
        };
        let sessions = vec![at(10), at(300), at(200), at(20)];
        let starts: Vec<i64> = overview_u(100, || sessions.clone(), HashMap::new)
            .iter()
            .map(|(s, _)| s.start)
            .collect();
        assert_eq!(starts, [200, 300, 20, 10]);
    }

    #[test]
    fn a_roster_that_cant_be_read_is_an_error() {
        let session = Session::new(session_input(), NOW);
        let bad = unreadable("events/roster/1");
        let result = report_u(
            session.id,
            |_| Ok(Some(session.clone())),
            |_| Err(bad.clone()),
            |_| vec![],
            |_| {},
        );
        assert_eq!(result.err(), Some(ReportError::Unreadable(bad)));
    }
}
