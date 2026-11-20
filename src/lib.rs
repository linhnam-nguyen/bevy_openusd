//! Reusable backend application services for native USDHub hosts.
//!
//! The viewer binary remains the render composition root. This library target
//! exposes only the Project application boundary needed by the native host.

pub mod project;
mod semantic_query;

// Keep the library and usdview binary on the same application composition.
// Project recovery diagnostics reference the viewport counter resource, and
// one composition avoids test-only module topology that can hide release
// compilation errors.
#[path = "viewport/mod.rs"]
pub(crate) mod viewport;
