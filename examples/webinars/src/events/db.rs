//! The IO: every kv call, with the keys. Usecases get these functions as
//! parameters; nothing else touches kv.
//!
//! Keys:
//!   events/session/<session>          the session
//!   events/roster/<session>           who has a seat, who waits, who cancelled
//!   events/reg/<session>/<reg>        one registration
//!   events/email/<session>/<email>    the registration of that email address
//!   events/team-key                   the secret part of the team's address

use std::collections::HashMap;

use super::domain::{RegId, Registration, Roster, Session, SessionId};
use crate::appos::kv;

const SESSIONS: &str = "events/session/";
const ROSTERS: &str = "events/roster/";
const REGS: &str = "events/reg/";
const EMAILS: &str = "events/email/";
const TEAM_KEY: &str = "events/team-key";

/// The secret part of the team's address, if it was made yet.
pub fn load_team_key() -> Option<String> {
    kv::get(TEAM_KEY)
}

pub fn save_team_key(key: &str) {
    kv::set(TEAM_KEY, key);
}

/// The session, if there is one; Unreadable if what is stored there isn't
/// a session (see ERRORS in appos guide).
pub fn load_session(id: SessionId) -> Result<Option<Session>, kv::Unreadable> {
    kv::get_json(&format!("{SESSIONS}{id}"))
}

pub fn save_session(session: &Session) {
    kv::set_json(&format!("{SESSIONS}{}", session.id), session);
}

pub fn delete_session(id: SessionId) {
    kv::delete(&format!("{SESSIONS}{id}"));
    kv::delete(&format!("{ROSTERS}{id}"));
}

/// All sessions, past and cancelled ones too. An unreadable session is
/// logged and left out, so one bad entry doesn't take the list down.
pub fn load_sessions() -> Vec<Session> {
    kv::scan_all_json(SESSIONS, false)
        .into_iter()
        .map(|(_, s)| s)
        .collect()
}

/// The session's roster; empty if nobody registered yet, Unreadable if
/// what is stored there isn't a roster.
pub fn load_roster(id: SessionId) -> Result<Roster, kv::Unreadable> {
    Ok(kv::get_json(&format!("{ROSTERS}{id}"))?.unwrap_or_default())
}

pub fn save_roster(id: SessionId, roster: &Roster) {
    kv::set_json(&format!("{ROSTERS}{id}"), roster);
}

/// Every session's roster, in one scan. For overviews only: a scan isn't
/// checked for changes by other requests, so rules read with load_roster.
/// An unreadable roster is logged and left out.
pub fn load_rosters() -> HashMap<SessionId, Roster> {
    kv::scan_all_json::<Roster>(ROSTERS, false)
        .into_iter()
        .filter_map(|(key, roster)| SessionId::parse(&key[ROSTERS.len()..]).map(|id| (id, roster)))
        .collect()
}

/// The registration, if there is one; Unreadable if what is stored there
/// isn't a registration.
pub fn load_registration(
    session: SessionId,
    id: RegId,
) -> Result<Option<Registration>, kv::Unreadable> {
    kv::get_json(&format!("{REGS}{session}/{id}"))
}

pub fn save_registration(reg: &Registration) {
    kv::set_json(&format!("{REGS}{}/{}", reg.session, reg.id), reg);
}

/// Deletes the registration and, if it points to it, its email entry.
pub fn delete_registration(reg: &Registration) {
    kv::delete(&format!("{REGS}{}/{}", reg.session, reg.id));
    let email = email_key(reg.session, &reg.email_key());
    if kv::get(&email).as_deref() == Some(reg.id.to_string().as_str()) {
        kv::delete(&email);
    }
}

/// A session's registrations, oldest first. An unreadable one is logged
/// and left out.
pub fn load_registrations(session: SessionId) -> Vec<Registration> {
    kv::scan_all_json(&format!("{REGS}{session}/"), false)
        .into_iter()
        .map(|(_, r)| r)
        .collect()
}

/// The registration this email address made for the session, if any.
pub fn find_email(session: SessionId, email: &str) -> Option<RegId> {
    kv::get(&email_key(session, email)).and_then(|id| RegId::parse(&id))
}

pub fn save_email(session: SessionId, email: &str, id: RegId) {
    kv::set(&email_key(session, email), &id.to_string());
}

fn email_key(session: SessionId, email: &str) -> String {
    format!("{EMAILS}{session}/{}", email.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_keys_ignore_case() {
        let id = SessionId::new();
        assert_eq!(
            email_key(id, "Ann@Acme.COM"),
            format!("events/email/{id}/ann@acme.com")
        );
    }
}
