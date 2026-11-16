//! Kitewell webinars: an AppOS app (`appos guide` explains the shell).
//!
//! One module, events, in src/events/. Its layers:
//!   mod.rs              routes and the menu entry
//!   web.rs, team.rs     handlers: the public pages and the events team's
//!                       pages. They parse the input (422 and the message if
//!                       it doesn't parse), call usecases, and turn their
//!                       results and errors into the page.
//!   <name>_usecase.rs   the logic; IO comes in as function parameters, so
//!                       tests pass fakes. `x_uc` wires `x_u` to db.rs.
//!   db.rs               every kv call, with the keys
//!   domain.rs           sessions, registrations, the seat and waitlist
//!                       rules, and the checks of what people type
//!   clock.rs            time zones (Berlin, London, New York); dates and
//!                       times themselves are appos::time
//!   view.rs, export.rs  how a session reads; calendar links, .ics, CSV
//! Each file tests its layer at the bottom; cargo test runs them all.

mod appos;
mod events;

use appos::{App, Module, Request, page};

const MODULES: &[Module] = &[events::MODULE];

struct WebinarsApp;

impl App for WebinarsApp {
    fn init() {
        appos::register(MODULES);
    }

    fn start(handler: String, req: Request) {
        appos::dispatch(MODULES, &handler, &req);
        page::done();
    }
}

appos::export!(WebinarsApp with_types_in appos);
