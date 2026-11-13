use std::collections::HashMap;
use std::time::Instant;

use usd_model::{CanonicalValue, EntityKey, SemanticSnapshot, SnapshotId, SnapshotSource};
use viewport_protocol::{
    BimFieldKey, BimPageRequest, BimSearchQuery, BimSearchResult, ClassificationColorIntent,
    ClassificationColorSource, ClassificationLevel, ClassificationRecipe, SceneAnchor,
    SelectionReadModel,
};

use super::test_fixtures::{digest, entity, property};
use super::{BimReadPolicy, BimReadService};

const ENTITY_COUNT: usize = 4_000;
const PROPERTY_COUNT_PER_ENTITY: usize = 3;

fn large_snapshot() -> SemanticSnapshot {
    let mut entities = HashMap::with_capacity(ENTITY_COUNT);
    for index in 0..ENTITY_COUNT {
        let key = EntityKey::from(format!("m8-entity-{index:05}"));
        let entity = entity(
            key.as_str(),
            &format!("/World/Entity{index:05}"),
            Some(if index % 2 == 0 { "Walls" } else { "Equipment" }),
            Some(if index % 3 == 0 {
                "Basic"
            } else {
                "Mechanical"
            }),
            Some(if index % 3 == 0 { "Wall" } else { "AHU" }),
            vec![
                property(
                    "Mark",
                    CanonicalValue::Text(format!("BIM-{index:05}")),
                    None,
                ),
                property("Width", CanonicalValue::Real(index as f64 / 100.0), None),
                property(
                    "Level",
                    CanonicalValue::Text(format!("{:02}", index % 20)),
                    None,
                ),
            ],
        );
        entities.insert(key, entity);
    }
    SemanticSnapshot {
        snapshot_id: SnapshotId("m8-performance-fixture".to_owned()),
        source: SnapshotSource::Working {
            session: "m8-performance".to_owned(),
            live_revision: 1,
        },
        config_hash: digest(8),
        entities,
    }
}

fn recipe() -> ClassificationRecipe {
    ClassificationRecipe::new(vec![
        ClassificationLevel::new("category", BimFieldKey::Category),
        ClassificationLevel::new("family", BimFieldKey::Family),
        ClassificationLevel::new("type", BimFieldKey::Type),
    ])
}

fn anchor(index: usize) -> SceneAnchor {
    SceneAnchor::active_session(format!("/World/Entity{index:05}"))
}

#[test]
fn large_bim_fixture_records_cold_idle_query_intersection_and_color_costs() {
    let snapshot = large_snapshot();
    let mut service = BimReadService::new(&snapshot);
    let recipe = recipe();

    let started = Instant::now();
    let roots = service
        .classification_page(&recipe, None, 0, 100)
        .expect("classification roots");
    let classification_ms = started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(service.classification_build_count(), 1);
    assert_eq!(roots.total, 2);

    let started = Instant::now();
    let idle_roots = service
        .classification_page(&recipe, None, 0, 100)
        .expect("cached classification roots");
    let idle_classification_us = started.elapsed().as_secs_f64() * 1_000_000.0;
    assert_eq!(idle_roots, roots);
    assert_eq!(service.classification_build_count(), 1);

    let started = Instant::now();
    let regex = service
        .search(&BimSearchQuery::PropertyValueRegex {
            pattern: "^BIM-039".to_owned(),
            page: BimPageRequest::new(0, 100),
        })
        .expect("regex search");
    let regex_ms = started.elapsed().as_secs_f64() * 1_000.0;
    assert!(matches!(
        regex,
        BimSearchResult::PropertyValues { total: 100, .. }
    ));

    let started = Instant::now();
    let first_page = service
        .search(&BimSearchQuery::ObjectPropertyMatch {
            property: "Mark".to_owned(),
            pattern: "^BIM-".to_owned(),
            page: BimPageRequest::new(0, 64),
        })
        .expect("first object page");
    let first_page_ms = started.elapsed().as_secs_f64() * 1_000.0;
    assert!(matches!(
        first_page,
        BimSearchResult::Objects {
            total: 4_000,
            has_more: true,
            ref matches,
            ..
        } if matches.len() == 64
    ));

    let second_page = service
        .search(&BimSearchQuery::ObjectPropertyMatch {
            property: "Mark".to_owned(),
            pattern: "^BIM-".to_owned(),
            page: BimPageRequest::new(64, 64),
        })
        .expect("second object page");
    assert!(matches!(
        second_page,
        BimSearchResult::Objects {
            total: 4_000,
            ref matches,
            ..
        } if matches.len() == 64 && matches[0].anchor == anchor(64)
    ));

    let started = Instant::now();
    let properties = service
        .read_properties(
            &SelectionReadModel {
                targets: vec![anchor(0), anchor(2)],
                primary: Some(anchor(0)),
            },
            7,
            BimReadPolicy::default(),
        )
        .expect("multi-selection property intersection");
    let intersection_us = started.elapsed().as_secs_f64() * 1_000_000.0;
    assert_eq!(properties.targets.len(), 2);
    assert_eq!(
        properties
            .groups
            .iter()
            .flat_map(|group| &group.properties)
            .count(),
        PROPERTY_COUNT_PER_ENTITY
    );

    let started = Instant::now();
    let colors = service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("category".to_owned()),
                generation: 1,
            },
        )
        .expect("classification colors");
    let color_ms = started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(colors.len(), ENTITY_COUNT);
    let idle_color_started = Instant::now();
    let idle_colors = service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("category".to_owned()),
                generation: 1,
            },
        )
        .expect("cached classification colors");
    let idle_color_us = idle_color_started.elapsed().as_secs_f64() * 1_000_000.0;
    assert_eq!(idle_colors, colors);
    assert_eq!(service.classification_build_count(), 1);

    eprintln!(
        "M8-C3 benchmark: entities={ENTITY_COUNT} properties={} levels=3 roots={} classification_ms={classification_ms:.3} idle_classification_us={idle_classification_us:.3} regex_ms={regex_ms:.3} first_page_ms={first_page_ms:.3} intersection_us={intersection_us:.3} color_ms={color_ms:.3} idle_color_us={idle_color_us:.3} classification_rebuilds={}",
        ENTITY_COUNT * PROPERTY_COUNT_PER_ENTITY,
        roots.total,
        service.classification_build_count()
    );
}
