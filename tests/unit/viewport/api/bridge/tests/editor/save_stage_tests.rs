#[cfg(test)]
mod tests {
    use openusd::usd::Stage;
    use viewport_protocol::*;

    use crate::cache::hydration::{
        ActiveProjectCacheContext, default_project_cache_config_hash,
    };
    use crate::cache::warming::ProjectCacheWarmQueue;
    use crate::cache::{ProjectCacheTarget, SceneCacheStore};
    use crate::api::bridge::state::{EditorHistories, EditorHistoryDomain};
    use crate::api::{ViewportCommandInbox, ViewportEventOutbox};
    use crate::viewport::stage::{SceneCacheOwnershipContext, StageHandle};

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

        assert!(
            app.world()
                .get_non_send::<usd_bevy::LiveStage>()
                .unwrap()
                .try_freeze_authoring()
        );
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStage);
        app.update();
        assert!(!path.exists(), "a save must honor an active publication lease");
        assert!(app.world_mut().resource_mut::<ViewportEventOutbox>().pop().is_some());
        let live = app.world().get_non_send::<usd_bevy::LiveStage>().unwrap();
        assert!(live.is_authoring_frozen());
        live.unfreeze_authoring();

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
                .expect("saved prim exists")
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
        )
        .unwrap();
        let path =
            crate::project::scene::authoring::author_scene_atomic(directory.path(), root_scene)
                .unwrap();
        crate::project::scene::authoring::author_scene_atomic(directory.path(), sibling).unwrap();
        let config = default_project_cache_config_hash();
        let store = SceneCacheStore::new(directory.path());
        store.advance_generation(root_scene, config).unwrap();
        store.advance_generation(sibling, config).unwrap();
        let context = ActiveProjectCacheContext::new(
            directory.path().to_path_buf(),
            ProjectCacheTarget::Scene {
                id: root_scene.to_string(),
            },
            viewport_protocol::RuntimeProfile::NativeMedium,
            config,
        )
        .unwrap();
        let stage = Stage::open(path.to_string_lossy().as_ref()).unwrap();
        stage
            .define_prim("/SceneRoot/SavedChange")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage));
        app.world_mut().insert_resource(StageHandle {
            path: path.clone(),
            error: None,
        });
        app.world_mut().insert_resource(context);
        (directory, app, root_scene, sibling, path)
    }

    #[test]
    fn canonical_project_scene_save_advances_only_owning_scene_generation() {
        let (directory, mut app, scene_id, sibling, _) = project_scene_app();
        let store = SceneCacheStore::new(directory.path());
        let before = store.load_descriptor(scene_id).unwrap().unwrap().generation;
        let sibling_before = store.load_descriptor(sibling).unwrap().unwrap().generation;
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStage);
        app.update();
        assert!(store.load_descriptor(scene_id).unwrap().unwrap().generation > before);
        assert_eq!(
            store.load_descriptor(sibling).unwrap().unwrap().generation,
            sibling_before
        );
    }

    #[test]
    fn canonical_project_scene_save_advances_generation_without_legacy_cache_context() {
        let (directory, mut app, scene_id, _, _) = project_scene_app();
        let store = SceneCacheStore::new(directory.path());
        let before = store.load_descriptor(scene_id).unwrap().unwrap().generation;
        let config_hash = default_project_cache_config_hash();
        app.world_mut()
            .remove_resource::<ActiveProjectCacheContext>();
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
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStageAs {
                filename: external.to_string_lossy().into_owned(),
            });
        app.update();
        assert_eq!(
            store.load_descriptor(scene_id).unwrap().unwrap().generation,
            before
        );

        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStageAs {
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
        stage
            .define_prim("/SceneRoot/BackpressureSave")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let live = usd_bevy::LiveStage::new(stage);
        let queue = ProjectCacheWarmQueue::default();
        queue.shutdown_without_waiting();
        let mut outbox = ViewportEventOutbox::default();
        let mut histories = EditorHistories::default();
        crate::api::bridge::editor::save_current_stage(
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
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::SaveStage,
                ..
            }
        ));
    }

    fn create_directory_alias(link: &std::path::Path, target: &std::path::Path) {
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    link.to_str().expect("utf-8 link"),
                    target.to_str().expect("utf-8 target"),
                ])
                .status()
                .expect("run mklink /J");
            assert!(status.success(), "mklink /J failed");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).expect("create directory symlink");
        }
    }

    #[test]
    fn save_stage_as_rejects_directory_alias_escape_and_allows_genuine_external_export() {
        let (directory, mut app, scene_id, _, canonical) = project_scene_app();
        let outside = tempfile::tempdir().expect("outside tempdir");
        let existing_outside = outside.path().join("existing.usda");
        let sentinel = b"#usda 1.0\n# untouched external file\n";
        std::fs::write(&existing_outside, sentinel).expect("write sentinel");

        let alias_dir = directory.path().join("exports");
        create_directory_alias(&alias_dir, outside.path());

        let store = SceneCacheStore::new(directory.path());
        let before_gen = store.load_descriptor(scene_id).unwrap().unwrap().generation;

        app.world_mut()
            .resource_mut::<EditorHistories>()
            .record(EditorHistoryDomain::Authoring);

        // 1. Existing file reached via directory alias inside Project -> rejected.
        let escaped_existing = alias_dir.join("existing.usda");
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStageAs {
                filename: escaped_existing.to_string_lossy().into_owned(),
            });
        app.update();
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("escape rejection event");
        assert!(matches!(event.event, ViewportEvent::CommandRejected { .. }));
        assert_eq!(std::fs::read(&existing_outside).unwrap(), sentinel);
        assert!(
            app.world().resource::<EditorHistories>().is_dirty,
            "dirty state must be preserved on rejected alias save"
        );

        // 2. New file reached via directory alias inside Project -> rejected.
        let escaped_new = alias_dir.join("new_via_alias.usda");
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStageAs {
                filename: escaped_new.to_string_lossy().into_owned(),
            });
        app.update();
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("new alias rejection event");
        assert!(matches!(event.event, ViewportEvent::CommandRejected { .. }));
        assert!(!outside.path().join("new_via_alias.usda").exists());
        assert!(app.world().resource::<EditorHistories>().is_dirty);

        // 3. Genuine external SaveStageAs -> succeeds without mutating Project cache.
        let genuine_external = outside.path().join("genuine_export.usda");
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStageAs {
                filename: genuine_external.to_string_lossy().into_owned(),
            });
        app.update();
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("external export completion event");
        assert!(matches!(
            event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::SaveStageAs,
                ..
            }
        ));
        assert!(genuine_external.is_file());
        assert_eq!(
            store.load_descriptor(scene_id).unwrap().unwrap().generation,
            before_gen
        );

        // 4. Subsequent managed SaveStage to canonical Scene -> succeeds cleanly.
        app.world_mut()
            .resource_mut::<EditorHistories>()
            .record(EditorHistoryDomain::Authoring);
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStageAs {
                filename: canonical.to_string_lossy().into_owned(),
            });
        app.update();
        let event = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("managed save completion event");
        assert!(matches!(
            event.event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::SaveStageAs,
                ..
            }
        ));
        assert!(store.load_descriptor(scene_id).unwrap().unwrap().generation > before_gen);
        assert!(!app.world().resource::<EditorHistories>().is_dirty);
    }

    #[test]
    fn save_stage_as_protects_unicode_equivalent_root_spelling_and_allows_sibling_prefix() {
        let base = tempfile::tempdir().expect("base tempdir");
        let lower_root = base.path().join("Pröject");
        let upper_root = base.path().join("PrÖject");
        let sibling_external = base.path().join("Pröject_external");
        let outside = base.path().join("OutsideTarget");
        std::fs::create_dir_all(&lower_root).expect("create lower_root");
        std::fs::create_dir_all(&sibling_external).expect("create sibling_external");
        std::fs::create_dir_all(&outside).expect("create outside");

        let root_scene = usd_project::SceneId::new_v4();
        let manifest = usd_project::ProjectManifestV1::new(
            usd_project::ProjectId::new_v4(),
            "Unicode Project",
            usd_project::ProjectRoot::Scene(root_scene),
            vec![usd_project::SceneManifestEntry {
                id: root_scene,
                storage_key: usd_project::StorageKey::new("Root").unwrap(),
                display_name: "Root".to_owned(),
            }],
            Vec::new(),
        );
        crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
            &lower_root,
            &manifest,
        )
        .unwrap();
        let canonical =
            crate::project::scene::authoring::author_scene_atomic(&lower_root, root_scene).unwrap();
        let config = default_project_cache_config_hash();
        let store = SceneCacheStore::new(&lower_root);
        store.advance_generation(root_scene, config).unwrap();
        let before_gen = store.load_descriptor(root_scene).unwrap().unwrap().generation;

        let context = ActiveProjectCacheContext::new(
            lower_root.clone(),
            ProjectCacheTarget::Scene {
                id: root_scene.to_string(),
            },
            viewport_protocol::RuntimeProfile::NativeMedium,
            config,
        )
        .unwrap();
        let stage = Stage::open(canonical.to_string_lossy().as_ref()).unwrap();
        stage
            .define_prim("/SceneRoot/UnicodeChange")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        let mut app = command_test_app();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage));
        app.world_mut().insert_resource(StageHandle {
            path: canonical.clone(),
            error: None,
        });
        app.world_mut().insert_resource(context);

        let existing_outside = outside.join("sentinel.usda");
        let sentinel = b"#usda 1.0\n# untouched unicode sentinel\n";
        std::fs::write(&existing_outside, sentinel).expect("write sentinel");
        create_directory_alias(&lower_root.join("exports"), &outside);

        // 1. Sibling prefix boundary (`Pröject_external`) is genuinely external -> succeeds.
        let sibling_export = sibling_external.join("out.usda");
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::SaveStageAs {
                filename: sibling_export.to_string_lossy().into_owned(),
            });
        app.update();
        assert!(matches!(
            app.world_mut().resource_mut::<ViewportEventOutbox>().pop().unwrap().event,
            ViewportEvent::EditorCommandCompleted {
                operation: EditorOperation::SaveStageAs,
                ..
            }
        ));
        assert!(sibling_export.is_file());
        assert_eq!(
            store.load_descriptor(root_scene).unwrap().unwrap().generation,
            before_gen
        );

        // 2. On filesystems where `PrÖject` and `Pröject` resolve to the same directory
        //    (e.g. Windows NTFS), verify both existing & new alias escapes are rejected
        //    and equivalent-spelling managed save succeeds.
        let unicode_equivalent = std::fs::canonicalize(&upper_root)
            .ok()
            .zip(std::fs::canonicalize(&lower_root).ok())
            .is_some_and(|(a, b)| a == b);
        #[cfg(windows)]
        assert!(
            unicode_equivalent,
            "Windows fixture expects PrÖject and Pröject to resolve to the same directory"
        );
        if unicode_equivalent {
            app.world_mut()
                .resource_mut::<EditorHistories>()
                .record(EditorHistoryDomain::Authoring);

            // Existing file via alternate-case Unicode root + alias -> rejected.
            let escaped_existing = upper_root.join("exports").join("sentinel.usda");
            app.world_mut()
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::SaveStageAs {
                    filename: escaped_existing.to_string_lossy().into_owned(),
                });
            app.update();
            assert!(matches!(
                app.world_mut().resource_mut::<ViewportEventOutbox>().pop().unwrap().event,
                ViewportEvent::CommandRejected { .. }
            ));
            assert_eq!(std::fs::read(&existing_outside).unwrap(), sentinel);
            assert!(app.world().resource::<EditorHistories>().is_dirty);

            // New file via alternate-case Unicode root + alias -> rejected.
            let escaped_new = upper_root.join("exports").join("new_escape.usda");
            app.world_mut()
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::SaveStageAs {
                    filename: escaped_new.to_string_lossy().into_owned(),
                });
            app.update();
            assert!(matches!(
                app.world_mut().resource_mut::<ViewportEventOutbox>().pop().unwrap().event,
                ViewportEvent::CommandRejected { .. }
            ));
            assert!(!outside.join("new_escape.usda").exists());
            assert!(app.world().resource::<EditorHistories>().is_dirty);

            // Managed save via alternate-case Unicode root spelling -> succeeds canonically.
            let relative_scene = canonical.strip_prefix(&lower_root).expect("scene under root");
            let upper_canonical = upper_root.join(relative_scene);
            app.world_mut()
                .resource_mut::<ViewportCommandInbox>()
                .send(ViewportCommand::SaveStageAs {
                    filename: upper_canonical.to_string_lossy().into_owned(),
                });
            app.update();
            assert!(matches!(
                app.world_mut().resource_mut::<ViewportEventOutbox>().pop().unwrap().event,
                ViewportEvent::EditorCommandCompleted {
                    operation: EditorOperation::SaveStageAs,
                    ..
                }
            ));
            assert!(store.load_descriptor(root_scene).unwrap().unwrap().generation > before_gen);
            assert!(!app.world().resource::<EditorHistories>().is_dirty);
        }
    }
}
