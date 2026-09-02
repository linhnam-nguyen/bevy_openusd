use bevy::prelude::*;
use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe};

use crate::viewport::api::ActiveHierarchyProvider;
use crate::viewport::bim::test_fixtures::snapshot;
use crate::viewport::scene::{ClassificationColorPlan, refresh_classification_color_plan};
use crate::viewport::semantic::SemanticSyncState;

#[test]
fn color_plan_clears_stale_entries_when_semantic_input_is_unavailable() {
    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category",
        BimFieldKey::Category,
    )]);
    let mut provider = ActiveHierarchyProvider::default();
    provider.set(
        viewport_protocol::HierarchySource::BimClassification,
        Some(recipe),
    );

    let mut app = App::new();
    app.insert_resource(provider)
        .insert_resource(SemanticSyncState::from_test_snapshot(snapshot()))
        .init_resource::<ClassificationColorPlan>()
        .add_systems(Update, refresh_classification_color_plan);
    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .accept_intent(viewport_protocol::ClassificationColorIntent {
            source: viewport_protocol::ClassificationColorSource::Auto,
            active_level: Some("category".to_owned()),
            generation: 1,
        })
        .expect("initial color intent");

    app.update();
    let first_revision = app.world().resource::<ClassificationColorPlan>().revision();
    assert!(
        !app.world()
            .resource::<ClassificationColorPlan>()
            .entries()
            .is_empty()
    );

    app.world_mut().remove_resource::<SemanticSyncState>();
    app.world_mut()
        .resource_mut::<ActiveHierarchyProvider>()
        .set(viewport_protocol::HierarchySource::BimClassification, None);
    app.update();

    let plan = app.world().resource::<ClassificationColorPlan>();
    assert!(plan.entries().is_empty());
    assert!(plan.revision() > first_revision);
}
