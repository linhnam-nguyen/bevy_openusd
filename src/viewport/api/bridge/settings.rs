//! Protocol-owned supplementary viewer settings state.
//!
//! Renderer configuration and ground-grid origin remain owned by the existing
//! presentation resources. This resource stores only settings introduced by
//! Viewer Settings that do not have another authoritative owner yet.

use bevy::prelude::*;
use viewport_protocol::{
    SamplingProvider, SelectionPresentationSettings, ViewerEnvironmentSettings,
    ViewerSettingsReadModel,
};

#[derive(Resource, Debug, Clone, Default)]
pub(in crate::viewport) struct ViewerSettingsState(pub(super) ViewerSettingsReadModel);

impl ViewerSettingsState {
    pub(in crate::viewport) fn read_model(&self) -> ViewerSettingsReadModel {
        self.0.clone()
    }

    pub(in crate::viewport) fn environment(&self) -> &ViewerEnvironmentSettings {
        &self.0.environment
    }

    pub(in crate::viewport) fn selection(&self) -> &SelectionPresentationSettings {
        &self.0.selection
    }

    #[cfg(test)]
    pub(in crate::viewport) fn environment_mut(&mut self) -> &mut ViewerEnvironmentSettings {
        &mut self.0.environment
    }

    pub(in crate::viewport) fn set_ray_traced_supported(&mut self, supported: bool) {
        if self.0.capabilities.ray_traced_supported != supported {
            self.0.capabilities.ray_traced_supported = supported;
        }
    }

    pub(in crate::viewport) fn ray_traced_supported(&self) -> bool {
        self.0.capabilities.ray_traced_supported
    }

    pub(in crate::viewport) fn sampling_capabilities(&self) -> (bool, bool) {
        (
            self.0.capabilities.dlss_available,
            self.0.capabilities.fsr_available,
        )
    }

    pub(in crate::viewport) fn set_sampling_capabilities(
        &mut self,
        dlss_available: bool,
        fsr_available: bool,
    ) {
        self.0.capabilities.dlss_available = dlss_available;
        self.0.capabilities.fsr_available = fsr_available;
    }

    pub(in crate::viewport) fn set_sampling(
        &mut self,
        preference_enabled: bool,
        provider: SamplingProvider,
    ) {
        self.0.sampling.preference.enabled = preference_enabled;
        self.0.sampling.provider = provider;
    }
}
