//! Vendor-neutral sampling provider selection.

use bevy::prelude::*;
use viewport_protocol::{ViewportEvent, ViewportEventEnvelope};

use crate::viewport::api::{ViewerSettingsState, ViewportEventOutbox};

pub(crate) mod coordinator;
pub(crate) mod dlss;

pub(crate) use coordinator::{
    ActiveUpscaler, SamplingCapabilities, SamplingCoordinatorState, SamplingSelectionError,
    choose_upscaler,
};
pub(crate) use dlss::{DlssCameraActivation, DlssCapability, DlssProviderPlugin, configure_dlss};

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
    settings: Option<ResMut<ViewerSettingsState>>,
    mut sampling: ResMut<SamplingCoordinatorState>,
    mut dlss_camera: ResMut<DlssCameraActivation>,
    mut outbox: ResMut<ViewportEventOutbox>,
) {
    let Some(mut settings) = settings else {
        return;
    };

    // FSR remains forward-compatible vocabulary only. B4 has no reviewed
    // Vulkan implementation, so it must never be advertised or selected.
    let capabilities = SamplingCapabilities::new(dlss.supported(), false);
    let capabilities_changed = settings.sampling_capabilities()
        != (capabilities.dlss_available(), capabilities.fsr_available());
    if capabilities_changed {
        settings
            .set_sampling_capabilities(capabilities.dlss_available(), capabilities.fsr_available());
    }

    let next_active = match (sampling.preference_enabled, sampling.active) {
        (false, ActiveUpscaler::None) => ActiveUpscaler::None,
        (false, _) => ActiveUpscaler::None,
        (true, ActiveUpscaler::Dlss) if !capabilities.dlss_available() => {
            choose_upscaler(true, capabilities).unwrap_or(ActiveUpscaler::None)
        }
        (true, ActiveUpscaler::Fsr) if !capabilities.fsr_available() => {
            choose_upscaler(true, capabilities).unwrap_or(ActiveUpscaler::None)
        }
        // A newly available provider does not replace an already applied
        // provider, and an unavailable None state waits for a new request.
        (_, active) => active,
    };
    let selection_changed = next_active != sampling.active;
    if selection_changed {
        let preference_enabled = sampling.preference_enabled;
        sampling.apply(preference_enabled, next_active);
    }

    let dlss_enabled = sampling.active == ActiveUpscaler::Dlss;
    if dlss_camera.enabled != dlss_enabled {
        dlss_camera.enabled = dlss_enabled;
    }

    if selection_changed {
        settings.set_sampling(sampling.preference_enabled, sampling.active.provider());
    }

    if capabilities_changed || selection_changed {
        outbox.push(ViewportEventEnvelope::new(
            None,
            ViewportEvent::ViewerSettingsChanged {
                settings: settings.read_model(),
            },
        ));
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
