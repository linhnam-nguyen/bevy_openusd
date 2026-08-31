use bevy::prelude::Resource;
use viewport_protocol::{BimClassificationFieldCatalogue, BimFieldKey};

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
                fields: vec![
                    BimFieldKey::Category,
                    BimFieldKey::Family,
                    BimFieldKey::Type,
                ],
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_catalogue_revision_is_published_only_once() {
        let mut state = BimClassificationFieldCatalogueState::default();
        let catalogue = BimClassificationFieldCatalogue {
            semantic_revision: 1,
            fields: vec![
                BimFieldKey::Category,
                BimFieldKey::Family,
                BimFieldKey::Type,
                BimFieldKey::property("Window Only"),
            ],
        };

        assert!(state.replace(catalogue.clone()));
        assert!(!state.replace(catalogue));
        assert_eq!(state.current().semantic_revision, 1);
    }
}
