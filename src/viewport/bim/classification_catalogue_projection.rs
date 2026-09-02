use super::BimReadService;

impl<'snapshot> BimReadService<'snapshot> {
    /// Builds the bounded, model-wide field catalogue from BIM-eligible
    /// entities only. The caller supplies the semantic revision so the
    /// browser can invalidate this immutable list without coupling it to
    /// selection-scoped property reads.
    pub(crate) fn classification_field_catalogue(
        &self,
        semantic_revision: u64,
    ) -> viewport_protocol::BimClassificationFieldCatalogue {
        self.index.field_catalogue(semantic_revision)
    }
}
