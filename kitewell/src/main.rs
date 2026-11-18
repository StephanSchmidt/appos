//! Kitewell's platform: the AppOS shell with Kitewell's booking flow as a
//! kit (booking.rs) and its own pages at /book (pages.rs). Engineering owns
//! this binary, the catalog, the review and the booking; the other slots
//! of the flow are apps that sales and product own.
//! It takes appos-shell's options (`--help`), and what `appos sync
//! --shell` and `appos dev --shell` ask of a platform's binary.
//!
//!   cargo build -p kitewell
//!   appos sync --shell <path>/target/debug/kitewell    (in an app)
//!   appos dev --shell <path>/target/debug/kitewell
//!
//! Drafts, offers and bookings are in KITEWELL_DB, a SQLite file; without
//! it, in memory, with the approved offers seeded on every start.

mod booking;
mod catalog;
mod pages;
mod store;
mod types;

use booking::{Booking, Slots};
use types::Slot;

/// The slots apps own, at the path of each one's page; review is the
/// platform's own page, /book/<draft>. While an app doesn't serve, its
/// slots are the platform's plain pages at /book/<draft>/<slot>.
const SLOTS: Slots = &[
    (Slot::Choose, "/packages"),
    (Slot::Configure, "/workspace/configure"),
    (Slot::Terms, "/packages/terms"),
    (Slot::Setup, "/workspace/setup"),
];

#[tokio::main]
async fn main() {
    let db = std::env::var("KITEWELL_DB").unwrap_or_else(|_| ":memory:".into());
    let store = match store::Store::open(&db) {
        Ok(store) => store,
        Err(e) => {
            appos_shell::terminal_eprintln!("error: Kitewell's data at {db}: {e:#}");
            std::process::exit(1);
        }
    };
    appos_shell::Platform::new()
        .kit(Booking::new(store, SLOTS))
        .main()
        .await
}
