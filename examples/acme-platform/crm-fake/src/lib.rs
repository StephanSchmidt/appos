//! The CRM's fake, for appos dev: acme:crm/customers (../wit/crm.wit) with
//! made-up customers. Notes it keeps in its own directory, "/" in its
//! WASI, one per line: customer, time and text, by tabs.

wit_bindgen::generate!({
    inline: "package acme:crm-fake; world fake { export acme:crm/customers@0.1.0; }",
    path: "../wit",
});

use exports::acme::crm::customers::{Customer, Guest, Note};

const NOTES: &str = "/notes.tsv";

const CUSTOMERS: &[(&str, &str, u32, &str)] = &[
    ("c-100", "Nordwind GmbH (fake)", 480_000, "ann@acme.com"),
    ("c-101", "Bluefin Logistics (fake)", 1_250_000, "bob@acme.com"),
    ("c-102", "Kleiner Laden (fake)", 12_000, "ann@acme.com"),
];

fn customer(&(id, name, revenue, owner): &(&str, &str, u32, &str)) -> Customer {
    Customer { id: id.into(), name: name.into(), revenue, owner: owner.into() }
}

struct Fake;

impl Guest for Fake {
    fn get(id: String) -> Option<Customer> {
        CUSTOMERS.iter().find(|c| c.0 == id).map(customer)
    }

    fn search(text: String) -> Vec<Customer> {
        let text = text.to_lowercase();
        CUSTOMERS.iter().filter(|c| c.1.to_lowercase().contains(&text)).map(customer).collect()
    }

    fn notes(id: String) -> Vec<Note> {
        let all = std::fs::read_to_string(NOTES).unwrap_or_default();
        let mut notes: Vec<Note> = all
            .lines()
            .filter_map(|line| match line.splitn(3, '\t').collect::<Vec<_>>()[..] {
                [of, at, text] if of == id => Some(Note { at: at.into(), by: "appos dev".into(), text: text.into() }),
                _ => None,
            })
            .collect();
        notes.reverse();
        notes
    }

    fn add_note(id: String, text: String) {
        use std::io::Write;
        let line = format!("{id}\t{}\t{}\n", now(), text.replace(['\t', '\n'], " "));
        let file = std::fs::OpenOptions::new().create(true).append(true).open(NOTES);
        if let Err(e) = file.and_then(|mut f| f.write_all(line.as_bytes())) {
            eprintln!("the fake could not keep the note: {e}");
        }
    }
}

/// Now, like 2026-11-18T09:30:00Z.
fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

export!(Fake);
