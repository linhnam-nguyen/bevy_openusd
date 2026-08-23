//! Vendor-neutral sampling provider selection.

pub(crate) mod coordinator;

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
