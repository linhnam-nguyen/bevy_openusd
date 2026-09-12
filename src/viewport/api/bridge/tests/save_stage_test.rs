#[cfg(test)]
mod tests {
    use openusd::usd::Stage;
    use viewport_protocol::*;

    use crate::project::cache::{ProjectCacheTarget, SceneCacheStore};
    use crate::project::cache_warmer::ProjectCacheWarmQueue;
    use crate::project::cache_hydration::{ActiveProjectCacheContext, default_project_cache_config_hash};
    use crate::viewport::api::bridge::state::{EditorHistories, EditorHistoryDomain};
    use crate::viewport::api::{ViewportCommandInbox, ViewportEventOutbox};
    use crate::viewport::session::{SceneCacheOwnershipContext, StageHandle};

    use super::super::support::command_test_app;

    fn stage_with_saved_prim() -> Stage {
        let stage = Stage::builder()
            .in_memory("bim_save_stage_test.usda")
            .expect("stage opens");
        stage
            .define_prim("/World/Saved")
            .expect("prim defines")
            .set_type_name("Xform")
            .expect("prim type authors");
        stage
    }

    #[test]
    fn save_stage_uses_current_stage_path_and_round_trips() {
        let temp_dir = tempfile::tempdir().expect("temporary directory creates");
        let path = temp_dir.path().join("saved.usda");
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage_with_saved_prim()));
        app.world_mut().insert_resource(StageHandle {
            path: path.clone(),
            error: None,
        });
        app.world_mut()
            .resource_mut::<EditorHistories>()
            .record(EditorHistoryDomain::Authoring);
        let request_id = app
            .world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStage);

        app.update();

        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("save publishes one event");
        assert_eq!(event.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::SaveStage,
                changed_paths,
                state,
                ..
            } if changed_paths.is_empty() && state.can_undo && !state.is_dirty
        ));

        let reopened = Stage::open(path.to_str().expect("temporary path is valid UTF-8"))
            .expect("saved stage reopens");
        assert!(
            reopened
                .prim(openusd::sdf::path("/World/Saved").expect("prim path parses"))
                .type_name()
                .expect("prim type reads")
                .is_some(),
            "saved stage retains authored prim"
        );
    }

    fn project_scene_app() -> (
        tempfile::TempDir,
        bevy::prelude::App,
        usd_project::SceneId,
        usd_project::SceneId,
        std::path::PathBuf,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let root_scene = usd_project::SceneId::new_v4();
        let sibling = usd_project::SceneId::new_v4();
        let manifest = usd_project::ProjectManifestV1::new(
            usd_project::ProjectId::new_v4(),
            "Save Project",
            usd_project::ProjectRoot::Scene(root_scene),
            vec![
                usd_project::SceneManifestEntry {
                    id: root_scene,
                    storage_key: usd_project::StorageKey::new("Root").unwrap(),
                    display_name: "Root".to_owned(),
                },
                usd_project::SceneManifestEntry {
                    id: sibling,
                    storage_key: usd_project::StorageKey::new("Sibling").unwrap(),
                    display_name: "Sibling".to_owned(),
                },
            ],
            Vec::new(),
        );
        crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
            directory.path(),
            &manifest,
        ).unwrap();
        let path = crate::project::scene::authoring::author_scene_atomic(directory.path(), root_scene).unwrap();
        crate::project::scene::authoring::author_scene_atomic(directory.path(), sibling).unwrap();
        let config = default_project_cache_config_hash();
        let store = SceneCacheStore::new(directory.path());
        store.advance_generation(root_scene, config).unwrap();
        store.advance_generation(sibling, config).unwrap();
        let context = ActiveProjectCacheContext::new(
            directory.path().to_path_buf(),
            ProjectCacheTarget::Scene { id: root_scene.to_string() },
            viewport_protocol::RuntimeProfile::NativeMedium,
            config,
        ).unwrap();
        let stage = Stage::open(path.to_string_lossy().as_ref()).unwrap();
        stage.define_prim("/SceneRoot/SavedChange").unwrap().set_type_name("Xform").unwrap();
        let mut app = command_test_app();
        app.world_mut().insert_non_send(usd_bevy::LiveStage::new(stage));
        app.world_mut().insert_resource(StageHandle { path: path.clone(), error: None });
        app.world_mut().insert_resource(context);
        (directory, app, root_scene, sibling, path)
    }

    #[test]
    fn canonical_project_scene_save_advances_only_owning_scene_generation() {
        let (directory, mut app, scene_id, sibling, _) = project_scene_app();
        let store = SceneCacheStore::new(directory.path());
        let before = store.load_descriptor(scene_id).unwrap().unwrap().generation;
        let sibling_before = store.load_descriptor(sibling).unwrap().unwrap().generation;
        app.world_mut().resource_mut::<ViewportCommandInbox>().send(ViewportCommand::SaveStage);
        app.update();
        assert!(store.load_descriptor(scene_id).unwrap().unwrap().generation > before);
        assert_eq!(store.load_descriptor(sibling).unwrap().unwrap().generation, sibling_before);
    }

    #[test]
    fn canonical_project_scene_save_advances_generation_without_legacy_cache_context() {
        let (directory, mut app, scene_id, _, _) = project_scene_app();
        let store = SceneCacheStore::new(directory.path());
        let before = store.load_descriptor(scene_id).unwrap().unwrap().generation;
        let config_hash = default_project_cache_config_hash();
        app.world_mut().remove_resource::<ActiveProjectCacheContext>();
        app.world_mut().insert_resource(SceneCacheOwnershipContext {
            project_root: directory.path().to_path_buf(),
            scene_id,
            config_hash,
        });

        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStage);
        app.update();

        assert!(store.load_descriptor(scene_id).unwrap().unwrap().generation > before);
    }

    #[test]
    fn save_stage_as_invalidates_only_when_destination_is_canonical_project_scene() {
        let (directory, mut app, scene_id, _, canonical) = project_scene_app();
        let store = SceneCacheStore::new(directory.path());
        let before = store.load_descriptor(scene_id).unwrap().unwrap().generation;
        let external = directory.path().join("export-copy.usda");
        app.world_mut().resource_mut::<ViewportCommandInbox>().send(ViewportCommand::SaveStageAs {
            filename: external.to_string_lossy().into_owned(),
        });
        app.update();
        assert_eq!(store.load_descriptor(scene_id).unwrap().unwrap().generation, before);

        app.world_mut().resource_mut::<ViewportCommandInbox>().send(ViewportCommand::SaveStageAs {
            filename: canonical.to_string_lossy().into_owned(),
        });
        app.update();
        assert!(store.load_descriptor(scene_id).unwrap().unwrap().generation > before);
    }


    #[test]
    fn canonical_save_succeeds_and_advances_generation_when_warm_worker_is_unavailable() {
        let (directory, app, scene_id, _, path) = project_scene_app();
        let context = app.world().resource::<ActiveProjectCacheContext>().clone();
        drop(app);
        let store = SceneCacheStore::new(directory.path());
        let before = store.load_descriptor(scene_id).unwrap().unwrap().generation;
        let stage = Stage::open(path.to_string_lossy().as_ref()).unwrap();
        stage.define_prim("/SceneRoot/BackpressureSave").unwrap().set_type_name("Xform").unwrap();
        let live = usd_bevy::LiveStage::new(stage);
        let queue = ProjectCacheWarmQueue::default();
        queue.shutdown_without_waiting();
        let mut outbox = ViewportEventOutbox::default();
        let mut histories = EditorHistories::default();
        crate::viewport::api::bridge::save::save_current_stage(
            "save-backpressure".to_owned(),
            &mut outbox,
            &mut histories,
            Some(&live),
            Some(&path),
            Some(&context),
            None,
            &queue,
        );
        assert!(store.load_descriptor(scene_id).unwrap().unwrap().generation > before);
        assert!(matches!(
            outbox.pop().unwrap().event,
            ViewportEvent::EditorCommandCompleted { operation: EditorOperation::SaveStage, .. }
        ));
    }

}
