//! Renderer-local FSR Vulkan adapter contract.
//!
//! Bevy 0.19 has no native FSR provider and this checkout does not contain a
//! reviewed FidelityFX backend. The adapter therefore stays fail-closed until
//! a backend supplies all required runtime capabilities. No Vulkan or SDK
//! handle is represented in this module's public-in-crate contract.

use bevy::prelude::*;

/// Runtime facts required before the coordinator may select FSR.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FsrVulkanCapability {
    pub(crate) vulkan_backend: bool,
    pub(crate) fidelityfx_backend: bool,
    pub(crate) input_contract_ready: bool,
}

impl FsrVulkanCapability {
    pub(crate) const fn from_probe(
        vulkan_backend: bool,
        fidelityfx_backend: bool,
        input_contract_ready: bool,
    ) -> Self {
        Self {
            vulkan_backend,
            fidelityfx_backend,
            input_contract_ready,
        }
    }

    pub(crate) const fn supported(self) -> bool {
        self.vulkan_backend && self.fidelityfx_backend && self.input_contract_ready
    }
}

/// Renderer-only resources required by the selected FSR generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FsrFrameInput {
    pub(crate) input_extent: UVec2,
    pub(crate) output_extent: UVec2,
    pub(crate) motion_vectors: bool,
    pub(crate) depth: bool,
    pub(crate) exposure: bool,
    pub(crate) cpu_readback: bool,
}

/// Explicit rejection reasons for incomplete provider integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FsrInputError {
    InvalidResolution,
    MissingMotionVectors,
    MissingDepth,
    MissingExposure,
    CpuReadbackInPipeline,
}

/// Isolated FSR adapter surface consumed by renderer code, never by protocol.
pub(crate) struct FsrVulkanProvider;

impl FsrVulkanProvider {
    pub(crate) fn validate_frame_input(input: FsrFrameInput) -> Result<(), FsrInputError> {
        if input.input_extent.x == 0
            || input.input_extent.y == 0
            || input.output_extent.x == 0
            || input.output_extent.y == 0
            || input.input_extent.x >= input.output_extent.x
            || input.input_extent.y >= input.output_extent.y
        {
            return Err(FsrInputError::InvalidResolution);
        }
        if !input.motion_vectors {
            return Err(FsrInputError::MissingMotionVectors);
        }
        if !input.depth {
            return Err(FsrInputError::MissingDepth);
        }
        if !input.exposure {
            return Err(FsrInputError::MissingExposure);
        }
        if input.cpu_readback {
            return Err(FsrInputError::CpuReadbackInPipeline);
        }
        Ok(())
    }
}

/// Registers the fail-closed capability resource for the sampling coordinator.
pub(crate) struct FsrVulkanProviderPlugin;

impl Plugin for FsrVulkanProviderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FsrVulkanCapability>();
    }
}

#[cfg(test)]
#[path = "fsr_vulkan_tests.rs"]
mod tests;
