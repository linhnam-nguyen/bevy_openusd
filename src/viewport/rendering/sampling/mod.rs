//! Vendor-neutral sampling provider selection.

pub(crate) mod coordinator;
pub(crate) mod dlss;
pub(crate) mod fsr_vulkan;

pub(crate) use dlss::{DlssProviderPlugin, configure_dlss};
pub(crate) use fsr_vulkan::FsrVulkanProviderPlugin;

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
