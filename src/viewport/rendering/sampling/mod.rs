//! Vendor-neutral sampling provider selection.

pub(crate) mod coordinator;
pub(crate) mod dlss;

pub(crate) use dlss::{DlssProviderPlugin, configure_dlss};

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
