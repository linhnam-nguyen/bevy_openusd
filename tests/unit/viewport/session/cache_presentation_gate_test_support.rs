//! Test-only helpers attached to the owning production module.
use super::*;

impl CachePresentationGate {
    pub(crate) fn rendered(&self) -> bool {
        self.rendered_generation == Some(self.generation)
    }
}
