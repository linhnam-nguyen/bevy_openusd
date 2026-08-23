//! Protocol-owned supplementary viewer settings state.
//!
//! Renderer configuration and ground-grid origin remain owned by the existing
//! presentation resources. This resource stores only settings introduced by
//! Viewer Settings that do not have another authoritative owner yet.

use bevy::prelude::*;
use viewport_protocol::{ViewerEnvironmentSettings, ViewerSettingsReadModel};

#[derive(Resource, Debug, Clone, Default)]
pub(in crate::viewport) struct ViewerSettingsState(pub(super) ViewerSettingsReadModel);

impl ViewerSettingsState {
    pub(in crate::viewport) fn environment(&self) -> &ViewerEnvironmentSettings {
        &self.0.environment
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
}
