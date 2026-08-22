//! Protocol-owned supplementary viewer settings state.
//!
//! Renderer configuration and ground-grid origin remain owned by the existing
//! presentation resources. This resource stores only settings introduced by
//! Viewer Settings that do not have another authoritative owner yet.

use bevy::prelude::*;
use viewport_protocol::{ViewerEnvironmentSettings, ViewerSettingsReadModel};

#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct ViewerSettingsState(pub(super) ViewerSettingsReadModel);

impl ViewerSettingsState {
    pub(crate) fn environment(&self) -> &ViewerEnvironmentSettings {
        &self.0.environment
    }

    pub(crate) fn environment_mut(&mut self) -> &mut ViewerEnvironmentSettings {
        &mut self.0.environment
    }
}
