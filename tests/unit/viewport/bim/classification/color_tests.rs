use super::*;
use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe};

use crate::viewport::bim::BimReadService;
use crate::viewport::bim::test_fixtures::snapshot;

#[test]
fn profile_uses_defined_level_and_value_mapping_for_unpaged_entities() {
    let snapshot = snapshot();
    let recipe = ClassificationRecipe::new(vec![
        ClassificationLevel::new("category", BimFieldKey::Category),
        ClassificationLevel::new("type", BimFieldKey::Type),
    ]);
    let mut service = BimReadService::new(&snapshot);
    let entries = service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Profile("default".to_owned()),
                active_level: Some("category".to_owned()),
                generation: 0,
            },
        )
        .expect("profile plan");
    assert_eq!(entries.len(), snapshot.entities.len());
    assert!(entries.iter().any(|entry| {
        entry.anchor.prim_path.contains("Wall") && entry.color == ColorRgb8::new(0x38, 0xBD, 0xF8)
    }));
}

#[test]
fn active_level_changes_group_color_identity() {
    let snapshot = snapshot();
    let recipe = ClassificationRecipe::new(vec![
        ClassificationLevel::new("category", BimFieldKey::Category),
        ClassificationLevel::new("type", BimFieldKey::Type),
    ]);
    let mut category_service = BimReadService::new(&snapshot);
    let category = category_service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("category".to_owned()),
                generation: 3,
            },
        )
        .expect("category plan");
    let mut type_service = BimReadService::new(&snapshot);
    let types = type_service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("type".to_owned()),
                generation: 3,
            },
        )
        .expect("type plan");
    assert_ne!(category, types);
}

#[test]
fn auto_changes_only_when_the_explicit_generation_changes() {
    let snapshot = snapshot();
    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category",
        BimFieldKey::Category,
    )]);
    let mut first_service = BimReadService::new(&snapshot);
    let first = first_service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("category".to_owned()),
                generation: 7,
            },
        )
        .expect("first auto plan");
    let mut same_service = BimReadService::new(&snapshot);
    let same = same_service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("category".to_owned()),
                generation: 7,
            },
        )
        .expect("same auto plan");
    let mut refreshed_service = BimReadService::new(&snapshot);
    let refreshed = refreshed_service
        .classification_color_entries(
            &recipe,
            &ClassificationColorIntent {
                source: ClassificationColorSource::Auto,
                active_level: Some("category".to_owned()),
                generation: 8,
            },
        )
        .expect("refreshed auto plan");
    assert_eq!(first, same);
    assert_ne!(first, refreshed);
}
