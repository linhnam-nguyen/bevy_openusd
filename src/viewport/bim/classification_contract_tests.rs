use usd_model::{CanonicalValue, SemanticSnapshot};
use viewport_protocol::{
    BimFieldKey, BimPageRequest, BimSearchQuery, BimSearchResult, ClassificationColorIntent,
    ClassificationColorSource, ClassificationLevel, ClassificationRecipe, UNCLASSIFIED_LABEL,
};

use super::BimReadService;
use super::test_fixtures::{entity, property, snapshot};

fn mixed_snapshot() -> SemanticSnapshot {
    let mut snapshot = snapshot();
    let mut window = entity(
        "window-a",
        "/World/Window_A",
        Some("Windows"),
        Some("Fixed Window"),
        Some("Window"),
        vec![property(
            "Mark",
            CanonicalValue::Text("WINDOW-A".to_owned()),
            None,
        )],
    );
    window.semantic.bim.element_id = Some("window-a-id".to_owned());
    snapshot.entities.insert(window.key.clone(), window);

    let mut unclassified = entity(
        "bim-missing-category",
        "/World/Window_B",
        None,
        Some("Fixed Window"),
        Some("Window"),
        vec![
            property("Mark", CanonicalValue::Text("WINDOW-B".to_owned()), None),
            property(
                "WindowOnly",
                CanonicalValue::Text("window-only".to_owned()),
                None,
            ),
        ],
    );
    unclassified.semantic.bim.element_id = Some("window-b-id".to_owned());
    snapshot
        .entities
        .insert(unclassified.key.clone(), unclassified);

    for (key, path, category, type_name) in [
        ("camera", "/World/Camera", "Cameras", "Camera"),
        ("light", "/World/Light", "Lights", "DistantLight"),
        ("helper", "/World/Helper", "Helpers", "Xform"),
        ("plain-mesh", "/World/PlainMesh", "Geometry", "Mesh"),
    ] {
        let mut non_bim = entity(
            key,
            path,
            Some(category),
            Some("Generic"),
            Some(type_name),
            vec![property(
                "NonBimOnly",
                CanonicalValue::Text("helper-data".to_owned()),
                None,
            )],
        );
        non_bim.semantic.bim = Default::default();
        snapshot.entities.insert(non_bim.key.clone(), non_bim);
    }
    snapshot
}

fn recipe() -> ClassificationRecipe {
    ClassificationRecipe::new(vec![
        ClassificationLevel::new("category", BimFieldKey::Category),
        ClassificationLevel::new("type", BimFieldKey::Type),
    ])
}

#[test]
fn mixed_projection_keeps_non_bim_out_of_hierarchy_search_and_colors() {
    let snapshot = mixed_snapshot();
    let recipe = recipe();
    let mut service = BimReadService::new(&snapshot);

    let hierarchy = service
        .classification_snapshot(&recipe)
        .expect("classification snapshot");
    let anchors = hierarchy
        .nodes
        .iter()
        .filter_map(|node| node.anchor.as_ref().map(|anchor| anchor.prim_path.as_str()))
        .collect::<Vec<_>>();
    assert!(anchors.iter().all(|path| !path.contains("Camera")));
    assert!(anchors.iter().all(|path| !path.contains("PlainMesh")));
    assert!(
        hierarchy
            .nodes
            .iter()
            .any(|node| node.name == UNCLASSIFIED_LABEL)
    );
    assert!(hierarchy.nodes.iter().any(|node| {
        node.anchor
            .as_ref()
            .is_some_and(|anchor| anchor.prim_path == "/World/Window_B")
    }));

    let search = service
        .search(&BimSearchQuery::PropertyNameRegex {
            pattern: "^NonBimOnly$".to_owned(),
            page: BimPageRequest::new(0, 20),
        })
        .expect("BIM property-name search");
    let BimSearchResult::PropertyNames { total, matches, .. } = search else {
        panic!("expected property-name search result");
    };
    assert_eq!(total, 0);
    assert!(matches.is_empty());

    let colors = service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("category".to_owned()),
                generation: 0,
            },
        )
        .expect("BIM classification colors");
    assert_eq!(colors.len(), 5);
    assert!(
        colors
            .iter()
            .all(|entry| !entry.anchor.prim_path.contains("Camera"))
    );
    assert!(
        colors
            .iter()
            .all(|entry| !entry.anchor.prim_path.contains("PlainMesh"))
    );
}

#[test]
fn field_catalogue_is_model_wide_bim_only_and_revision_scoped() {
    let snapshot = mixed_snapshot();
    let service = BimReadService::new(&snapshot);
    let catalogue = service.classification_field_catalogue(42);

    assert_eq!(catalogue.semantic_revision, 42);
    assert!(
        catalogue
            .fields
            .iter()
            .any(|field| field.key == BimFieldKey::Category)
    );
    assert!(
        catalogue
            .fields
            .iter()
            .any(|field| field.key == BimFieldKey::Family)
    );
    assert!(
        catalogue
            .fields
            .iter()
            .any(|field| field.key == BimFieldKey::Type)
    );
    assert!(
        catalogue
            .fields
            .iter()
            .any(|field| field.key == BimFieldKey::property("Mark"))
    );
    assert!(
        catalogue
            .fields
            .iter()
            .any(|field| field.key == BimFieldKey::property("WindowOnly"))
    );
    assert!(
        catalogue
            .fields
            .iter()
            .all(|field| field.key != BimFieldKey::property("NonBimOnly"))
    );
}
