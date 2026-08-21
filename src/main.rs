//! `usdview` binary entry point.
//!
//! Viewer implementation lives in [`viewport`], keeping this file as the
//! composition boundary for the render-server executable.

// The binary intentionally contains dormant protocol, recovery, and diagnostic
// surfaces that are exercised by focused integration tests or downstream
// composition roots. Keep those APIs visible without turning the binary-wide
// hardening gate into a dead-code or query-shape redesign.
#![allow(dead_code, unused_imports)]
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::while_let_loop
)]

mod project;
mod viewport;

fn main() {
    viewport::run();
}
