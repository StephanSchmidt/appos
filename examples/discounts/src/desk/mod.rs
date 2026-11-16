//! The discount desk: reps ask for discounts, the policy decides who has to
//! approve, approvers decide in order, and everything stays on record.
//! Other modules reach it only through what this file makes `pub`.

mod close_usecase;
mod db;
mod decide_usecase;
mod domain;
mod examples;
mod examples_usecase;
mod list_usecase;
mod money;
mod people;
mod resubmit_usecase;
mod show_usecase;
mod stats;
mod submit_usecase;
mod time;
mod views;
mod web;

use crate::appos::{Module, Request, Route};

/// The module's routes and its menu entry.
pub const MODULE: Module = Module {
    routes: ROUTES,
    menu: &[("Discount desk", "/")],
};

/// Who is looking is in the path: /as/<person>/...
const ROUTES: &[Route] = &[
    Route::get("/", web::pick),
    Route::get("/switch", web::switch),
    Route::get("/as/:who", web::home),
    Route::get("/as/:who/all", web::all),
    Route::get("/as/:who/new", web::new_request),
    Route::post("/as/:who/requests", web::create),
    Route::post("/as/:who/quote", web::quote),
    Route::get("/as/:who/r/:n", web::request),
    Route::post("/as/:who/r/:n/decide", web::decide),
    Route::post("/as/:who/r/:n/close", web::close),
    Route::get("/as/:who/r/:n/change", web::change_form),
    Route::post("/as/:who/r/:n/change", web::resubmit),
];

/// Puts in the example requests on the very first visit.
pub fn ensure_examples(req: &Request) {
    examples_usecase::examples_uc(&req.now().to_string());
}
