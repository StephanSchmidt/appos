//! Undo: the reverse of an admin's change, worked out from its event. The
//! toast offers it for ten minutes, and only while nothing changed the same
//! thing since, so an Undo never takes back a later change.

use crate::platform::{Actor, Event, Kind, PlatformDb, limits_before};
use crate::runtime::AdminLimits;

use super::model::{MINUTE, seen_kinds};

/// How long a change can be undone from its toast.
pub(super) const UNDO_MS: i64 = 10 * MINUTE;

/// What reverses a change.
#[derive(Debug, PartialEq)]
pub(super) enum Reverse {
    Turn {
        app: String,
        on: bool,
    },
    /// Turns on what Stop all turned off.
    StartAll,
    Freeze(bool),
    /// Brings the problems a "seen" saw back to the inbox.
    Unseen {
        app: String,
    },
    Limits {
        app: String,
        to: AdminLimits,
    },
    MakeLive {
        app: String,
        label: String,
    },
}

/// The change that reverses `e` at `now`, if it can still be undone: an
/// admin made it in the last ten minutes and it is still what holds.
pub(super) fn reverse(db: &PlatformDb, e: &Event, now: i64) -> Option<Reverse> {
    if e.actor != Actor::Admin || !(e.at..=e.at + UNDO_MS).contains(&now) {
        return None;
    }
    let app = e.app_id.clone();
    // Nothing of these kinds happened to its app since.
    let newest = |kinds: &[Kind]| db.events_since(Some(&e.app_id), e.at, kinds).ok().and_then(|es| es.last().map(|x| x.id)) == Some(e.id);
    match e.kind {
        Kind::TurnedOff => {
            (db.off(&app).ok()??.id == e.id).then_some(())?;
            Some(if db.stopped_all().ok()?.contains(&app) { Reverse::StartAll } else { Reverse::Turn { app, on: true } })
        }
        // Start all has no undo: Stop all is a button of its own.
        Kind::TurnedOn if !e.message.ends_with("Start all") => {
            (newest(&[Kind::TurnedOff, Kind::TurnedOn]) && db.off(&app).ok()?.is_none()).then_some(Reverse::Turn { app, on: false })
        }
        Kind::Frozen => (db.frozen().ok()??.id == e.id).then_some(Reverse::Freeze(false)),
        Kind::Unfrozen => (newest(&[Kind::Frozen, Kind::Unfrozen]) && db.frozen().ok()?.is_none()).then_some(Reverse::Freeze(true)),
        Kind::Seen if !seen_kinds(e).is_empty() => newest(&[Kind::Seen]).then_some(Reverse::Unseen { app }),
        Kind::LimitsSet => {
            let to = limits_before(e)?;
            newest(&[Kind::LimitsSet]).then_some(Reverse::Limits { app, to })
        }
        // "Make live": the version that was live before goes live again.
        Kind::Deployed | Kind::RolledBack => {
            let label = e.before.clone()?;
            let live = db.live_of(&app).ok()??;
            (live.label == e.label && newest(&[Kind::Deployed, Kind::RolledBack, Kind::AutoRolledBack])).then_some(Reverse::MakeLive { app, label })
        }
        _ => None,
    }
}
