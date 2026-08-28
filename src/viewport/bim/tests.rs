use usd_model::CanonicalValue;
use viewport_protocol::{BimPageRequest, BimSearchQuery, CommonValue, SelectionReadModel};

use super::{BimQueryError, BimReadPolicy, BimReadService};

use super::test_fixtures::snapshot;

#[test]
fn property_read_projects_intersection_and_authoritative_units() {
    let snapshot = snapshot();
    let service = BimReadService::new(&snapshot);
    let targets = vec![
        viewport_protocol::SceneAnchor::active_session("/World/WallA"),
        viewport_protocol::SceneAnchor::active_session("/World/WallB"),
    ];
    let selection = SelectionReadModel {
        targets,
        primary: None,
    };
    let result = service
        .read_properties(
            &selection,
            17,
            BimReadPolicy {
                allow_value_edit: true,
            },
        )
        .expect("selected properties read");

    assert_eq!(result.targets, selection.targets);
    assert_eq!(result.selection_revision, 17);
    assert_eq!(result.properties.len(), 3);
    let mark = result
        .properties
        .iter()
        .find(|property| property.key == "Mark")
        .expect("common Mark property");
    assert!(matches!(mark.value, CommonValue::Multiple));
    assert_eq!(
        mark.group_id,
        viewport_protocol::BimPropertyGroupId::Semantic
    );
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
fn property_read_preserves_observed_connector_source_fallback_group() {
    let mut snapshot = super::test_fixtures::snapshot();
    for path in ["/World/WallA", "/World/WallB"] {
        snapshot
            .entities
            .get_mut(&usd_model::EntityKey::from(if path.ends_with('A') {
                "wall-a"
            } else {
                "wall-b"
            }))
            .expect("fixture wall")
            .properties
            .push(super::test_fixtures::property(
                "BIM:Instance:Surface",
                CanonicalValue::Text("22 m²".to_owned()),
                None,
            ));
    }
    let service = BimReadService::new(&snapshot);
    let selection = SelectionReadModel {
        targets: ["/World/WallA", "/World/WallB"]
            .into_iter()
            .map(viewport_protocol::SceneAnchor::active_session)
            .collect(),
        primary: None,
    };

    let result = service
        .read_properties(&selection, 18, BimReadPolicy::default())
        .expect("selected properties read");
    let source_property = result
        .properties
        .iter()
        .find(|property| property.key == "BIM:Instance:Surface")
        .expect("observed source property");
    assert_eq!(
        source_property.group_id,
        viewport_protocol::BimPropertyGroupId::SourceFallback
    );
}

#[test]
fn empty_selection_preserves_authoritative_revision() {
    let snapshot = snapshot();
    let service = BimReadService::new(&snapshot);
    let result = service
        .read_properties(&SelectionReadModel::default(), 23, BimReadPolicy::default())
        .expect("empty selection read");

    assert!(result.targets.is_empty());
    assert!(result.properties.is_empty());
    assert_eq!(result.selection_revision, 23);
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
fn search_pages_are_deterministic_with_a_bounded_window() {
    let snapshot = snapshot();
    let service = BimReadService::new(&snapshot);
    let page = BimPageRequest::new(1, 1);

    let objects = service
        .search(&BimSearchQuery::ObjectPropertyMatch {
            property: "Mark".to_owned(),
            pattern: "^AHU".to_owned(),
            page,
        })
        .expect("object search page");
    let viewport_protocol::BimSearchResult::Objects { matches, .. } = objects else {
        panic!("expected object result");
    };
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].anchor.prim_path, "/World/WallA");

    let preview = service
        .search(&BimSearchQuery::ReplacementPreview {
            property: "Mark".to_owned(),
            pattern: "^AHU".to_owned(),
            replacement: "MEP".to_owned(),
            page,
        })
        .expect("replacement page");
    let viewport_protocol::BimSearchResult::ReplacementPreview { rows, .. } = preview else {
        panic!("expected replacement result");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].old_value, "AHU-01");
}
