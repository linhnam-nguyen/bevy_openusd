use bevy::prelude::Resource;
use viewport_protocol::BimClassificationFieldCatalogue;

/// Current immutable model-wide classification field catalogue.
///
/// It is replaced only when semantic synchronization accepts a new snapshot
/// or when a client explicitly requests the current snapshot. Selection
/// property responses never mutate this resource.
#[derive(Debug, Resource)]
pub(crate) struct BimClassificationFieldCatalogueState {
    catalogue: BimClassificationFieldCatalogue,
}

impl Default for BimClassificationFieldCatalogueState {
    fn default() -> Self {
        Self {
            catalogue: BimClassificationFieldCatalogue {
                semantic_revision: 0,
                fields: Vec::new(),
            },
        }
    }
}

impl BimClassificationFieldCatalogueState {
    pub(crate) fn current(&self) -> &BimClassificationFieldCatalogue {
        &self.catalogue
    }

    pub(crate) fn replace(&mut self, catalogue: BimClassificationFieldCatalogue) -> bool {
        if self.catalogue == catalogue {
            return false;
        }
        self.catalogue = catalogue;
        true
    }

    pub(crate) fn clear(&mut self) -> bool {
        self.replace(Self::default().catalogue)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/bim/field_catalogue_inline_tests.rs"]
mod tests;
