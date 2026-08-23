//! Vendor-neutral sampling provider selection.

use bevy::prelude::*;

pub(crate) mod coordinator;
pub(crate) mod dlss;
pub(crate) mod fsr_vulkan;

pub(crate) use coordinator::{
    ActiveUpscaler, SamplingCapabilities, SamplingCoordinatorState, SamplingSelectionError,
    choose_upscaler,
};
pub(crate) use dlss::{DlssCameraActivation, DlssCapability, DlssProviderPlugin, configure_dlss};
pub(crate) use fsr_vulkan::{FsrVulkanCapability, FsrVulkanProviderPlugin};

/// Publishes provider availability into the renderer-neutral read model.
pub(crate) struct SamplingCoordinatorPlugin;

impl Plugin for SamplingCoordinatorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SamplingCoordinatorState>().add_systems(
            Update,
            publish_sampling_capabilities
                .before(crate::viewport::api::ViewportBridgeSet::ApplyCommands),
        );
    }
}

fn publish_sampling_capabilities(
    dlss: Res<DlssCapability>,
    fsr: Res<FsrVulkanCapability>,
    settings: Option<Res<crate::viewport::api::ViewerSettingsState>>,
    mut commands: Commands,
) {
    let Some(settings) = settings else {
        return;
    };
    let capabilities = (dlss.supported(), fsr.supported());
    if settings.sampling_capabilities() == capabilities {
        return;
    }

    let mut next = (*settings).clone();
    next.set_sampling_capabilities(capabilities.0, capabilities.1);
    commands.insert_resource(next);
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
