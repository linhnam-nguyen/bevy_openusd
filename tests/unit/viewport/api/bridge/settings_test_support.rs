//! Test-only helpers attached to the owning production module.
use super::*;
use viewport_protocol::ViewerEnvironmentSettings;

impl ViewerSettingsState {
    pub(crate) fn environment_mut(&mut self) -> &mut ViewerEnvironmentSettings {
        &mut self.0.environment
    }
}
