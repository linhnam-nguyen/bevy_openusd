//! Test-only helpers attached to the owning production module.
use super::*;

impl ViewerSettingsState {
    pub(crate) fn environment_mut(&mut self) -> &mut ViewerEnvironmentSettings {
        &mut self.0.environment
    }
}
