//! The hello module: say hello, and see who said hello before. Other modules
//! reach it only through what this file makes `pub`; its data (db.rs) stays
//! private to it.

mod db;
mod domain;
mod greet_usecase;
mod list_usecase;
mod show_usecase;
mod web;

use crate::appos::{Module, Route};

/// The module's routes and menu entries. Paths are below the app's base;
/// `:name` segments arrive as `req.param("name")`.
pub const MODULE: Module = Module {
    routes: &[
        Route::get("/", web::index),
        Route::post("/greet", web::greet),
        Route::get("/greetings/:id", web::greeting),
    ],
    menu: &[("Hello", "/")],
};
