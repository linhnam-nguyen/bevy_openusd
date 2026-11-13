//! Test-only helpers attached to the owning production module.
use super::*;

impl ViewerSettingsState {
    pub(in crate::viewport) fn environment_mut(&mut self) -> &mut ViewerEnvironmentSettings {
        &mut self.0.environment
    }
}
