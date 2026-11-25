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
pub(crate) struct ViewerSettingsState(pub(super) ViewerSettingsReadModel);

impl ViewerSettingsState {
    pub(crate) fn read_model(&self) -> ViewerSettingsReadModel {
        self.0.clone()
    }

    pub(crate) fn environment(&self) -> &ViewerEnvironmentSettings {
        &self.0.environment
    }

    pub(crate) fn selection(&self) -> &SelectionPresentationSettings {
        &self.0.selection
    }

    pub(crate) fn section_box_enabled(&self) -> bool {
        self.0.section_box.enabled
    }

    pub(crate) fn set_section_box_enabled(&mut self, enabled: bool) {
        self.0.section_box.enabled = enabled;
    }

    pub(crate) fn set_gizmo_size_level(&mut self, level: u8) {
        self.0.selection.gizmo_size_level = level;
    }

    pub(crate) fn set_ray_traced_supported(&mut self, supported: bool) {
        if self.0.capabilities.ray_traced_supported != supported {
            self.0.capabilities.ray_traced_supported = supported;
        }
    }

    pub(crate) fn ray_traced_supported(&self) -> bool {
        self.0.capabilities.ray_traced_supported
    }

    pub(crate) fn sampling_capabilities(&self) -> (bool, bool) {
        (
            self.0.capabilities.dlss_available,
            self.0.capabilities.fsr_available,
        )
    }

    pub(crate) fn set_sampling_capabilities(
        &mut self,
        dlss_available: bool,
        fsr_available: bool,
    ) {
        self.0.capabilities.dlss_available = dlss_available;
        self.0.capabilities.fsr_available = fsr_available;
    }

    pub(crate) fn set_sampling(
        &mut self,
        preference_enabled: bool,
        provider: SamplingProvider,
    ) {
        self.0.sampling.preference.enabled = preference_enabled;
        self.0.sampling.provider = provider;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/api/bridge/settings_test_support.rs"]
mod test_support;
