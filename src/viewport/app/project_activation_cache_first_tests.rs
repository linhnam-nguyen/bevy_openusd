use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use tempfile::tempdir;
use usd_bevy::LiveStage;

use super::ProductionActivationWorld;
use crate::project::cache::SceneCacheStore;
use crate::project::cache_contract::{
    SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION, SceneCacheDescriptorV3,
    SceneCacheIndex, SceneCacheState, SceneSpatialIndex,
};
use crate::project::cache_hydration::default_project_cache_config_hash;
use crate::project::service::{ProjectStageActivationTarget, ProjectStagePresentationContext};
use crate::viewport::session::SceneCachePresentation;

#[test]
fn scene_cache_presentation_is_published_before_blocking_stage_open() {
    let directory = tempdir().expect("activation cache fixture directory");
    let scene_id = usd_project::SceneId::new_v4();
    let config_hash = default_project_cache_config_hash();
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, 1, config_hash);
    descriptor.state = SceneCacheState::Partial;
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: Vec::new(),
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(directory.path());
    store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish cache-first fixture");
    let activation = store
        .load_activation(scene_id)
        .expect("load cache-first fixture")
        .expect("cache-first fixture activation");

    let project_id = usd_project::ProjectId::new_v4();
    let command = ProjectActivationCommand::new(
        "cache-first-before-open",
        1,
        project_id,
        ProjectStageTarget::Scene(scene_id),
    );
    let target = ProjectStageActivationTarget {
        project_id,
        target: command.target.clone(),
        project_root: directory.path().to_path_buf(),
        path: directory.path().join("blocking-stage-open.usda"),
        archive_paths: Vec::new(),
        cache_identity: None,
        scene_cache: Some(activation),
        presentation: ProjectStagePresentationContext::default(),
    };
    let mut production = ProductionActivationWorld::new();
    assert!(production.admit("cache-first-session", &command));
    let reply = production.apply("cache-first-session", &command, Ok(Some(target)));
    assert!(
        reply.is_none(),
        "cache-first bootstrap must defer its reply"
    );
    assert_eq!(
        production
            .world()
            .resource::<SceneCachePresentation>()
            .generation,
        1
    );
    assert!(production.world().get_non_send::<LiveStage>().is_none());
    production.update();
    assert!(production.world().get_non_send::<LiveStage>().is_none());
    assert!(
        !production
            .world()
            .resource::<crate::viewport::session::CachePresentationGate>()
            .rendered()
    );
    assert!(
        production
            .world()
            .resource::<crate::viewport::api::RenderServerInterface>()
            .shared()
            .take_project_activation_result(&viewport_protocol::SessionId::new(
                "cache-first-session"
            ))
            .is_none()
    );
    production.mark_cache_rendered_for_test();
    production.update();
    let reply = production
        .world()
        .resource::<crate::viewport::api::RenderServerInterface>()
        .shared()
        .take_project_activation_result(&viewport_protocol::SessionId::new("cache-first-session"))
        .expect("deferred Stage-open failure publishes a reply");
    assert!(matches!(
        reply.reply.result,
        project_protocol::ProjectActivationResult::Failed { .. }
    ));
    assert!(
        production
            .world()
            .get_resource::<SceneCachePresentation>()
            .is_none()
    );
    assert!(
        production
            .world()
            .get_resource::<crate::viewport::session::PendingCanonicalVisualHandoff>()
            .is_none()
    );
}

#[test]
fn stale_cache_first_continuation_rolls_back_without_replacing_newer_authority() {
    let directory = tempdir().expect("activation cache fixture directory");
    let scene_id = usd_project::SceneId::new_v4();
    let config_hash = default_project_cache_config_hash();
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, 1, config_hash);
    descriptor.state = SceneCacheState::Partial;
    let index = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: Vec::new(),
    };
    let spatial = SceneSpatialIndex {
        schema_version: SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
        scene_id,
        generation: 1,
        entries: Vec::new(),
    };
    let store = SceneCacheStore::new(directory.path());
    store
        .publish_generation(&descriptor, &index, &spatial)
        .expect("publish stale-continuation fixture");
    let activation = store
        .load_activation(scene_id)
        .expect("load stale-continuation fixture")
        .expect("stale-continuation fixture activation");
    let project_id = usd_project::ProjectId::new_v4();
    let first = ProjectActivationCommand::new(
        "cache-first-stale",
        1,
        project_id,
        ProjectStageTarget::Scene(scene_id),
    );
    let target = ProjectStageActivationTarget {
        project_id,
        target: first.target.clone(),
        project_root: directory.path().to_path_buf(),
        path: directory.path().join("stale-stage-open.usda"),
        archive_paths: Vec::new(),
        cache_identity: None,
        scene_cache: Some(activation),
        presentation: ProjectStagePresentationContext::default(),
    };
    let newer = ProjectActivationCommand::new(
        "cache-first-newer",
        2,
        project_id,
        ProjectStageTarget::ProjectRoot(usd_project::ProjectRoot::Empty),
    );
    let mut production = ProductionActivationWorld::new();
    assert!(production.admit("cache-first-session", &first));
    assert!(
        production
            .apply("cache-first-session", &first, Ok(Some(target)))
            .is_none()
    );
    assert!(production.admit("cache-first-session", &newer));
    production.update();
    production.mark_cache_rendered_for_test();
    production.update();
    let reply = production
        .world()
        .resource::<crate::viewport::api::RenderServerInterface>()
        .shared()
        .take_project_activation_result(&viewport_protocol::SessionId::new("cache-first-session"))
        .expect("stale continuation publishes a reply");
    assert!(matches!(
        reply.reply.result,
        project_protocol::ProjectActivationResult::Failed { .. }
    ));
    assert!(
        production
            .world()
            .get_resource::<SceneCachePresentation>()
            .is_none()
    );
}
