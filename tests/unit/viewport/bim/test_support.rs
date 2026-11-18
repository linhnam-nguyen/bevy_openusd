use super::*;

impl<'snapshot> BimReadService<'snapshot> {
    pub(crate) fn classification_build_count(&self) -> u64 {
        self.classification_cache
            .as_ref()
            .map_or(0, |cache| cache.build_count)
    }
}
