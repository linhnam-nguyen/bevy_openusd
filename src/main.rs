//! `usdview` binary entry point.

#![allow(dead_code)]
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::while_let_loop
)]

mod animation;
mod api;
mod app;
mod authoring;
mod bim;
mod cache;
mod comparison;
mod diagnostics;
mod document;
mod history;
mod physics;
mod project;
mod semantic;
mod transport;
mod viewport;

fn main() {
    app::run();
}
