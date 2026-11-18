//! Acme's platform: the AppOS shell with Acme's CRM as a kit (crm.rs).
//! It takes appos-shell's options (`--help`), and what `appos sync
//! --shell` and `appos dev --shell` ask of a platform's binary.
//!
//!   cargo build -p acme-platform
//!   appos sync --shell <path>/target/debug/acme-platform    (in an app)
//!   appos dev --shell <path>/target/debug/acme-platform
//!
//! The CRM keeps its data in ACME_CRM_DB, a SQLite file; without it, in
//! memory, seeded with a few customers on every start.

mod crm;

#[tokio::main]
async fn main() {
    let db = std::env::var("ACME_CRM_DB").unwrap_or_else(|_| ":memory:".into());
    let crm = match crm::Crm::open(&db) {
        Ok(crm) => crm,
        Err(e) => {
            appos_shell::terminal_eprintln!("error: the CRM at {db}: {e:#}");
            std::process::exit(1);
        }
    };
    appos_shell::Platform::new().kit(crm).main().await
}
