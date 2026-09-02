use anyhow::{Result, bail};
use bevy::prelude::*;
use usd_model::{CanonicalValue, SnapshotSource};
use viewport_protocol::{
    BimFieldKey, BimPageRequest, BimSearchQuery, BimSearchResult, ClassificationLevel,
    ClassificationRecipe, ViewportCommand, ViewportEvent,
};

use crate::viewport::api::bridge::scene_query::{
    dispatch_scene_query_commands, publish_scene_query_results, refresh_active_hierarchy_projection,
};
use crate::viewport::api::bridge::state::{SceneSearchRequest, SceneSearchRequests};
use crate::viewport::api::scene_query::SceneQueryService;
use crate::viewport::api::{
    ActiveHierarchyProvider, BimClassificationRecipeState, CurrentHierarchyProjection,
    ViewportCommandInbox, ViewportEventOutbox,
};
use crate::viewport::semantic::SemanticDiffState;
use crate::viewport::semantic::SemanticSyncState;
use crate::viewport::session::StageInfo;

use super::m3_integration_support::*;
use super::support::runtime_semantic_test_app;

#[test]
fn live_edit_converges_into_bim_classification_search_and_diff() -> Result<()> {
    let project_root = tempfile::tempdir()?;
    let mut app = runtime_semantic_test_app(project_root.path().to_path_buf());
    app.world_mut()
        .insert_non_send(usd_bevy::LiveStage::new(stage_with_widths()));
    app.init_resource::<ActiveHierarchyProvider>()
        .init_resource::<BimClassificationRecipeState>()
        .init_resource::<SceneQueryService>()
        .init_resource::<SceneSearchRequests>();
    app.add_systems(
        Update,
        (publish_scene_query_results, dispatch_scene_query_commands)
            .chain()
            .before(crate::viewport::api::bridge::commands::apply_viewport_commands),
    );
    app.add_systems(
        PostUpdate,
        refresh_active_hierarchy_projection
            .after(crate::viewport::semantic::synchronize_live_stage),
    );

    let initial = wait_for_initial_semantics(&mut app)?;
    assert_eq!(
        app.world()
            .resource::<SemanticSyncState>()
            .config()
            .nvidia_revit
            .identity
            .element_id_property
            .as_deref(),
        Some("BIM:Instance:ElementId")
    );
    assert!(
        initial
            .entities
            .values()
            .any(|entity| { entity.semantic.bim.element_id.as_deref() == Some("A") })
    );
    let mut baseline = initial.clone();
    baseline.source = SnapshotSource::GitCommit {
        oid: "m8-consistency-baseline".to_owned(),
    };
    assert!(
        app.world_mut()
            .resource_mut::<SemanticDiffState>()
            .set_git_baseline(baseline)
    );

    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "width",
        BimFieldKey::property("Width"),
    )]);
    let contextual_source = app.world().resource::<ActiveHierarchyProvider>().source();
    assert_eq!(contextual_source, viewport_protocol::HierarchySource::Prim);
    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetBimClassificationRecipe {
            recipe: Some(recipe.clone()),
        },
    );
    app.update();
    assert_eq!(
        app.world()
            .resource::<BimClassificationRecipeState>()
            .recipe(),
        Some(&recipe)
    );
    assert_eq!(
        app.world().resource::<ActiveHierarchyProvider>().source(),
        contextual_source
    );
    assert_eq!(
        app.world()
            .resource::<CurrentHierarchyProjection>()
            .source(),
        contextual_source
    );

    let initial_search_request =
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SearchBim {
                query: BimSearchQuery::ObjectPropertyMatch {
                    property: "Width".to_owned(),
                    pattern: "^1$".to_owned(),
                    page: BimPageRequest::new(0, 20),
                },
            });
    let initial_search_event = wait_for_event(&mut app, &initial_search_request)?;
    let ViewportEvent::BimSearchResults {
        result: BimSearchResult::Objects { total, matches, .. },
    } = initial_search_event.event
    else {
        bail!("expected initial BIM classification search results")
    };
    assert_eq!(total, 1);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].anchor.prim_path, "/World/A");

    select_targets(&mut app, &["/World/A"]);
    let previous_revision = app
        .world()
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .current_revision()
        .0;
    let edit_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::EditBimProperty {
            mutation: mutation("/World/A", 1.0, 10.0),
        },
    );
    let (edit_event, change_batch) = wait_for_change(&mut app, &edit_request, previous_revision)?;
    assert!(matches!(
        edit_event.event,
        ViewportEvent::BimPropertyEditCompleted { outcome, .. }
            if outcome.status == viewport_protocol::BimPropertyEditStatus::Applied
    ));
    assert!(
        change_batch
            .changes
            .iter()
            .flat_map(|change| &change.changed_info)
            .any(|path| path == "/World/A.Width")
    );
    assert_eq!(read_width(&app, "/World/A"), 10.0);
    assert_eq!(semantic_width(&app, "/World/A"), CanonicalValue::Real(10.0));
    assert_modified_diff(&app, "/World/A");

    assert_eq!(
        app.world().resource::<ActiveHierarchyProvider>().source(),
        contextual_source
    );
    assert_eq!(
        app.world()
            .resource::<CurrentHierarchyProjection>()
            .source(),
        contextual_source
    );
    assert_eq!(
        app.world()
            .resource::<BimClassificationRecipeState>()
            .recipe(),
        Some(&recipe)
    );

    let properties_request = app
        .world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::RequestBimProperties);
    let properties_event = wait_for_event(&mut app, &properties_request)?;
    let ViewportEvent::BimPropertiesRead { properties, .. } = properties_event.event else {
        bail!("expected BIM property read after semantic convergence")
    };
    let width = properties
        .groups
        .iter()
        .flat_map(|group| &group.properties)
        .find(|property| property.key == "Width")
        .expect("Width property in BIM read model");
    assert_eq!(width.target_values, vec![CanonicalValue::Real(10.0)]);

    let search_request =
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SearchBim {
                query: BimSearchQuery::ObjectPropertyMatch {
                    property: "Width".to_owned(),
                    pattern: "10".to_owned(),
                    page: BimPageRequest::new(0, 20),
                },
            });
    let search_event = wait_for_event(&mut app, &search_request)?;
    let ViewportEvent::BimSearchResults { result } = search_event.event else {
        bail!("expected BIM search after semantic convergence")
    };
    assert!(matches!(
        result,
        BimSearchResult::Objects { total: 1, matches, .. }
            if matches.len() == 1 && matches[0].anchor.prim_path == "/World/A"
    ));
    Ok(())
}

#[test]
fn stale_bim_search_result_is_dropped_after_project_activation() {
    let semantic =
        SemanticSyncState::from_test_snapshot(crate::viewport::bim::test_fixtures::snapshot());
    let snapshot = semantic.shared_snapshot().expect("fixture snapshot");
    let index = semantic.shared_bim_index().expect("fixture BIM index");
    let service = SceneQueryService::default();
    assert!(service.submit_bim_search(
        "stale-bim-generation".to_owned(),
        BimSearchQuery::PropertyNameRegex {
            pattern: ".*".to_owned(),
            page: BimPageRequest::new(0, 20),
        },
        snapshot,
        index,
        1,
    ));

    let mut pending = SceneSearchRequests::default();
    pending.pending.insert(
        "stale-bim-generation".to_owned(),
        SceneSearchRequest {
            query: "bim".to_owned(),
            offset: 0,
            submitted_at: std::time::Instant::now(),
        },
    );
    let mut app = App::new();
    app.insert_resource(service)
        .insert_resource(pending)
        .init_resource::<ViewportEventOutbox>()
        .insert_resource(StageInfo {
            activation_generation: 2,
            ..default()
        })
        .add_systems(Update, publish_scene_query_results);

    for _ in 0..200 {
        app.update();
        if app
            .world()
            .resource::<SceneSearchRequests>()
            .pending
            .is_empty()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert!(
        app.world()
            .resource::<SceneSearchRequests>()
            .pending
            .is_empty()
    );
    assert!(
        app.world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .is_none()
    );
}
