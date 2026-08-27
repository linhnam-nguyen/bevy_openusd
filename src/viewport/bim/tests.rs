use std::collections::HashMap;
use std::time::Instant;

use usd_model::{
    CanonicalValue, EntityKey, EntitySnapshot, HashDigest, IdentitySource, MeasurementMetadata,
    SemanticInfo, SemanticProperty, SemanticSnapshot, SnapshotId, SnapshotSource,
    TransformSignature,
};
use viewport_protocol::{
    BimFieldKey, BimPageRequest, BimSearchQuery, ClassificationLevel, ClassificationRecipe,
    ClassificationRow, CommonValue,
};

use super::{BimQueryError, BimReadPolicy, BimReadService};

fn digest(seed: u8) -> HashDigest {
    HashDigest::new([seed; HashDigest::BYTE_LEN])
}

fn property(
    name: &str,
    value: CanonicalValue,
    measurement: Option<MeasurementMetadata>,
) -> SemanticProperty {
    SemanticProperty {
        name: name.to_owned(),
        value,
        measurement,
    }
}

fn entity(
    key: &str,
    path: &str,
    category: Option<&str>,
    family: Option<&str>,
    type_name: Option<&str>,
    properties: Vec<SemanticProperty>,
) -> EntitySnapshot {
    EntitySnapshot {
        key: EntityKey::from(key),
        prim_path: path.to_owned(),
        identity_source: IdentitySource::PrimPath,
        semantic: SemanticInfo {
            category: category.map(str::to_owned),
            family: family.map(str::to_owned),
            type_name: type_name.map(str::to_owned),
            ..SemanticInfo::default()
        },
        transform: TransformSignature {
            translation_mm: [0, 0, 0],
            rotation_quantized: [0, 0, 0, 1],
            scale_quantized: [1, 1, 1],
            hash: digest(3),
        },
        geometry: None,
        properties,
        metadata_hash: digest(4),
        full_hash: digest(5),
    }
}

fn snapshot() -> SemanticSnapshot {
    let measured = Some(MeasurementMetadata::new(
        "length",
        "m",
        Some("mm".to_owned()),
    ));
    let entities = [
        entity(
            "wall-a",
            "/World/WallA",
            Some("Walls"),
            Some("Basic"),
            Some("Wall"),
            vec![
                property("Mark", CanonicalValue::Text("AHU-01".to_owned()), None),
                property("Width", CanonicalValue::Real(0.2), measured.clone()),
                property("Level", CanonicalValue::Text("02".to_owned()), None),
            ],
        ),
        entity(
            "wall-b",
            "/World/WallB",
            Some("Walls"),
            Some("Basic"),
            Some("Wall"),
            vec![
                property("Mark", CanonicalValue::Text("AHU-02".to_owned()), None),
                property("Width", CanonicalValue::Real(0.2), measured),
                property("Level", CanonicalValue::Text("03".to_owned()), None),
            ],
        ),
        entity(
            "equipment-a",
            "/World/EquipmentA",
            Some("Equipment"),
            Some("Mechanical"),
            Some("AHU"),
            vec![property(
                "Mark",
                CanonicalValue::Text("AHU-03".to_owned()),
                None,
            )],
        ),
    ]
    .into_iter()
    .map(|entity| (entity.key.clone(), entity))
    .collect();
    SemanticSnapshot {
        snapshot_id: SnapshotId("bim-test-snapshot".to_owned()),
        source: SnapshotSource::Working {
            session: "bim-tests".to_owned(),
            live_revision: 1,
        },
        config_hash: digest(1),
        entities,
    }
}

fn recipe() -> ClassificationRecipe {
    ClassificationRecipe::new(vec![
        ClassificationLevel::new("category", BimFieldKey::Category),
        ClassificationLevel::new("level", BimFieldKey::property("Level")),
        ClassificationLevel::new("type", BimFieldKey::Type),
    ])
}

#[test]
fn property_read_projects_intersection_and_authoritative_units() {
    let snapshot = snapshot();
    let service = BimReadService::new(&snapshot);
    let targets = vec![
        viewport_protocol::SceneAnchor::active_session("/World/WallA"),
        viewport_protocol::SceneAnchor::active_session("/World/WallB"),
    ];
    let result = service
        .read_properties(
            &targets,
            BimReadPolicy {
                allow_value_edit: true,
            },
        )
        .expect("selected properties read");

    assert_eq!(result.properties.len(), 3);
    let mark = result
        .properties
        .iter()
        .find(|property| property.key == "Mark")
        .expect("common Mark property");
    assert!(matches!(mark.value, CommonValue::Multiple));
    assert!(mark.editable);

    let width = result
        .properties
        .iter()
        .find(|property| property.key == "Width")
        .expect("common Width property");
    assert_eq!(width.value, CommonValue::Same(CanonicalValue::Real(0.2)));
    assert_eq!(
        width.measurement.as_ref().unwrap().canonical_unit.as_str(),
        "m"
    );
    assert!(width.units.iter().any(|unit| unit.unit.as_str() == "mm"));
}

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
        .rows
        .iter()
        .find_map(|row| match row {
            ClassificationRow::Group(group) if group.label == "Walls" => Some(group.id.clone()),
            _ => None,
        })
        .expect("Walls classification node");

    let wall_levels = service
        .classification_page(&recipe, Some(&walls_id), 0, 20)
        .expect("Walls children");
    assert_eq!(wall_levels.total, 2);
    assert!(
        wall_levels
            .rows
            .iter()
            .all(|row| matches!(row, ClassificationRow::Group(_)))
    );
    assert_eq!(service.classification_build_count(), 1);

    let equipment_id = roots
        .rows
        .iter()
        .find_map(|row| match row {
            ClassificationRow::Group(group) if group.label == "Equipment" => Some(group.id.clone()),
            _ => None,
        })
        .expect("Equipment classification node");
    let equipment_level = service
        .classification_page(&recipe, Some(&equipment_id), 0, 20)
        .expect("Equipment children");
    let unclassified_id = equipment_level
        .rows
        .iter()
        .find_map(|row| match row {
            ClassificationRow::Group(group) if group.label == "<Unclassified>" => {
                Some(group.id.clone())
            }
            _ => None,
        })
        .expect("missing Level uses unclassified bucket");
    let leaves = service
        .classification_page(&recipe, Some(&unclassified_id), 0, 20)
        .expect("unclassified children");
    assert!(matches!(leaves.rows[0], ClassificationRow::Group(_)));
    assert_eq!(service.classification_build_count(), 1);
}

#[test]
fn search_supports_all_variants_and_compiles_one_bounded_regex() {
    let snapshot = snapshot();
    let service = BimReadService::new(&snapshot);

    let names = service
        .search(&BimSearchQuery::PropertyNameRegex {
            pattern: "Mark|Width".to_owned(),
            page: BimPageRequest::new(0, 20),
        })
        .expect("property-name search");
    let viewport_protocol::BimSearchResult::PropertyNames { total, .. } = names else {
        panic!("expected property-name result");
    };
    assert_eq!(total, 2);

    let values = service
        .search(&BimSearchQuery::PropertyValueRegex {
            pattern: "^AHU-0".to_owned(),
            page: BimPageRequest::new(0, 20),
        })
        .expect("property-value search");
    let viewport_protocol::BimSearchResult::PropertyValues { total, .. } = values else {
        panic!("expected property-value result");
    };
    assert_eq!(total, 3);

    let objects = service
        .search(&BimSearchQuery::ObjectPropertyMatch {
            property: "Mark".to_owned(),
            pattern: "^AHU-02$".to_owned(),
            page: BimPageRequest::new(0, 20),
        })
        .expect("object search");
    let viewport_protocol::BimSearchResult::Objects { matches, .. } = objects else {
        panic!("expected object result");
    };
    assert_eq!(matches[0].anchor.prim_path, "/World/WallB");

    let preview = service
        .search(&BimSearchQuery::ReplacementPreview {
            property: "Mark".to_owned(),
            pattern: "^AHU-(\\d+)$".to_owned(),
            replacement: "MEP-AHU-${1}".to_owned(),
            page: BimPageRequest::new(0, 20),
        })
        .expect("replacement preview");
    let viewport_protocol::BimSearchResult::ReplacementPreview { rows, .. } = preview else {
        panic!("expected replacement result");
    };
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .any(|row| { row.old_value == "AHU-01" && row.proposed_value == "MEP-AHU-01" })
    );

    let invalid = service.search(&BimSearchQuery::PropertyValueRegex {
        pattern: "(".to_owned(),
        page: BimPageRequest::new(0, 20),
    });
    assert!(matches!(invalid, Err(BimQueryError::InvalidRegex(_))));
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
        .search(&BimSearchQuery::PropertyValueRegex {
            pattern: "^BIM-19".to_owned(),
            page: BimPageRequest::new(0, 100),
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
