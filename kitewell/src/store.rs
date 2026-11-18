//! Where Kitewell keeps drafts, offers, bookings and workspace members:
//! one SQLite file, or memory. Every write is harmless to repeat, because
//! a request can run again: a draft is written whole, a booking once per
//! draft, a member once per booking and email.

use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};

use crate::catalog::{self, Offer, State};
use crate::types::{Booking, Customer, Member, Quote};

/// Where workspaces and invites live; Kitewell's own app, not AppOS.
const APP: &str = "https://app.kitewell.example";

#[derive(Clone)]
pub struct Store {
    db: Arc<Mutex<Connection>>,
}

impl Store {
    /// The store in a SQLite file, or ":memory:"; offers seeded when empty.
    pub fn open(path: &str) -> anyhow::Result<Store> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS drafts (id TEXT PRIMARY KEY, state TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS offers (code TEXT PRIMARY KEY, offer TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS bookings (id TEXT PRIMARY KEY, at TEXT NOT NULL, company TEXT NOT NULL,
                 name TEXT NOT NULL, email TEXT NOT NULL, consent TEXT NOT NULL, state TEXT NOT NULL,
                 quote TEXT NOT NULL, workspace TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS members (booking TEXT NOT NULL, email TEXT NOT NULL COLLATE NOCASE,
                 name TEXT NOT NULL, at TEXT NOT NULL, invite TEXT NOT NULL, PRIMARY KEY (booking, email));",
        )?;
        let empty: bool = conn.query_row("SELECT COUNT(*) = 0 FROM offers", [], |r| r.get(0))?;
        if empty {
            for offer in catalog::seed_offers() {
                conn.execute(
                    "INSERT INTO offers VALUES (?1, ?2)",
                    params![offer.code, serde_json::to_string(&offer)?],
                )?;
            }
        }
        Ok(Store {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// A new draft with nothing chosen, under an id that is hard to guess.
    pub fn start(&self) -> anyhow::Result<(String, State)> {
        let (id, state) = (random_id()?, State::default());
        self.save(&id, &state)?;
        Ok((id, state))
    }

    pub fn draft(&self, id: &str) -> rusqlite::Result<Option<State>> {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT state FROM drafts WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    pub fn save(&self, id: &str, state: &State) -> rusqlite::Result<()> {
        let json = serde_json::to_string(state).expect("a state is JSON");
        self.conn().execute(
            "INSERT INTO drafts VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET state = ?2",
            params![id, json],
        )?;
        Ok(())
    }

    /// An offer by its code, any case.
    pub fn offer(&self, code: &str) -> rusqlite::Result<Option<Offer>> {
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT offer FROM offers WHERE code = ?1 COLLATE NOCASE",
                [code.trim()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// Books a draft, once: a second time for the same draft changes
    /// nothing. The booking gets its workspace, with the customer as its
    /// first member.
    pub fn book(
        &self,
        id: &str,
        customer: &Customer,
        consent: &str,
        state: &State,
        quote: &Quote,
        at: &str,
    ) -> anyhow::Result<()> {
        let workspace = format!("{APP}/w/{}-{}", slug(&customer.company), &random_id()?[..6]);
        let conn = self.conn();
        let added = conn.execute(
            "INSERT OR IGNORE INTO bookings VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                at,
                customer.company,
                customer.name,
                customer.email,
                consent,
                serde_json::to_string(state)?,
                serde_json::to_string(quote)?,
                workspace
            ],
        )?;
        if added == 1 {
            conn.execute(
                "INSERT OR IGNORE INTO members VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, customer.email, customer.name, at, invite()?],
            )?;
        }
        Ok(())
    }

    pub fn booking(&self, id: &str) -> rusqlite::Result<Option<Booking>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT company, state, quote, workspace FROM bookings WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((company, state, quote, workspace)) = row else {
            return Ok(None);
        };
        let state: State = serde_json::from_str(&state).unwrap_or_default();
        let mut stmt = conn.prepare(
            "SELECT name, email, invite FROM members WHERE booking = ?1 ORDER BY at, rowid",
        )?;
        let members = stmt
            .query_map([id], |r| {
                Ok(Member {
                    name: r.get(0)?,
                    email: r.get(1)?,
                    invite: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(Booking {
            id: id.to_string(),
            company,
            package: state.package.clone(),
            users: state.users,
            workspace,
            members,
            project: state.template.as_deref().and_then(catalog::template_name),
            quote: serde_json::from_str(&quote).unwrap_or_default(),
        }))
    }

    /// Adds a member to a booking and answers with them, invite link and
    /// all. An email that is in already answers with that member, so a
    /// repeat is harmless. The reason in words when they can't be added.
    pub fn add_member(
        &self,
        booking: &str,
        name: &str,
        email: &str,
        at: &str,
    ) -> anyhow::Result<Result<Member, String>> {
        let (name, email) = (name.trim(), email.trim());
        let Some(b) = self.booking(booking)? else {
            return Ok(Err("There is no such booking".into()));
        };
        if let Some(m) = b
            .members
            .iter()
            .find(|m| m.email.eq_ignore_ascii_case(email))
        {
            return Ok(Ok(m.clone()));
        }
        if name.is_empty() {
            return Ok(Err("Their name is missing".into()));
        }
        if !looks_like_email(email) {
            return Ok(Err(format!("{email} doesn't look like an email address")));
        }
        if b.members.len() >= b.quote.billed_users as usize {
            return Ok(Err(format!("All {} seats are taken", b.quote.billed_users)));
        }
        let member = Member {
            name: name.into(),
            email: email.into(),
            invite: invite()?,
        };
        self.conn().execute(
            "INSERT OR IGNORE INTO members VALUES (?1, ?2, ?3, ?4, ?5)",
            params![booking, member.email, member.name, at, member.invite],
        )?;
        Ok(Ok(member))
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub fn looks_like_email(email: &str) -> bool {
    let email = email.trim();
    email.split_once('@').is_some_and(|(user, domain)| {
        !user.is_empty() && domain.contains('.') && !email.contains(' ')
    })
}

/// 128 random bits, as 32 hex digits.
fn random_id() -> anyhow::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn invite() -> anyhow::Result<String> {
    Ok(format!("{APP}/join/{}", random_id()?))
}

/// "Acme Analytics" as "acme-analytics".
fn slug(text: &str) -> String {
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        "team".into()
    } else {
        words.join("-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Plan;

    fn customer() -> Customer {
        Customer {
            company: "Acme Analytics".into(),
            name: "Ann Lee".into(),
            email: "ann@acme.example".into(),
        }
    }

    #[test]
    fn drafts_offers_and_a_booking_once() {
        let store = Store::open(":memory:").unwrap();
        let (id, mut state) = store.start().unwrap();
        assert_eq!(id.len(), 32);
        state.plan = Some(Plan::Team);
        state.users = 4;
        store.save(&id, &state).unwrap();
        assert_eq!(store.draft(&id).unwrap(), Some(state.clone()));
        assert_eq!(
            store.offer("kw-acme-2026").unwrap().map(|o| o.users),
            Some(120)
        );

        let quote = Quote {
            billed_users: 3,
            ..Quote::default()
        };
        store
            .book(
                &id,
                &customer(),
                "I agree.",
                &state,
                &quote,
                "2026-11-18T10:00:00Z",
            )
            .unwrap();
        let first = store.booking(&id).unwrap().unwrap();
        store
            .book(
                &id,
                &Customer {
                    company: "Other".into(),
                    ..customer()
                },
                "I agree.",
                &state,
                &quote,
                "2026-11-18T10:00:01Z",
            )
            .unwrap();
        let booking = store.booking(&id).unwrap().unwrap();
        assert_eq!(booking, first, "the first one stands");
        assert!(
            booking
                .workspace
                .starts_with("https://app.kitewell.example/w/acme-analytics-")
        );
        assert_eq!(booking.members.len(), 1);
    }

    #[test]
    fn a_member_once_per_email_and_no_more_than_the_seats() {
        let store = Store::open(":memory:").unwrap();
        let (id, state) = store.start().unwrap();
        let quote = Quote {
            billed_users: 3,
            ..Quote::default()
        };
        store
            .book(
                &id,
                &customer(),
                "I agree.",
                &state,
                &quote,
                "2026-11-18T10:00:00Z",
            )
            .unwrap();
        let at = "2026-11-18T10:01:00Z";

        let tom = store
            .add_member(&id, "Tom", "tom@acme.example", at)
            .unwrap()
            .unwrap();
        assert!(tom.invite.starts_with("https://app.kitewell.example/join/"));
        let again = store
            .add_member(&id, "Tom again", " TOM@acme.example ", at)
            .unwrap()
            .unwrap();
        assert_eq!(again, tom, "a repeat answers with the same member");
        assert_eq!(
            store.add_member(&id, "Eve", "not an email", at).unwrap(),
            Err("not an email doesn't look like an email address".into())
        );
        assert_eq!(
            store.add_member(&id, " ", "eve@acme.example", at).unwrap(),
            Err("Their name is missing".into())
        );
        store
            .add_member(&id, "Eve", "eve@acme.example", at)
            .unwrap()
            .unwrap();
        assert_eq!(
            store
                .add_member(&id, "Max", "max@acme.example", at)
                .unwrap(),
            Err("All 3 seats are taken".into())
        );
        assert_eq!(
            store
                .add_member("nope", "Max", "max@acme.example", at)
                .unwrap(),
            Err("There is no such booking".into())
        );

        let names: Vec<String> = store
            .booking(&id)
            .unwrap()
            .unwrap()
            .members
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["Ann Lee", "Tom", "Eve"]);
    }
}
