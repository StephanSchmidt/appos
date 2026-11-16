//! Hello world: an AppOS app. Change it into yours; `appos guide` explains
//! the shell and its calls.
//!
//! An app is made of modules, one folder each under src/. This one has a
//! single module, hello. A module owns its routes, its pages and its kv keys,
//! and every module has the same layers:
//!   mod.rs              its routes and menu entries (MODULE)
//!   web.rs              handlers: parse the input (422 and the message if
//!                       it doesn't parse), call usecases with the parsed
//!                       values, turn their results and errors (an unknown
//!                       id: 404) into the page. Errors end here.
//!   <name>_usecase.rs   one usecase per file, as a pair:
//!                         greet_u    the logic; every IO it needs comes in
//!                                    as a function parameter (add_greeting,
//!                                    log), so tests pass fakes
//!                         greet_uc   wires greet_u to the real IO in db.rs
//!                       A usecase that can fail returns its own error enum
//!                       (show_usecase.rs: ShowError::NotFound).
//!   db.rs               the IO: every kv call, with the keys
//!   domain.rs           the data and its rules, plain Rust without IO; also
//!                       the checks of what visitors type (Name::parse)
//! Each file tests its layer at the bottom; cargo test runs them all,
//! without the shell. src/appos.rs is the platform's side: leave it to
//! `appos sync`.

mod appos;
mod hello;

use appos::{App, Module, Request, page};

/// Every module of the app: a new module adds its MODULE here.
const MODULES: &[Module] = &[hello::MODULE];

struct HelloApp;

impl App for HelloApp {
    fn init() {
        appos::register(MODULES);
    }

    fn start(handler: String, req: Request) {
        appos::dispatch(MODULES, &handler, &req);
        page::done();
    }
}

appos::export!(HelloApp with_types_in appos);
