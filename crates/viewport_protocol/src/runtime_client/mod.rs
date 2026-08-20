//! Client-side assembly of authorized self-render runtime payloads.
//!
//! This module is transport-neutral. A native or frontend adapter feeds it
//! the `SessionEvent` values received from the reliable control channel; it
//! never receives filesystem paths and it never treats an incomplete bundle as
//! renderable.

mod assembler;
mod types;

pub use assembler::RuntimeDeliveryAssembler;
pub use types::{HydratedRuntimeDelivery, RuntimeDeliveryClientError, RuntimeDeliveryUpdate};

#[cfg(test)]
mod tests;
