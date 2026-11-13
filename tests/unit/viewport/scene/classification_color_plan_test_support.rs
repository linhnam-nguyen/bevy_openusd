//! Test-only helpers attached to the owning production module.
use super::*;

impl ClassificationColorPlan {
    pub(in crate::viewport) fn replace(
        &mut self,
        generation: u64,
        entries: Vec<ClassificationColorEntry>,
    ) {
        self.replace_entries(entries);
        self.generation = generation;
    }
}
