//! A request's kv transaction. Writes stay in memory until the request
//! ends, and the request's own reads see them. At the end `commit` applies
//! them all at once, unless a key the request read with kv::get has changed
//! since: then nothing is written and the request runs again (runtime.rs).
//! A request that fails writes nothing.
//!
//! The store only needs get, scan, and one atomic "check these values, then
//! apply these writes": a Postgres transaction, Redis WATCH/MULTI or a Lua
//! script would do. This demo shell uses SQLite.
//!
//! Scans are not checked at commit: a request that lists keys while another
//! adds one would otherwise run again every time.

use std::collections::BTreeMap;
use std::mem::DropGuard;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, MutexGuard};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub type Db = Arc<Mutex<Connection>>;

/// Keys one kv::scan may return.
pub const MAX_SCAN: u32 = 1000;
/// Bytes of keys and values returned by a scan. A row count alone allows
/// a thousand 1 MB values to be copied into the host before Wasm can stop it.
pub const MAX_SCAN_BYTES: usize = 4 * 1024 * 1024;

const GET: &str = "SELECT value FROM kv WHERE app = ?1 AND key = ?2";
const SET: &str = "INSERT INTO kv (app, key, value) VALUES (?1, ?2, ?3) ON CONFLICT (app, key) DO UPDATE SET value = ?3";
const DELETE: &str = "DELETE FROM kv WHERE app = ?1 AND key = ?2";

#[derive(Default)]
pub struct Tx {
    deadline: Option<Instant>,
    /// What kv::get read from the store, per key: checked at commit.
    reads: BTreeMap<String, Option<String>>,
    /// Writes not yet applied: Some sets the key, None deletes it.
    writes: BTreeMap<String, Option<String>>,
}

fn deadline_error() -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT), Some("request deadline elapsed".into()))
}

fn check_deadline(deadline: Option<Instant>) -> rusqlite::Result<()> {
    if deadline.is_some_and(|d| Instant::now() >= d) {
        return Err(deadline_error());
    }
    Ok(())
}

fn before_statement(conn: &Connection, deadline: Option<Instant>) -> rusqlite::Result<()> {
    check_deadline(deadline)?;
    if let Some(deadline) = deadline {
        conn.busy_timeout(deadline.saturating_duration_since(Instant::now()))?;
    }
    Ok(())
}

impl Tx {
    pub fn with_deadline(deadline: Instant) -> Self {
        Self { deadline: Some(deadline), ..Self::default() }
    }

    fn access<'a>(&self, db: &'a Db) -> rusqlite::Result<DropGuard<MutexGuard<'a, Connection>, impl FnOnce(MutexGuard<'a, Connection>)>> {
        let conn = match self.deadline {
            Some(deadline) => db.try_lock_until(deadline).ok_or_else(deadline_error)?,
            None => db.lock(),
        };
        // Deadline hooks belong to this operation. Reset them before the
        // lock is released, including on setup errors and unwinding.
        let timed = self.deadline.is_some();
        let mut access = DropGuard::new(conn, move |mut conn| {
            if timed {
                let _ = conn.progress_handler(0, None::<fn() -> bool>);
                let _ = conn.busy_timeout(Duration::from_secs(5));
            }
        });
        before_statement(&access, self.deadline)?;
        if let Some(deadline) = self.deadline {
            access.progress_handler(100, Some(move || Instant::now() >= deadline))?;
        }
        Ok(access)
    }

    pub fn get(&mut self, db: &Db, app: &str, key: &str) -> rusqlite::Result<Option<String>> {
        check_deadline(self.deadline)?;
        if let Some(value) = self.writes.get(key).or_else(|| self.reads.get(key)) {
            return Ok(value.clone());
        }
        let value = self.access(db)?.query_row(GET, params![app, key], |r| r.get(0)).optional()?;
        self.reads.insert(key.to_string(), value.clone());
        Ok(value)
    }

    pub fn set(&mut self, key: String, value: String) {
        self.writes.insert(key, Some(value));
    }

    pub fn delete(&mut self, key: String) {
        self.writes.insert(key, None);
    }

    /// Keys starting with `prefix` past `after`, in key order (or reverse),
    /// at most `limit`: the store's keys with this request's writes applied.
    pub fn scan(&self, db: &Db, app: &str, prefix: &str, after: Option<&str>, limit: u32, reverse: bool) -> rusqlite::Result<Vec<(String, String)>> {
        check_deadline(self.deadline)?;
        let limit = limit.min(MAX_SCAN) as usize;
        if limit == 0 {
            return Ok(vec![]);
        }
        let past = |key: &str| after.is_none_or(|a| if reverse { key < a } else { key > a });
        let mut mine: Vec<_> = self.writes.iter().filter(|(k, _)| k.starts_with(prefix) && past(k)).collect();

        // Ask the store for enough rows that deleting some still leaves
        // `limit`. Merge in order and stop before reading or cloning rows
        // beyond that result, so their size cannot exhaust host memory.
        let deleted = mine.iter().filter(|(_, v)| v.is_none()).count();
        if reverse {
            mine.reverse();
        }
        let mut mine = mine.into_iter().peekable();
        // Binary text order agrees with Rust's UTF-8 string order. Seek
        // directly into the prefix range using the (app, key) index;
        // substr(key, ...) would visit every unrelated key in this app.
        let end = prefix_end(prefix);
        let upper = match (end.as_deref(), after.filter(|_| reverse)) {
            (Some(end), Some(after)) => Some(end.min(after)),
            (end, after) => end.or(after),
        };
        let lower = if !reverse && after.is_some_and(|a| a >= prefix) { "key > ?4" } else { "key >= ?2" };
        let upper_bound = if upper.is_some() { "AND key < ?3" } else { "" };
        let order = if reverse { "DESC" } else { "ASC" };
        let sql = format!("SELECT key, value FROM kv WHERE app = ?1 AND {lower} {upper_bound} ORDER BY key {order} LIMIT ?5");
        let conn = self.access(db)?;
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut rows = stmt.query(params![app, prefix, upper, after, (limit + deleted) as i64])?;
        let next =
            |rows: &mut rusqlite::Rows<'_>| -> rusqlite::Result<Option<(String, String)>> { rows.next()?.map(|r| Ok((r.get(0)?, r.get(1)?))).transpose() };
        let mut stored = next(&mut rows)?;
        let (mut out, mut bytes) = (Vec::new(), 0);
        while out.len() < limit {
            let own_first = match (mine.peek(), stored.as_ref()) {
                (Some((key, _)), Some((db_key, _))) => {
                    if reverse {
                        *key >= db_key
                    } else {
                        *key <= db_key
                    }
                }
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if own_first {
                let (key, value) = mine.next().expect("own_first requires a pending write");
                if stored.as_ref().is_some_and(|(k, _)| k == key) {
                    stored = next(&mut rows)?;
                }
                if let Some(value) = value {
                    scan_bytes(&mut bytes, key, value)?;
                    out.push((key.clone(), value.clone()));
                }
            } else {
                let (key, value) = stored.take().expect("a stored row is pending");
                scan_bytes(&mut bytes, &key, &value)?;
                out.push((key, value));
                // Do not copy another value once the result is complete.
                if out.len() < limit {
                    stored = next(&mut rows)?;
                }
            }
        }
        Ok(out)
    }

    /// Applies the writes at once, if every key read with get still holds
    /// what was read. false: something changed, nothing was written.
    pub fn commit(&self, db: &Db, app: &str) -> rusqlite::Result<bool> {
        check_deadline(self.deadline)?;
        if self.writes.is_empty() {
            return Ok(true);
        }
        let mut conn = self.access(db)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (key, seen) in &self.reads {
            before_statement(&tx, self.deadline)?;
            let now: Option<String> = tx.query_row(GET, params![app, key], |r| r.get(0)).optional()?;
            if now != *seen {
                return Ok(false);
            }
        }
        for (key, value) in &self.writes {
            before_statement(&tx, self.deadline)?;
            match value {
                Some(v) => tx.execute(SET, params![app, key, v])?,
                None => tx.execute(DELETE, params![app, key])?,
            };
        }
        before_statement(&tx, self.deadline)?;
        tx.commit()?;
        Ok(true)
    }
}

/// The least valid UTF-8 string greater than every string with `prefix`.
/// None means the range has no upper bound (empty or all-maximal scalars).
fn prefix_end(prefix: &str) -> Option<String> {
    for (i, c) in prefix.char_indices().rev() {
        let next = match c {
            '\u{10ffff}' => continue,
            '\u{d7ff}' => '\u{e000}', // Skip Unicode's surrogate range.
            c => char::from_u32(c as u32 + 1).expect("incremented scalar stays valid"),
        };
        let mut end = prefix[..i].to_string();
        end.push(next);
        return Some(end);
    }
    None
}

fn scan_bytes(bytes: &mut usize, key: &str, value: &str) -> rusqlite::Result<()> {
    *bytes += key.len() + value.len();
    if *bytes > MAX_SCAN_BYTES {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_TOOBIG),
            Some(format!("over budget: kv::scan returns more than {} MB", MAX_SCAN_BYTES >> 20)),
        ));
    }
    Ok(())
}

pub fn open_db(path: &std::path::Path) -> anyhow::Result<Db> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    create(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn create(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS kv (app TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (app, key));
         CREATE TABLE IF NOT EXISTS examples (app TEXT NOT NULL, handler TEXT NOT NULL, request TEXT NOT NULL, PRIMARY KEY (app, handler));",
    )?;
    Ok(())
}

/// Keeps `request` (JSON) as the example of the app's route `handler`,
/// in place of the one before (see review.rs).
pub fn keep_example(db: &Db, app: &str, handler: &str, request: &str) -> rusqlite::Result<()> {
    db.lock().execute("INSERT OR REPLACE INTO examples (app, handler, request) VALUES (?1, ?2, ?3)", (app, handler, request)).map(|_| ())
}

/// The examples kept for the app, by handler.
pub fn examples(db: &Db, app: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let conn = db.lock();
    let mut stmt = conn.prepare("SELECT handler, request FROM examples WHERE app = ?1 ORDER BY handler")?;
    stmt.query_map([app], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

/// Every key of the app and its value, in key order.
pub fn entries(db: &Db, app: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let conn = db.lock();
    let mut stmt = conn.prepare("SELECT key, value FROM kv WHERE app = ?1 ORDER BY key")?;
    stmt.query_map([app], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

pub(crate) fn in_memory() -> Db {
    let conn = Connection::open_in_memory().unwrap();
    create(&conn).unwrap();
    Arc::new(Mutex::new(conn))
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "test";

    #[test]
    fn an_expired_transaction_never_commits() {
        let db = db();
        let mut tx = Tx::with_deadline(std::time::Instant::now() - std::time::Duration::from_millis(1));
        tx.set("late".into(), "must not be written".into());
        assert!(tx.commit(&db, APP).is_err());
        assert_eq!(Tx::default().get(&db, APP, "late").unwrap(), None);
    }

    #[test]
    fn unwinding_restores_deadline_hooks_before_releasing_the_connection() {
        let db = db();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tx = Tx::with_deadline(Instant::now() + Duration::from_secs(1));
            let mut conn = tx.access(&db).unwrap();
            conn.progress_handler(1, Some(|| true)).unwrap();
            panic!("the operation failed while holding the connection");
        }));
        assert!(result.is_err());
        let conn = db.lock();
        assert_eq!(conn.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, u32>(0)).unwrap(), 5000);
        // A leaked progress hook would interrupt this query immediately.
        assert_eq!(conn.query_row("SELECT 1 + 1", [], |r| r.get::<_, u32>(0)).unwrap(), 2);
    }

    fn db() -> Db {
        in_memory()
    }

    #[test]
    fn waiting_for_the_connection_obeys_the_deadline_without_late_writes() {
        let db = db();
        let guard = db.lock();
        let worker_db = db.clone();
        let (finished, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut tx = Tx::with_deadline(Instant::now() + Duration::from_millis(20));
            tx.set("late".into(), "must not be written".into());
            finished.send(tx.commit(&worker_db, APP)).unwrap();
        });
        let outcome = result.recv_timeout(Duration::from_millis(500));
        drop(guard);
        worker.join().unwrap();
        assert!(matches!(outcome, Ok(Err(_))), "a database lock kept an expired worker alive");
        assert_eq!(Tx::default().get(&db, APP, "late").unwrap(), None);
    }

    #[test]
    fn sqlite_writer_contention_obeys_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kv.db");
        let db = open_db(&path).unwrap();
        let other_writer = Connection::open(&path).unwrap();
        other_writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let mut tx = Tx::with_deadline(Instant::now() + Duration::from_millis(20));
        tx.set("late".into(), "must not be written".into());
        let started = Instant::now();
        let err = tx.commit(&db, APP).unwrap_err();
        assert!(matches!(err.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::OperationInterrupted)));
        assert!(started.elapsed() < Duration::from_secs(1), "SQLite used its default five-second busy timeout");
        other_writer.execute_batch("ROLLBACK").unwrap();
        assert_eq!(Tx::default().get(&db, APP, "late").unwrap(), None);
    }

    #[test]
    fn an_interrupted_commit_rolls_back_all_writes_and_restores_the_connection() {
        let db = db();
        db.lock()
            .execute_batch(
                "CREATE TRIGGER slow_insert BEFORE INSERT ON kv WHEN NEW.key = 'z' BEGIN
            SELECT sum(n) FROM (WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n < 1000000000) SELECT n FROM seq);
            END;",
            )
            .unwrap();
        let mut tx = Tx::with_deadline(Instant::now() + Duration::from_millis(20));
        tx.set("a".into(), "first write".into());
        tx.set("z".into(), "slow write".into());
        assert_eq!(tx.commit(&db, APP).unwrap_err().sqlite_error_code(), Some(rusqlite::ErrorCode::OperationInterrupted));
        assert_eq!(Tx::default().get(&db, APP, "a").unwrap(), None);
        assert_eq!(Tx::default().get(&db, APP, "z").unwrap(), None);
        put(&db, &[("later", "works")]);
        assert_eq!(Tx::default().get(&db, APP, "later").unwrap().as_deref(), Some("works"));
        assert_eq!(db.lock().query_row("PRAGMA busy_timeout", [], |r| r.get::<_, u32>(0)).unwrap(), 5000);
    }

    #[test]
    fn prefix_ranges_cover_unicode_extremes() {
        let db = db();
        let keys =
            ["", "a\0", "a\0x", "a\u{10ffff}", "a\u{10ffff}x", "b", "\u{d7ff}", "\u{e000}", "\u{10ffff}", "\u{10ffff}\u{10ffff}", "\u{10ffff}\u{10ffff}x"];
        for key in keys {
            put(&db, &[(key, "value")]);
        }
        let expected: BTreeMap<_, _> = keys.iter().map(|k| (k.to_string(), "value".to_string())).collect();
        for prefix in keys {
            for reverse in [false, true] {
                let mut wanted: Vec<_> = expected.iter().filter(|(key, _)| key.starts_with(prefix)).map(|(k, v)| (k.clone(), v.clone())).collect();
                if reverse {
                    wanted.reverse();
                }
                assert_eq!(Tx::default().scan(&db, APP, prefix, None, 100, reverse).unwrap(), wanted, "prefix {prefix:?}, reverse {reverse}");
            }
        }
    }

    fn put(db: &Db, pairs: &[(&str, &str)]) {
        let mut tx = Tx::default();
        for (k, v) in pairs {
            tx.set(k.to_string(), v.to_string());
        }
        assert!(tx.commit(db, APP).unwrap());
    }

    fn keys(rows: Vec<(String, String)>) -> Vec<String> {
        rows.into_iter().map(|(k, _)| k).collect()
    }

    #[test]
    fn reads_see_the_requests_own_writes() {
        let db = db();
        put(&db, &[("a", "1")]);
        let mut tx = Tx::default();
        tx.set("a".into(), "2".into());
        tx.delete("b".into());
        assert_eq!(tx.get(&db, APP, "a").unwrap().as_deref(), Some("2"));
        assert_eq!(tx.get(&db, APP, "b").unwrap(), None);
        // Nothing is written before commit.
        assert_eq!(Tx::default().get(&db, APP, "a").unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn scan_merges_the_requests_writes() {
        let db = db();
        put(&db, &[("h/1", "a"), ("h/2", "b"), ("h/3", "c"), ("x/1", "other")]);
        let mut tx = Tx::default();
        tx.set("h/4".into(), "d".into());
        tx.delete("h/3".into());
        tx.set("h/1".into(), "A".into());

        let all = tx.scan(&db, APP, "h/", None, 10, false).unwrap();
        assert_eq!(all, [("h/1".into(), "A".into()), ("h/2".into(), "b".into()), ("h/4".into(), "d".into())]);
        assert_eq!(keys(tx.scan(&db, APP, "h/", None, 2, true).unwrap()), ["h/4", "h/2"]);
        assert_eq!(keys(tx.scan(&db, APP, "h/", Some("h/1"), 1, false).unwrap()), ["h/2"]);
        assert_eq!(keys(tx.scan(&db, APP, "h/", Some("h/4"), 2, true).unwrap()), ["h/2", "h/1"]);
    }

    #[test]
    fn scan_still_fills_the_limit_after_deletes() {
        let db = db();
        put(&db, &[("h/1", ""), ("h/2", ""), ("h/3", ""), ("h/4", "")]);
        let mut tx = Tx::default();
        tx.delete("h/1".into());
        tx.delete("h/2".into());
        tx.set("h/9".into(), String::new());
        assert_eq!(keys(tx.scan(&db, APP, "h/", None, 2, false).unwrap()), ["h/3", "h/4"]);
    }

    #[test]
    fn scans_bound_the_bytes_they_return() {
        let db = db();
        let mut tx = Tx::default();
        for n in 0..20 {
            tx.set(format!("{n:02}"), "x".repeat(256 * 1024));
        }
        assert!(tx.commit(&db, APP).unwrap());
        assert!(Tx::default().scan(&db, APP, "", None, 1000, false).is_err());
        // Limits apply to the selected rows, after merging our writes.
        tx.set("00".into(), "small".into());
        assert_eq!(tx.scan(&db, APP, "", None, 1, false).unwrap(), [("00".into(), "small".into())]);
    }

    #[test]
    fn scan_prefixes_include_embedded_nuls_and_unicode() {
        let db = db();
        put(&db, &[("a\0x/1", "one"), ("a\0y/1", "other"), ("café/1", "two")]);
        let tx = Tx::default();
        assert_eq!(keys(tx.scan(&db, APP, "a\0x/", None, 10, false).unwrap()), ["a\0x/1"]);
        assert_eq!(keys(tx.scan(&db, APP, "café/", None, 10, false).unwrap()), ["café/1"]);
    }

    #[test]
    fn scans_do_not_visit_unrelated_keys() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let db = db();
        let mut seed = Tx::default();
        for n in 0..2000 {
            seed.set(format!("a/{n:04}"), "before".into());
            seed.set(format!("z/{n:04}"), "after".into());
        }
        seed.set("m/only".into(), "wanted".into());
        seed.commit(&db, APP).unwrap();
        let steps = Arc::new(AtomicUsize::new(0));
        let counted = steps.clone();
        db.lock()
            .progress_handler(
                100,
                Some(move || {
                    counted.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        for reverse in [false, true] {
            for prefix in ["m/", "m/missing"] {
                steps.store(0, Ordering::Relaxed);
                let result = Tx::default().scan(&db, APP, prefix, None, 10, reverse).unwrap();
                assert_eq!(result.len(), usize::from(prefix == "m/"));
                assert!(
                    steps.load(Ordering::Relaxed) < 10,
                    "prefix {prefix:?}, reverse {reverse}: scan executed at least {} VM steps on unrelated keys",
                    steps.load(Ordering::Relaxed) * 100
                );
            }
        }
    }

    #[test]
    fn scan_merge_matches_a_reference_map_for_pagination_and_all_write_combinations() {
        let db = db();
        let initial = [("h/1", "a"), ("h/3", "c"), ("h/5", "e"), ("other", "x")];
        put(&db, &initial);
        for mut states in 0..81 {
            let mut tx = Tx::default();
            let mut expected: BTreeMap<_, _> = initial.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            for key in ["h/1", "h/2", "h/3", "h/7"] {
                match states % 3 {
                    1 => {
                        tx.set(key.into(), "new".into());
                        expected.insert(key.into(), "new".into());
                    }
                    2 => {
                        tx.delete(key.into());
                        expected.remove(key);
                    }
                    _ => {}
                }
                states /= 3;
            }
            for reverse in [false, true] {
                for after in [None, Some("h/2"), Some("h/8")] {
                    let mut ordered: Vec<_> = expected
                        .iter()
                        .filter(|(k, _)| k.starts_with("h/") && after.is_none_or(|a| if reverse { k.as_str() < a } else { k.as_str() > a }))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    if reverse {
                        ordered.reverse();
                    }
                    for limit in 0..6 {
                        let wanted: Vec<_> = ordered.iter().take(limit as usize).cloned().collect();
                        assert_eq!(tx.scan(&db, APP, "h/", after, limit, reverse).unwrap(), wanted);
                    }
                }
            }
        }
    }

    #[test]
    fn a_changed_read_stops_the_commit() {
        let db = db();
        put(&db, &[("count", "1")]);
        let mut slow = Tx::default();
        let n: u32 = slow.get(&db, APP, "count").unwrap().unwrap().parse().unwrap();
        slow.set("count".into(), (n + 1).to_string());

        put(&db, &[("count", "5")]);
        assert!(!slow.commit(&db, APP).unwrap());
        assert_eq!(Tx::default().get(&db, APP, "count").unwrap().as_deref(), Some("5"));
    }

    #[test]
    fn a_key_created_meanwhile_stops_the_commit() {
        let db = db();
        let mut tx = Tx::default();
        assert_eq!(tx.get(&db, APP, "ann@example.com").unwrap(), None);
        tx.set("ann@example.com".into(), "registered".into());
        put(&db, &[("ann@example.com", "registered first")]);
        assert!(!tx.commit(&db, APP).unwrap());
    }

    #[test]
    fn writes_to_other_keys_and_scans_dont_conflict() {
        let db = db();
        let mut tx = Tx::default();
        tx.scan(&db, APP, "h/", None, 10, false).unwrap();
        tx.set("h/1".into(), "mine".into());
        put(&db, &[("h/2", "theirs")]);
        assert!(tx.commit(&db, APP).unwrap());
        assert_eq!(keys(Tx::default().scan(&db, APP, "h/", None, 10, false).unwrap()), ["h/1", "h/2"]);
    }
}
