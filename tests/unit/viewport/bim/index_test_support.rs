//! Test-only helpers attached to the owning production module.
use super::*;

impl BimReadIndex {
    pub(in crate::viewport::bim) fn classification_cache_len(&self) -> usize {
        self.classification_cache
            .lock()
            .expect("BIM classification cache mutex is not poisoned")
            .len()
    }
}
