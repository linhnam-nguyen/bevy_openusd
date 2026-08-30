//! Exact dependency closure handling for Project Scene and Model imports.

#[path = "source_closure_discovery.rs"]
mod discovery;
#[path = "source_closure_io.rs"]
mod io;
#[path = "source_closure_localize.rs"]
mod localize;

pub(crate) use localize::{materialize_source_closure, source_closure_fingerprint};

#[cfg(test)]
#[path = "source_closure_tests.rs"]
mod tests;
