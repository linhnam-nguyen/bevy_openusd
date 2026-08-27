//! Reusable backend application services for native USDHub hosts.
//!
//! The viewer binary remains the render composition root. This library target
//! exposes only the Project application boundary needed by the native host.

#[path = "project_api.rs"]
pub mod project;
