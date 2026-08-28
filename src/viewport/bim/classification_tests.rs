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
    let mut snapshot = snapshot();
    for (key, element_id) in [("wall-a", "184392"), ("wall-b", "184393")] {
        snapshot
            .entities
            .get_mut(&EntityKey::from(key))
            .expect("fixture wall")
            .semantic
            .type_id = Some(element_id.to_owned());
    }
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
        node.name == "184392-Basic" && node.breadcrumb.ends_with("/ 184392-Basic")
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
            .any(|node| { node.anchor.is_some() && node.name == "184392-Basic" })
    );
}

#[test]
fn classification_leaf_name_follows_all_normalized_identity_fallbacks() {
    let mut snapshot = snapshot();
    let key = EntityKey::from("equipment-a");
    let mut entity = snapshot.entities.get(&key).expect("fixture entity").clone();
    entity.semantic.type_id = Some("184392".to_owned());
    entity.semantic.family = Some("Air Handling Unit".to_owned());
    entity.semantic.display_name = Some("Projected AHU".to_owned());
    snapshot.entities.insert(key.clone(), entity);

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

    assert_eq!(leaves.nodes.len(), 1);
    assert!(
        leaves
            .nodes
            .iter()
            .any(|node| node.name == "184392-Air Handling Unit")
    );
    assert!(
        leaves
            .nodes
            .iter()
            .all(|node| node.name == node.breadcrumb.rsplit(" / ").next().unwrap())
    );

    let mut expected = snapshot.entities[&key].clone();
    expected.semantic.family = None;
    assert_eq!(
        super::classification::projected_entity_name(&expected),
        "184392"
    );
    expected.semantic.type_id = None;
    expected.semantic.family = Some("Air Handling Unit".to_owned());
    assert_eq!(
        super::classification::projected_entity_name(&expected),
        "Air Handling Unit"
    );
    expected.semantic.family = None;
    assert_eq!(
        super::classification::projected_entity_name(&expected),
        "Projected AHU"
    );
    expected.semantic.display_name = None;
    assert_eq!(
        super::classification::projected_entity_name(&expected),
        "/World/EquipmentA"
    );
}

#[test]
fn classification_groups_by_arbitrary_property_and_reuses_projected_leaf_name() {
    let mut snapshot = snapshot();
    let wall_a = snapshot
        .entities
        .get_mut(&EntityKey::from("wall-a"))
        .expect("fixture wall A");
    wall_a.semantic.type_id = Some("184392".to_owned());
    wall_a.semantic.family = Some("Air Handling Unit".to_owned());
    wall_a.properties.push(property(
        "BIM:Type:Largeur",
        CanonicalValue::Real(200.0),
        None,
    ));
    let wall_b = snapshot
        .entities
        .get_mut(&EntityKey::from("wall-b"))
        .expect("fixture wall B");
    wall_b.semantic.type_id = Some("184393".to_owned());
    wall_b.semantic.family = Some("Air Handling Unit".to_owned());
    wall_b.properties.push(property(
        "BIM:Type:Largeur",
        CanonicalValue::Real(300.0),
        None,
    ));

    let mut service = BimReadService::new(&snapshot);
    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "width",
        BimFieldKey::property("BIM:Type:Largeur"),
    )]);
    let roots = service
        .classification_page(&recipe, None, 0, 20)
        .expect("arbitrary property roots");
    assert_eq!(roots.total, 3);
    let width_200 = roots
        .nodes
        .iter()
        .find(|node| node.name == "200")
        .map(|node| node.id.clone())
        .expect("200 width group");
    let leaves = service
        .classification_page(&recipe, Some(&width_200), 0, 20)
        .expect("arbitrary property leaves");
    assert_eq!(leaves.nodes[0].name, "184392-Air Handling Unit");

    let objects = service
        .search(&viewport_protocol::BimSearchQuery::ObjectPropertyMatch {
            property: "BIM:Type:Largeur".to_owned(),
            pattern: "^200$".to_owned(),
            page: viewport_protocol::BimPageRequest::new(0, 20),
        })
        .expect("arbitrary property object search");
    let viewport_protocol::BimSearchResult::Objects { matches, .. } = objects else {
        panic!("expected object search result");
    };
    assert_eq!(matches[0].label, "184392-Air Handling Unit");
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
