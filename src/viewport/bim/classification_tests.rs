use std::collections::HashMap;
use std::time::Instant;

use usd_model::{CanonicalValue, EntityKey, SemanticSnapshot, SnapshotId, SnapshotSource};
use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe};

use super::test_fixtures::{digest, entity, property, recipe, snapshot};
use super::{BimQueryError, BimReadService};

#[test]
fn classification_is_virtual_paged_and_cached() {
    let snapshot = snapshot();
    let mut service = BimReadService::new(&snapshot);
    let recipe = recipe();

    let roots = service
        .classification_page(&recipe, None, 0, 20)
        .expect("classification roots");
    assert_eq!(roots.total, 2);
    assert_eq!(service.classification_build_count(), 1);
    let walls_id = roots
        .nodes
        .iter()
        .find(|node| node.name == "Walls")
        .map(|node| node.id.clone())
        .expect("Walls classification node");

    let wall_levels = service
        .classification_page(&recipe, Some(&walls_id), 0, 20)
        .expect("Walls children");
    assert_eq!(wall_levels.total, 2);
    assert!(wall_levels.nodes.iter().all(|node| node.anchor.is_none()));
    assert_eq!(service.classification_build_count(), 1);

    let equipment_id = roots
        .nodes
        .iter()
        .find(|node| node.name == "Equipment")
        .map(|node| node.id.clone())
        .expect("Equipment classification node");
    let equipment_level = service
        .classification_page(&recipe, Some(&equipment_id), 0, 20)
        .expect("Equipment children");
    let unclassified_id = equipment_level
        .nodes
        .iter()
        .find(|node| node.name == "<Unclassified>")
        .map(|node| node.id.clone())
        .expect("missing Level uses unclassified bucket");
    let leaves = service
        .classification_page(&recipe, Some(&unclassified_id), 0, 20)
        .expect("unclassified children");
    assert!(leaves.nodes[0].anchor.is_none());
    assert_eq!(service.classification_build_count(), 1);
}

#[test]
fn classification_uses_normalized_leaf_names_and_generic_projection() {
    let snapshot = snapshot();
    let mut service = BimReadService::new(&snapshot);
    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category",
        BimFieldKey::Category,
    )]);

    let roots = service
        .classification_page(&recipe, None, 0, 20)
        .expect("classification roots");
    let walls_id = roots
        .nodes
        .iter()
        .find(|node| node.name == "Walls")
        .map(|node| node.id.clone())
        .expect("Walls classification node");
    let leaves = service
        .classification_page(&recipe, Some(&walls_id), 0, 20)
        .expect("classification leaves");

    assert_eq!(
        leaves.source,
        viewport_protocol::HierarchySource::BimClassification
    );
    assert!(leaves.nodes.iter().all(|node| node.anchor.is_some()));
    assert!(leaves.nodes.iter().any(|node| {
        node.name == "wall-a-Basic" && node.breadcrumb.ends_with("/ wall-a-Basic")
    }));
    assert!(
        leaves
            .nodes
            .iter()
            .all(|node| node.name == node.breadcrumb.rsplit(" / ").next().unwrap())
    );

    let snapshot = service
        .classification_snapshot(&recipe)
        .expect("classification snapshot");
    assert_eq!(
        snapshot.source,
        viewport_protocol::HierarchySource::BimClassification
    );
    assert!(
        snapshot
            .nodes
            .iter()
            .all(|node| { node.anchor.is_none() || node.name.contains('-') })
    );
}

#[test]
fn classification_leaf_name_falls_back_to_path_and_unclassified_family() {
    let mut snapshot = snapshot();
    let key = EntityKey::from("equipment-a");
    let mut entity = snapshot.entities.get(&key).expect("fixture entity").clone();
    entity.key = EntityKey::new("");
    entity.prim_path = "/World/EquipmentFallback".to_owned();
    entity.semantic.family = None;
    snapshot.entities.insert(EntityKey::new(""), entity);

    let mut service = BimReadService::new(&snapshot);
    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category",
        BimFieldKey::Category,
    )]);
    let roots = service
        .classification_page(&recipe, None, 0, 20)
        .expect("classification roots");
    let equipment_id = roots
        .nodes
        .iter()
        .find(|node| node.name == "Equipment")
        .map(|node| node.id.clone())
        .expect("Equipment classification node");
    let leaves = service
        .classification_page(&recipe, Some(&equipment_id), 0, 20)
        .expect("classification leaves");

    assert_eq!(leaves.nodes.len(), 2);
    assert!(
        leaves
            .nodes
            .iter()
            .any(|node| node.name == "/World/EquipmentFallback-<Unclassified>")
    );
    assert!(
        leaves
            .nodes
            .iter()
            .all(|node| node.name == node.breadcrumb.rsplit(" / ").next().unwrap())
    );
}

#[test]
fn large_fixture_reports_bounded_read_work_and_zero_idle_rebuilds() {
    let mut entities = HashMap::with_capacity(2_000);
    for index in 0..2_000 {
        let entity = entity(
            &format!("entity-{index:04}"),
            &format!("/World/Entity{index:04}"),
            Some(if index % 2 == 0 { "Walls" } else { "Equipment" }),
            Some(if index % 3 == 0 {
                "Basic"
            } else {
                "Mechanical"
            }),
            Some(if index % 3 == 0 { "Wall" } else { "AHU" }),
            vec![property(
                "Mark",
                CanonicalValue::Text(format!("BIM-{index:04}")),
                None,
            )],
        );
        entities.insert(entity.key.clone(), entity);
    }
    let snapshot = SemanticSnapshot {
        snapshot_id: SnapshotId("bim-large-fixture".to_owned()),
        source: SnapshotSource::Working {
            session: "bim-tests".to_owned(),
            live_revision: 2,
        },
        config_hash: digest(6),
        entities,
    };
    let mut service = BimReadService::new(&snapshot);
    let recipe = ClassificationRecipe::new(vec![
        ClassificationLevel::new("category", BimFieldKey::Category),
        ClassificationLevel::new("family", BimFieldKey::Family),
        ClassificationLevel::new("type", BimFieldKey::Type),
    ]);

    let start = Instant::now();
    let page = service
        .classification_page(&recipe, None, 0, 100)
        .expect("large classification roots");
    let classification_ms = start.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(page.total, 2);
    assert_eq!(service.classification_build_count(), 1);

    let start = Instant::now();
    let search = service
        .search(&viewport_protocol::BimSearchQuery::PropertyValueRegex {
            pattern: "^BIM-19".to_owned(),
            page: viewport_protocol::BimPageRequest::new(0, 100),
        })
        .expect("large search");
    let search_ms = start.elapsed().as_secs_f64() * 1_000.0;
    assert!(matches!(
        search,
        viewport_protocol::BimSearchResult::PropertyValues { total: 100, .. }
    ));

    let _ = service
        .classification_page(&recipe, None, 0, 100)
        .expect("cached classification roots");
    assert_eq!(service.classification_build_count(), 1);
    eprintln!(
        "M2-C6 fixture: entities=2000 levels=3 roots={} classification_ms={classification_ms:.3} search_ms={search_ms:.3}",
        page.total
    );
}

#[test]
fn classification_unknown_parent_is_rejected() {
    let snapshot = snapshot();
    let mut service = BimReadService::new(&snapshot);
    let error = service
        .classification_page(
            &recipe(),
            Some(&viewport_protocol::HierarchyNodeId::new("missing")),
            0,
            20,
        )
        .expect_err("unknown parent");
    assert!(matches!(
        error,
        BimQueryError::ClassificationNodeNotFound(_)
    ));
}
