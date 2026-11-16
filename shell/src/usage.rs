//! Usage metering: what each request cost. Requests only hand their record
//! to a channel; a background task writes them in batches, adds them up per
//! hour, notes requests that hit a limit as events, and prunes old ones, so
//! no request waits for this.

use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::manifest::MAX_SECONDS;
use crate::platform::{Actor, Event, Kind, NewEvent, Outcome, PlatformDb, Request, Usage};
use crate::runtime::QUEUE_WAIT;
use crate::terminal_errorln as eprintln;

/// Requests kept per app.
pub const KEEP: u32 = 5000;
const PRUNE_EVERY: Duration = Duration::from_secs(60);
/// Requests written at once: short, so others wait little for platform.db.
const BATCH: usize = 128;
/// At most one limit-hit event per app and outcome this often.
pub const LIMIT_HIT_EVERY_MS: i64 = 10 * 60 * 1000;

/// A request's time is when it came in; its event is written when it
/// ended, after waiting in line and running as long as an app may.
const CAME_IN_BEFORE_MS: i64 = ((QUEUE_WAIT.as_secs() + MAX_SECONDS) * 1000) as i64;

/// The failed requests behind an event, the first `limit` and how many:
/// for a limit hit, those of its outcome in its 10 minutes (the log notes
/// only the first); for an automatic rollback, the failures it counted,
/// from the first to the rollback. None for other events.
pub fn behind(db: &PlatformDb, e: &Event, limit: u32) -> anyhow::Result<Option<(Vec<Request>, u32)>> {
    let (outcomes, from, to) = match (e.kind, e.outcome) {
        (Kind::LimitHit, Some(o)) => (vec![o], e.at - CAME_IN_BEFORE_MS, e.at + LIMIT_HIT_EVERY_MS),
        (Kind::AutoRolledBack, _) => {
            let counted = Outcome::ALL.into_iter().filter(|o| o.counts_toward_rollback()).collect();
            (counted, e.at - e.recovery_ms.unwrap_or(0) - CAME_IN_BEFORE_MS, e.at + 1)
        }
        _ => return Ok(None),
    };
    db.requests_like(&e.app_id, &outcomes, from, to, limit, false).map(Some)
}

/// The requests in `batch` that get a limit-hit event: the first per app
/// and outcome in 10 minutes. `last` remembers when each was written.
fn limit_hits(batch: &[Usage], last: &mut HashMap<(String, Outcome), i64>) -> Vec<usize> {
    let mut out = vec![];
    for (i, u) in batch.iter().enumerate() {
        if u.outcome.limit_hit().is_none() {
            continue;
        }
        let key = (u.app_id.clone(), u.outcome);
        if last.get(&key).is_some_and(|at| u.at - at < LIMIT_HIT_EVERY_MS) {
            continue;
        }
        last.insert(key, u.at);
        out.push(i);
    }
    out
}

#[derive(Clone)]
pub struct Recorder(mpsc::Sender<Usage>);

impl Recorder {
    /// Never waits: when the writer falls behind, records are dropped.
    pub fn record(&self, usage: Usage) {
        let _ = self.0.try_send(usage);
    }
}

/// Starts the writer.
pub fn start(db: PlatformDb) -> Recorder {
    let (tx, mut rx) = mpsc::channel::<Usage>(10_000);
    let writer = db.clone();
    tokio::spawn(async move {
        let mut batch = Vec::with_capacity(BATCH);
        let mut last = HashMap::new();
        while rx.recv_many(&mut batch, BATCH).await > 0 {
            let (db, rows) = (writer.clone(), std::mem::take(&mut batch));
            let hits = limit_hits(&rows, &mut last);
            let write = move || {
                db.record(&rows)?;
                for u in hits.iter().map(|i| &rows[*i]) {
                    let message = format!("{} on {}", u.outcome.limit_hit().unwrap_or(u.outcome.as_str()), u.route);
                    let new = NewEvent::new(&u.app_id, &u.label, Kind::LimitHit, Actor::Shell, &message);
                    db.event(&NewEvent { reason: u.error.as_deref(), risk: u.risk, outcome: Some(u.outcome), ..new })?;
                }
                anyhow::Ok(())
            };
            match tokio::task::spawn_blocking(write).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("usage: could not write: {e:#}"),
                Err(e) => eprintln!("usage: {e}"),
            }
        }
    });
    tokio::spawn(async move {
        let mut every = tokio::time::interval(PRUNE_EVERY);
        loop {
            every.tick().await;
            let db = db.clone();
            if let Ok(Err(e)) = tokio::task::spawn_blocking(move || db.prune(KEEP)).await {
                eprintln!("usage: could not prune: {e:#}");
            }
        }
    });
    Recorder(tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    use Outcome::*;

    fn u(app: &str, outcome: Outcome, at: i64) -> Usage {
        Usage {
            at,
            app_id: app.into(),
            label: "l".into(),
            route: "GET /".into(),
            outcome,
            status: 503,
            wall_us: 0,
            queue_us: 0,
            fuel: 0,
            io: 0,
            runs: 1,
            error: None,
            risk: None,
        }
    }

    /// The time a request may take in line and running is behind the
    /// event of a limit hit.
    #[test]
    fn requests_came_in_up_to_40_s_before_their_event() {
        assert_eq!(CAME_IN_BEFORE_MS, 40_000);
    }

    /// One event per app and kind in 10 minutes, across batches.
    #[test]
    fn limit_hits_are_written_at_most_every_10_minutes() {
        let mut last = HashMap::new();
        let min = 60_000;
        let first = [u("a", Timeout, 0), u("a", Timeout, min), u("a", Crashed, min), u("b", Timeout, min), u("a", Busy, min), u("a", Ok, min)];
        assert_eq!(limit_hits(&first, &mut last), vec![0, 2, 3]);
        assert_eq!(limit_hits(&[u("a", Timeout, 9 * min)], &mut last), Vec::<usize>::new());
        assert_eq!(limit_hits(&[u("a", Timeout, 10 * min)], &mut last), vec![0]);
    }

    fn event(kind: Kind, at: i64, message: &str, recovery_ms: Option<i64>, outcome: Option<Outcome>) -> Event {
        Event {
            id: 1,
            at,
            app_id: "a".into(),
            label: "l".into(),
            kind,
            severity: crate::platform::Severity::Medium,
            actor: Actor::Shell,
            message: message.into(),
            reason: None,
            risk: None,
            recovery_ms,
            outcome,
            before: None,
            person: None,
        }
    }

    /// A limit hit stands for its outcome's requests in its 10 minutes; an
    /// automatic rollback for the failures up to it. Conflicts, busy and
    /// other apps' requests are not behind either.
    #[test]
    fn the_requests_behind_an_event() {
        let db = PlatformDb::in_memory();
        let s = 1000;
        let reqs = [
            u("a", Timeout, 100 * s),
            u("a", Crashed, 110 * s),
            u("a", Conflict, 115 * s),
            u("a", Busy, 118 * s),
            u("b", Timeout, 119 * s),
            u("a", Timeout, 120 * s),
            u("a", Timeout, 130 * s),
            u("a", Timeout, 900 * s),
        ];
        db.record(&reqs).unwrap();
        let count = |e: &Event| behind(&db, e, 2).unwrap().map(|(first, n)| (first.len(), n));
        // Written when the first timeout ended; the one at 900 s is past its 10 minutes.
        assert_eq!(count(&event(Kind::LimitHit, 102 * s, "Timed out on GET /", None, Some(Timeout))), Some((2, 3)));
        // Rolled back at 131 s, 30 s after the first failure was counted.
        assert_eq!(count(&event(Kind::AutoRolledBack, 131 * s, "Rolled back automatically", Some(30 * s), None)), Some((2, 4)));
        assert_eq!(count(&event(Kind::Deployed, 131 * s, "Deployed", None, None)), None);
    }
}
