use super::*;
use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe};

use crate::viewport::api::ActiveHierarchyProvider;
use crate::viewport::bim::test_fixtures::snapshot;
use crate::viewport::semantic::SemanticSyncState;

fn intent(generation: u64) -> ClassificationColorIntent {
    ClassificationColorIntent {
        source: ClassificationColorSource::Auto,
        active_level: Some("category".to_owned()),
        generation,
    }
}

#[test]
fn intent_generation_is_monotonic_and_unchanged_intent_is_idle() {
    let mut plan = ClassificationColorPlan::default();
    assert!(plan.accept_intent(intent(4)).expect("first intent"));
    assert!(!plan.accept_intent(intent(4)).expect("same intent"));
    assert!(plan.accept_intent(intent(5)).expect("new generation"));
    assert!(plan.accept_intent(intent(3)).is_err());
    assert_eq!(plan.intent_revision(), 2);
}

#[test]
fn backend_builds_complete_unpaged_plan_once_and_stays_idle() {
    let mut provider = ActiveHierarchyProvider::default();
    provider.set(
        viewport_protocol::HierarchySource::BimClassification,
        Some(ClassificationRecipe::new(vec![ClassificationLevel::new(
            "category",
            BimFieldKey::Category,
        )])),
    );
    let snapshot = snapshot();
    let entity_count = snapshot.entities.len();
    let mut app = App::new();
    app.insert_resource(provider)
        .insert_resource(SemanticSyncState::from_test_snapshot(snapshot))
        .init_resource::<ClassificationColorPlan>()
        .add_systems(Update, refresh_classification_color_plan);
    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .accept_intent(ClassificationColorIntent {
            source: ClassificationColorSource::Profile("default".to_owned()),
            active_level: Some("category".to_owned()),
            generation: 0,
        })
        .expect("initial color intent");

    app.update();
    let first_revision = app.world().resource::<ClassificationColorPlan>().revision();
    assert_eq!(
        app.world()
            .resource::<ClassificationColorPlan>()
            .entries()
            .len(),
        entity_count
    );
    app.update();
    assert_eq!(
        app.world().resource::<ClassificationColorPlan>().revision(),
        first_revision
    );
}

#[test]
fn backend_builds_color_plan_from_bim_recipe_without_contextual_provider() {
    let snapshot = snapshot();
    let entity_count = snapshot.entities.len();
    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category",
        BimFieldKey::Category,
    )]);
    let mut app = App::new();
    app.insert_resource(BimClassificationRecipeState::default())
        .insert_resource(SemanticSyncState::from_test_snapshot(snapshot))
        .init_resource::<ClassificationColorPlan>()
        .add_systems(Update, refresh_classification_color_plan);
    app.world_mut()
        .resource_mut::<BimClassificationRecipeState>()
        .set(Some(recipe));
    app.world_mut()
        .resource_mut::<ClassificationColorPlan>()
        .accept_intent(intent(0))
        .expect("initial color intent");

    app.update();

    assert_eq!(
        app.world()
            .resource::<ClassificationColorPlan>()
            .entries()
            .len(),
        entity_count
    );
}
