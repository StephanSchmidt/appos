//! Webinars: the public list of sessions, registration with a waitlist,
//! each person's own registration page, and the events team's pages.
//! Other modules reach it only through what this file makes `pub`.

mod cancel_session_usecase;
mod cancel_usecase;
mod clock;
mod db;
mod delete_usecase;
mod domain;
mod export;
mod register_usecase;
mod report_usecase;
mod save_session_usecase;
mod team;
mod team_key_usecase;
#[cfg(test)]
mod testing;
mod ticket_usecase;
mod upcoming_usecase;
mod view;
mod web;

use crate::appos::{Module, Route};

pub const MODULE: Module = Module {
    routes: ROUTES,
    menu: &[("Webinars", "/")],
};

const ROUTES: &[Route] = &[
    // Public
    Route::get("/", web::index),
    Route::get("/sessions/:session", web::session),
    Route::post("/sessions/:session/register", web::register),
    Route::get("/registration/:session/:reg", web::registration),
    Route::post("/registration/:session/:reg/cancel", web::cancel),
    // The events team, under a secret key (team.rs)
    Route::get("/team/:key", team::home),
    Route::get("/team/:key/new", team::new_session),
    Route::post("/team/:key/sessions", team::create),
    Route::get("/team/:key/sessions/:session", team::report),
    Route::post("/team/:key/sessions/:session", team::update),
    Route::get("/team/:key/sessions/:session/edit", team::edit),
    Route::post("/team/:key/sessions/:session/cancel", team::cancel_session),
    Route::post("/team/:key/sessions/:session/delete", team::delete_session),
    Route::post(
        "/team/:key/sessions/:session/registrations/:reg/delete",
        team::delete_registration,
    ),
];
