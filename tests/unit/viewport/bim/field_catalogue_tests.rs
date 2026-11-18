use super::*;
use viewport_protocol::{BimClassificationFieldDescriptor, BimFieldKey, BimPropertyScope};

#[test]
fn identical_catalogue_revision_is_published_only_once() {
    let mut state = BimClassificationFieldCatalogueState::default();
    let catalogue = BimClassificationFieldCatalogue {
        semantic_revision: 1,
        fields: vec![
            BimClassificationFieldDescriptor::new(
                BimFieldKey::Category,
                "Category",
                BimPropertyScope::Instance,
            ),
            BimClassificationFieldDescriptor::new(
                BimFieldKey::Family,
                "Family",
                BimPropertyScope::Other,
            ),
            BimClassificationFieldDescriptor::new(
                BimFieldKey::Type,
                "Type",
                BimPropertyScope::Type,
            ),
            BimClassificationFieldDescriptor::new(
                BimFieldKey::property("Window Only"),
                "Window Only",
                BimPropertyScope::Other,
            ),
        ],
    };

    assert!(state.replace(catalogue.clone()));
    assert!(!state.replace(catalogue));
    assert_eq!(state.current().semantic_revision, 1);
}
