//! `usdview` binary entry point.
//!
//! Viewer implementation lives in [`viewport`], keeping this file as the
//! composition boundary for the desktop executable.

mod viewport;

fn main() {
    viewport::run();
}
