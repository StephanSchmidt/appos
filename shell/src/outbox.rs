//! Commands on their way to kits (kit.rs). A request's commit puts them in
//! the outbox, in appos.db with its kv writes (tx.rs): no commit, no
//! message. One delivery task per interface takes them out, oldest first,
//! and hands each to its kit's `run`, with the sender's context. A slow
//! kit holds up only its own messages.
//!
//! Delivery is at least once: a message the kit failed, or that took too
//! long, comes again later with the same id, after 1 s, 2 s, 4 s and so on
//! up to 5 minutes. After 10 tries it waits for an admin (the admin's
//! messages page) and is never dropped.
//!
//! The kit is found at each batch: a Wasm kit's new version gets the next
//! one. While the kit doesn't serve (off, or waiting for a kit itself),
//! its messages wait without using up tries. A task starts when its
//! interface first appears: at startup, or with a Wasm kit's deploy.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

use crate::kit::{Kit, Message};
use crate::platform::{Actor, Kind, NewEvent, PlatformDb, now_ms};
use crate::state::Live;
use crate::terminal_errorln as eprintln;
use crate::tx::{self, Db, Waiting};

/// Tries before a message waits for an admin.
pub const MAX_ATTEMPTS: u32 = 10;
/// How long `Kit::run` may take for one message.
const RUN_TIME: Duration = Duration::from_secs(30);
/// The longest wait between two tries.
const MAX_BACKOFF_MS: i64 = 5 * 60 * 1000;
/// Messages taken out at once.
const BATCH: u32 = 32;
/// How long a delivery task sleeps with nothing due; a commit that sends
/// wakes it sooner.
const IDLE: Duration = Duration::from_secs(60);
/// How long a task waits for its kit to serve again, unless woken.
const WAITING: Duration = Duration::from_secs(5);
/// The longest error kept per message.
const MAX_ERROR: usize = 2000;

/// Starts and wakes the delivery tasks.
#[derive(Clone)]
pub struct Outbox(Arc<Inner>);

struct Inner {
    db: Db,
    platform: PlatformDb,
    live: Live,
    /// By interface.
    tasks: Mutex<HashMap<String, Arc<Notify>>>,
}

impl Outbox {
    /// A request committed messages, a kit went live or was turned on:
    /// tasks start for new interfaces, and every task looks at once.
    pub fn wake(&self) {
        let mut tasks = self.0.tasks.lock();
        for interface in self.0.live.kits().interfaces() {
            tasks.entry(interface.clone()).or_insert_with(|| {
                let wake = Arc::new(Notify::new());
                let to = Deliver { interface, db: self.0.db.clone(), platform: self.0.platform.clone(), live: self.0.live.clone() };
                tokio::spawn(deliver(to, wake.clone()));
                wake
            });
        }
        for task in tasks.values() {
            task.notify_one();
        }
    }
}

/// Starts one delivery task per interface; in a Tokio runtime. A message
/// that waits for an admin is logged in `platform`. `live` has the kits,
/// and tells them which apps serve.
pub fn start(db: Db, platform: PlatformDb, live: Live) -> Outbox {
    let outbox = Outbox(Arc::new(Inner { db, platform, live, tasks: Mutex::default() }));
    outbox.wake();
    outbox
}

/// Where one delivery task delivers, and what it needs.
struct Deliver {
    interface: String,
    db: Db,
    platform: PlatformDb,
    live: Live,
}

impl Deliver {
    /// The interface's kit, if it serves; else what it waits for.
    fn kit(&self) -> Result<Arc<dyn Kit>, String> {
        let kits = self.live.kits();
        let Some((kit, grants)) = kits.kit_of(&self.interface) else { return Err("its kit".into()) };
        match self.live.kit_waits(&kits, &grants) {
            Some(kit) => Err(kit),
            None => Ok(kit),
        }
    }
}

async fn deliver(to: Deliver, wake: Arc<Notify>) {
    let (db, interface) = (&to.db, &to.interface);
    loop {
        if to.kit().is_err() {
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(WAITING) => {}
            }
            continue;
        }
        let batch = {
            let (db, interface) = (db.clone(), interface.clone());
            tokio::task::spawn_blocking(move || tx::due(&db, &interface, now_ms(), BATCH)).await
        };
        match batch {
            Ok(Ok(batch)) if !batch.is_empty() => {
                for message in batch {
                    // Its kit may have gone off while the batch was read.
                    let Ok(kit) = to.kit() else { break };
                    if let Err(e) = try_one(&to, kit, message).await {
                        eprintln!("{interface}: the outbox could not be updated: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
                continue;
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("{interface}: the outbox could not be read: {e}"),
            Err(e) => eprintln!("{interface}: the outbox could not be read: {e}"),
        }
        let next = tx::next_due(db, interface).ok().flatten();
        let wait = next.map_or(IDLE, |due| Duration::from_millis((due - now_ms()).max(0) as u64).min(IDLE));
        tokio::select! {
            _ = wake.notified() => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// Runs one message in its own task (a kit that panics fails the message,
/// not delivery), then takes it out or keeps it for later.
async fn try_one(to: &Deliver, kit: Arc<dyn Kit>, waiting: Waiting) -> rusqlite::Result<()> {
    let (id, attempts, function, app) = (waiting.id.clone(), waiting.attempts, waiting.function.clone(), waiting.app.clone());
    let (db, live) = (&to.db, to.live.clone());
    let run = tokio::spawn(async move {
        let (cx, msg) = Message::of(waiting, RUN_TIME, live);
        kit.run(cx, msg).await
    });
    let abort = run.abort_handle();
    let result = match tokio::time::timeout(RUN_TIME, run).await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => Err(anyhow::anyhow!("the kit crashed: {e}")),
        Err(_) => {
            abort.abort();
            Err(anyhow::anyhow!("took longer than {} s", RUN_TIME.as_secs()))
        }
    };
    match result {
        Ok(()) => tx::delivered(db, &id),
        Err(e) => {
            let attempts = attempts + 1;
            let mut error = format!("{e:#}");
            error.truncate(error.floor_char_boundary(MAX_ERROR));
            let due = (attempts < MAX_ATTEMPTS).then(|| now_ms() + backoff(attempts));
            tx::failed(db, &id, attempts, due, &error)?;
            if due.is_none() {
                eprintln!("{function} from {app}: failed {attempts} times, waits for an admin: {error}");
                let message = format!("{function} failed {attempts} times and waits for an admin");
                let logged = to
                    .platform
                    .run(move |p| p.event(&NewEvent { reason: Some(&error), ..NewEvent::new(&app, "", Kind::MessageFailed, Actor::Shell, &message) }))
                    .await;
                if let Err(e) = logged {
                    eprintln!("could not log it: {e:#}");
                }
            }
            Ok(())
        }
    }
}

/// The wait after `attempts` failed tries: 1 s, 2 s, 4 s, ... 5 minutes.
fn backoff(attempts: u32) -> i64 {
    (1000i64 << attempts.saturating_sub(1).min(20)).min(MAX_BACKOFF_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Context, Grant, KitLinker, Run};

    /// A kit whose every command fails, or panics.
    struct Down {
        panics: bool,
    }

    impl Kit for Down {
        fn wit(&self) -> Vec<(String, String)> {
            vec![]
        }
        fn grants(&self) -> Vec<Grant> {
            vec![]
        }
        fn link(&self, _: &mut KitLinker<'_>) -> anyhow::Result<()> {
            Ok(())
        }
        fn run(&self, _: Context, _: Message) -> Run<'_> {
            let panics = self.panics;
            Box::pin(async move {
                assert!(!panics, "the kit's own bug");
                anyhow::bail!("the CRM is down")
            })
        }
    }

    fn sent(db: &Db) -> Waiting {
        let mut t = tx::Tx::default();
        let args = r#"["web"]"#.into();
        let (interface, function, about) = ("acme:crm/leads".into(), "acme:crm/leads#add-lead".into(), "{}".into());
        t.send(tx::Outgoing { id: "m1".into(), interface, function, args, about });
        assert!(t.commit(db, "webinars").unwrap());
        tx::outbox(db).unwrap().remove(0)
    }

    #[tokio::test]
    async fn a_failed_command_is_tried_again_then_waits_for_an_admin() {
        let (db, platform) = (tx::in_memory(), PlatformDb::in_memory());
        sent(&db);
        let to = Deliver { interface: "acme:crm/leads".into(), db: db.clone(), platform: platform.clone(), live: Live::default() };
        for n in 1..=MAX_ATTEMPTS {
            try_one(&to, Arc::new(Down { panics: false }), tx::outbox(&db).unwrap().remove(0)).await.unwrap();
            let w = tx::outbox(&db).unwrap().remove(0);
            assert_eq!((w.attempts, w.error.as_deref(), w.due.is_none()), (n, Some("the CRM is down"), n == MAX_ATTEMPTS));
        }
        let events = platform.events(Some("webinars"), 10).unwrap();
        assert_eq!(events.iter().map(|e| (e.kind, e.reason.as_deref())).collect::<Vec<_>>(), [(Kind::MessageFailed, Some("the CRM is down"))]);

        let w = tx::retry(&db, "m1", now_ms()).unwrap().expect("it waits for an admin");
        assert_eq!(w.function, "acme:crm/leads#add-lead");
        let w = tx::outbox(&db).unwrap().remove(0);
        assert_eq!((w.attempts, w.due.is_some()), (0, true));
        assert!(tx::retry(&db, "m1", now_ms()).unwrap().is_none(), "it doesn't wait for an admin now");
    }

    #[tokio::test]
    async fn a_kit_that_panics_fails_the_message_not_delivery() {
        let (db, platform) = (tx::in_memory(), PlatformDb::in_memory());
        let to = Deliver { interface: "acme:crm/leads".into(), db: db.clone(), platform, live: Live::default() };
        try_one(&to, Arc::new(Down { panics: true }), sent(&db)).await.unwrap();
        let w = tx::outbox(&db).unwrap().remove(0);
        assert!(w.error.as_deref().is_some_and(|e| e.starts_with("the kit crashed")), "{:?}", w.error);
        assert_eq!(w.attempts, 1);
    }

    #[test]
    fn tries_again_later_and_later() {
        assert_eq!([1, 2, 3, 9, 10, 40].map(backoff), [1000, 2000, 4000, 256_000, 300_000, 300_000]);
    }
}
