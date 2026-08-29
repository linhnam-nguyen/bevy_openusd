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
use crate::viewport::api::bridge::state::SceneSearchRequests;
use crate::viewport::api::scene_query::SceneQueryService;
use crate::viewport::api::{
    ActiveHierarchyProvider, CurrentHierarchyProjection, ViewportCommandInbox,
};
use crate::viewport::semantic::SemanticDiffState;

use super::m3_integration_support::*;
use super::support::runtime_semantic_test_app;

#[test]
fn live_edit_converges_into_bim_classification_search_and_diff() -> Result<()> {
    let project_root = tempfile::tempdir()?;
    let mut app = runtime_semantic_test_app(project_root.path().to_path_buf());
    app.world_mut()
        .insert_non_send(usd_bevy::LiveStage::new(stage_with_widths()));
    app.init_resource::<ActiveHierarchyProvider>()
        .init_resource::<SceneQueryService>()
        .init_resource::<SceneSearchRequests>();
    app.add_systems(
        Update,
        (publish_scene_query_results, dispatch_scene_query_commands).chain(),
    );
    app.add_systems(
        PostUpdate,
        refresh_active_hierarchy_projection
            .after(crate::viewport::semantic::synchronize_live_stage),
    );

    let initial = wait_for_initial_semantics(&mut app)?;
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
    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetHierarchySource {
            source: viewport_protocol::HierarchySource::BimClassification,
            classification_recipe: Some(recipe),
        },
    );
    app.update();
    let initial_projection = app
        .world()
        .resource::<CurrentHierarchyProjection>()
        .snapshot()
        .nodes
        .iter()
        .map(|node| node.name.clone())
        .collect::<Vec<_>>();
    assert!(initial_projection.iter().any(|name| name == "1"));

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

    let projection = app
        .world()
        .resource::<CurrentHierarchyProjection>()
        .snapshot();
    assert!(projection.nodes.iter().any(|node| node.name == "10"));
    assert!(!projection.nodes.iter().any(|node| node.name == "1"));

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
