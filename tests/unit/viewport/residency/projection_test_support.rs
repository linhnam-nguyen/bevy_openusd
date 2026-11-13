//! Test-only helpers attached to the owning production module.
use super::*;

impl SceneResidencyProjection {
    pub(super) fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn active_entity_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.entity.is_some())
            .count()
    }
}
