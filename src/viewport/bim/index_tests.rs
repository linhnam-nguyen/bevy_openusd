use std::sync::Arc;

use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe};

use super::test_fixtures::snapshot;
use super::{BimReadIndex, BimReadService};

#[test]
fn one_snapshot_index_is_reused_by_services_and_property_postings() {
    let snapshot = snapshot();
    let index = Arc::new(BimReadIndex::build(&snapshot));
    assert_eq!(index.entity_order().len(), snapshot.entities.len());
    assert_eq!(
        index
            .property_postings(index.property_id("Mark").expect("Mark property"))
            .len(),
        snapshot.entities.len()
    );

    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category",
        BimFieldKey::Category,
    )]);
    let mut first = BimReadService::with_index(&snapshot, Arc::clone(&index));
    first
        .classification_snapshot(&recipe)
        .expect("first classification projection");
    let mut second = BimReadService::with_index(&snapshot, index.clone());
    second
        .classification_snapshot(&recipe)
        .expect("second classification projection");

    assert_eq!(index.classification_cache_len(), 1);
}

#[test]
fn snapshot_index_keeps_deterministic_bim_entity_order() {
    let snapshot = snapshot();
    let index = BimReadIndex::build(&snapshot);
    let paths = index
        .entity_order()
        .iter()
        .map(|key| snapshot.entities[key].prim_path.as_str())
        .collect::<Vec<_>>();
    assert!(paths.windows(2).all(|pair| pair[0] <= pair[1]));
}
