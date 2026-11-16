//! Kitewell's discount desk: an AppOS app. `appos guide` explains the shell
//! and its calls.
//!
//! One module, desk, with the usual layers:
//!   mod.rs              its routes and menu entry
//!   web.rs              handlers: parse the input (422 and the message if
//!                       it doesn't parse), call usecases with the parsed
//!                       values, turn their results and errors into the page
//!   views.rs            the pages: template structs with every text
//!                       formatted, so the templates only place it
//!   <name>_usecase.rs   one usecase per file: <name>_u takes its IO as
//!                       parameters (tests pass fakes), <name>_uc wires it
//!                       to db.rs
//!   db.rs               the IO: every kv call, with the keys
//!   domain.rs           the terms, the numbers, the approval policy and how
//!                       a request moves; also the checks of the form
//!   people.rs, time.rs, money.rs, stats.rs, examples.rs
//!                       plain Rust the domain and the pages use
//! Each file tests its layer at the bottom; cargo test runs them all,
//! without the shell.

mod appos;
mod desk;

use appos::{App, Module, Request, page};

const MODULES: &[Module] = &[desk::MODULE];

struct DiscountsApp;

impl App for DiscountsApp {
    fn init() {
        appos::register(MODULES);
    }

    fn start(handler: String, req: Request) {
        desk::ensure_examples(&req);
        appos::dispatch(MODULES, &handler, &req);
        page::done();
    }
}

appos::export!(DiscountsApp with_types_in appos);
