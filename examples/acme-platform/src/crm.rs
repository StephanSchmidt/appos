//! Acme's CRM as a kit: apps look up customers and their notes (grant
//! crm-read) and add notes (crm-write), through acme:crm/customers
//! (wit/crm.wit). AppOS checks the grant, the policy and the request's
//! budget before any of this runs.

use std::sync::{Arc, Mutex};

use appos_shell::kit::{ComponentType, Context, Grant, Kit, KitLinker, Lift, Lower, Message, Run};
use rusqlite::{Connection, OptionalExtension, params};

const CUSTOMERS: &str = "acme:crm/customers@0.1.0";
const ADD_NOTE: &str = "acme:crm/customers#add-note";
/// At most this many customers per search.
const SEARCH_LIMIT: u32 = 20;

#[derive(Clone)]
pub struct Crm {
    db: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct Customer {
    id: String,
    name: String,
    revenue: u32,
    owner: String,
}

#[derive(Debug, Clone, PartialEq, ComponentType, Lift, Lower)]
#[component(record)]
pub struct Note {
    at: String,
    by: String,
    text: String,
}

const SEED: &[(&str, &str, u32, &str)] = &[
    ("c-100", "Nordwind GmbH", 480_000, "ann@acme.com"),
    ("c-101", "Bluefin Logistics", 1_250_000, "bob@acme.com"),
    ("c-102", "Kleiner Laden", 12_000, "ann@acme.com"),
    ("c-103", "Polar Foods", 95_000, "eve@acme.com"),
    ("c-104", "Nordlicht Media", 230_000, "bob@acme.com"),
];

impl Crm {
    /// The CRM in a SQLite file, or ":memory:"; seeded when empty.
    pub fn open(path: &str) -> anyhow::Result<Crm> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS customers (id TEXT PRIMARY KEY, name TEXT NOT NULL, revenue INTEGER NOT NULL, owner TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS notes (message TEXT PRIMARY KEY, customer TEXT NOT NULL, at TEXT NOT NULL, by TEXT NOT NULL, text TEXT NOT NULL);",
        )?;
        let empty: bool = conn.query_row("SELECT COUNT(*) = 0 FROM customers", [], |r| r.get(0))?;
        if empty {
            for (id, name, revenue, owner) in SEED {
                conn.execute(
                    "INSERT INTO customers VALUES (?1, ?2, ?3, ?4)",
                    params![id, name, revenue, owner],
                )?;
            }
        }
        Ok(Crm {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    fn get(&self, id: &str) -> rusqlite::Result<Option<Customer>> {
        self.conn()
            .query_row(
                "SELECT id, name, revenue, owner FROM customers WHERE id = ?1",
                [id],
                customer,
            )
            .optional()
    }

    fn search(&self, text: &str) -> rusqlite::Result<Vec<Customer>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name, revenue, owner FROM customers WHERE instr(lower(name), lower(?1)) > 0 ORDER BY name LIMIT ?2")?;
        stmt.query_map(params![text, SEARCH_LIMIT], customer)?
            .collect()
    }

    fn notes(&self, id: &str) -> rusqlite::Result<Vec<Note>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT at, by, text FROM notes WHERE customer = ?1 ORDER BY at DESC, rowid DESC",
        )?;
        stmt.query_map([id], |r| {
            Ok(Note {
                at: r.get(0)?,
                by: r.get(1)?,
                text: r.get(2)?,
            })
        })?
        .collect()
    }

    /// Keyed by the message: a command delivered twice adds one note.
    fn add_note(&self, message: &str, id: &str, by: &str, text: &str) -> anyhow::Result<()> {
        anyhow::ensure!(self.get(id)?.is_some(), "no customer {id}");
        let at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        self.conn().execute(
            "INSERT OR IGNORE INTO notes VALUES (?1, ?2, ?3, ?4, ?5)",
            params![message, id, at, by, text],
        )?;
        Ok(())
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn customer(r: &rusqlite::Row) -> rusqlite::Result<Customer> {
    Ok(Customer {
        id: r.get(0)?,
        name: r.get(1)?,
        revenue: r.get(2)?,
        owner: r.get(3)?,
    })
}

impl Kit for Crm {
    fn wit(&self) -> Vec<(String, String)> {
        vec![("crm.wit".into(), include_str!("../wit/crm.wit").into())]
    }

    fn grants(&self) -> Vec<Grant> {
        let f = |name: &str| format!("acme:crm/customers#{name}");
        let read = [f("get"), f("search"), f("notes")];
        vec![
            Grant::new(
                "crm-read",
                "Look up customers and their notes: customers::get, search, notes",
                &read.each_ref().map(String::as_str),
                1,
            ),
            Grant::new(
                "crm-write",
                "Add notes to customers: customers::add-note",
                &[ADD_NOTE],
                5,
            ),
        ]
    }

    fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
        let crm = self.clone();
        linker.func(CUSTOMERS, "get", move |_, (id,): (String,)| {
            let found = crm.get(&id);
            Box::pin(async move { Ok((found?,)) })
        })?;
        let crm = self.clone();
        linker.func(CUSTOMERS, "search", move |_, (text,): (String,)| {
            let found = crm.search(&text);
            Box::pin(async move { Ok((found?,)) })
        })?;
        let crm = self.clone();
        linker.func(CUSTOMERS, "notes", move |_, (id,): (String,)| {
            let found = crm.notes(&id);
            Box::pin(async move { Ok((found?,)) })
        })?;
        linker.command::<(String, String)>(CUSTOMERS, "add-note")
    }

    fn run(&self, cx: Context, msg: Message) -> Run<'_> {
        Box::pin(async move {
            anyhow::ensure!(msg.function == ADD_NOTE, "no command {}", msg.function);
            let (id, text): (String, String) = msg.args()?;
            let by = cx
                .user()
                .map_or_else(|| format!("app {}", cx.app()), |u| u.email.clone());
            self.add_note(&msg.id, &id, &by, &text)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customers_and_their_notes() {
        let crm = Crm::open(":memory:").unwrap();
        assert_eq!(
            crm.get("c-100").unwrap().map(|c| c.name),
            Some("Nordwind GmbH".into())
        );
        assert_eq!(crm.get("nobody").unwrap(), None);
        let names: Vec<String> = crm
            .search("NORD")
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Nordlicht Media", "Nordwind GmbH"]);

        crm.add_note("m1", "c-100", "ann@acme.com", "10% approved")
            .unwrap();
        crm.add_note("m1", "c-100", "ann@acme.com", "10% approved")
            .unwrap();
        let notes = crm.notes("c-100").unwrap();
        assert_eq!(
            notes
                .iter()
                .map(|n| (n.by.as_str(), n.text.as_str()))
                .collect::<Vec<_>>(),
            [("ann@acme.com", "10% approved")],
            "once per message"
        );
        assert!(crm.add_note("m2", "nobody", "ann@acme.com", "x").is_err());
    }

    #[test]
    fn the_platform_takes_the_kit() {
        let contract = appos_shell::kit::Kits::builtin()
            .with(Crm::open(":memory:").unwrap())
            .unwrap()
            .contract();
        assert!(
            contract["appos.wit"].contains("    import acme:crm/customers@0.1.0;\n"),
            "{}",
            contract["appos.wit"]
        );
        assert!(contract["deps/acme-crm/crm.wit"].starts_with("package acme:crm@0.1.0;"));
    }
}
