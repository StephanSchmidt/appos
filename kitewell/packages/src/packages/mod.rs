//! Slots 1 and 3 of Kitewell's booking flow, which sales owns: the
//! package (choose) and the terms. The platform calls them at
//! /packages and /packages/terms (kitewell/src/main.rs).

mod domain;
mod web;

use crate::appos::{Module, Route};

pub const MODULE: Module = Module {
    routes: &[
        Route::get("/", web::index),
        Route::post("/choose", web::choose),
        Route::post("/offer", web::offer),
        Route::get("/terms", web::terms),
        Route::post("/terms", web::set_terms),
    ],
    menu: &[("Packages", "/")],
};
