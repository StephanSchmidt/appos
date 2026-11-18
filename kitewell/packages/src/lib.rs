//! Packages: the steps of Kitewell's booking that sales owns. A customer
//! picks a package for their team size, or types the code of an offer
//! sales approved, and later picks how they pay. Sales changes these
//! pages, the packages and their pitch without engineering; the prices and
//! the rules for booking are the platform's (kitewell:booking).
//!
//! The app holds three grants (app.toml): booking-read, booking-choose and
//! booking-terms. It can't touch the workspace's slot or book anything.

mod appos;
mod packages;
mod quote;

use appos::{App, Module, Request, page};

const MODULES: &[Module] = &[packages::MODULE];

struct Packages;

impl App for Packages {
    fn init() {
        appos::register(MODULES);
    }

    fn start(handler: String, req: Request) {
        appos::dispatch(MODULES, &handler, &req);
        page::done();
    }
}

appos::export!(Packages with_types_in appos);
