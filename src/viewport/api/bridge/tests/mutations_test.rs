#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use viewport_protocol::*;

    use crate::project::recovery::{RecoverySettings, RecoveryStore};
    use crate::project::recovery_worker::{RecoveryRuntime, drain_recovery_results};
    use crate::viewport::animation::UsdStageTime;
    use crate::viewport::api::bridge::commands::apply_viewport_commands;
    use crate::viewport::api::bridge::plugin::checkpoint_recovery;
    use crate::viewport::api::bridge::state::{EditorHistories, RuntimeMutationCoordinator};
    use crate::viewport::api::{
        SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommandInbox,
    };
    use crate::viewport::camera::CameraMount;
    use crate::viewport::physics::PhysicsActive;
    use crate::viewport::scene::SelectedPrim;
    use crate::viewport::scene::visualization::DisplayToggles;
    use crate::viewport::semantic::synchronize_live_stage;
    use crate::viewport::semantic::{
        SemanticDiffState, SemanticFilter, SemanticQuery, SemanticResponse, SemanticSyncState,
        SemanticWorkingStore,
    };
    use crate::viewport::session::{LoaderTuning, ReloadRequest, Spawned, StageInfo};

    fn command_test_app() -> App {
        let mut app = App::new();
        app.init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<ReloadRequest>()
            .init_resource::<SelectedPrim>()
            .init_resource::<CameraMount>()
            .init_resource::<UsdStageTime>()
            .init_resource::<DisplayToggles>()
            .init_resource::<LoaderTuning>()
            .init_resource::<PhysicsActive>()
            .init_resource::<EditorHistories>()
            .init_resource::<RuntimeMutationCoordinator>()
            .init_resource::<Spawned>()
            .insert_resource(StageInfo {
                path: "fixtures/spinner.usda".to_owned(),
                ..default()
            })
            .add_systems(Update, apply_viewport_commands);
        app
    }

    fn runtime_semantic_test_app(project_root: std::path::PathBuf) -> App {
        let mut app = App::new();
        app.add_plugins(usd_bevy::UsdPlugin)
            .add_plugins(usd_bevy::LiveStagePlugin)
            .init_resource::<ViewportCommandInbox>()
            .init_resource::<ViewportEventOutbox>()
            .init_resource::<ViewportTreeCommandInbox>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<ReloadRequest>()
            .init_resource::<SelectedPrim>()
            .init_resource::<CameraMount>()
            .init_resource::<UsdStageTime>()
            .init_resource::<DisplayToggles>()
            .init_resource::<LoaderTuning>()
            .init_resource::<PhysicsActive>()
            .init_resource::<EditorHistories>()
            .init_resource::<RuntimeMutationCoordinator>()
            .init_resource::<Spawned>()
            .init_resource::<SemanticWorkingStore>()
            .init_resource::<SemanticSyncState>()
            .init_resource::<SemanticDiffState>()
            .init_resource::<RecoveryRuntime>()
            .insert_resource(RecoverySettings { project_root })
            .insert_resource(StageInfo {
                path: "runtime-semantic-test.usda".to_owned(),
                ..default()
            })
            .add_systems(Update, apply_viewport_commands)
            .add_systems(
                PostUpdate,
                (
                    synchronize_live_stage,
                    drain_recovery_results,
                    checkpoint_recovery,
                )
                    .chain(),
            );
        app
    }

    #[test]
    fn runtime_mutation_batch_uses_one_writer_and_preserves_revision_guard() {
        let mut app = command_test_app();
        let stage = openusd::usd::Stage::builder()
            .in_memory("bridge_runtime_batch_test.usda")
            .unwrap();
        stage
            .define_prim("/World")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage));

        let batch = RuntimeMutationBatch {
            source_id: "connector-a".to_owned(),
            sequence: 1,
            base_revision: 0,
            operations: vec![
                RuntimeMutation::DefinePrim {
                    path: "/World/Box".to_owned(),
                    type_name: "Cube".to_owned(),
                },
                RuntimeMutation::SetAttribute {
                    prim_path: "/World/Box".to_owned(),
                    name: "size".to_owned(),
                    type_name: "double".to_owned(),
                    value: serde_json::json!(3.0),
                },
            ],
        };
        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::ApplyRuntimeMutationBatch {
                batch: batch.clone(),
            },
        );
        app.update();

        let accepted = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("runtime batch should publish an acceptance event");
        assert_eq!(accepted.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            accepted.event,
            ViewportEvent::RuntimeMutationBatchAccepted {
                source_id,
                sequence: 1,
                base_revision: 0,
                applied_operations: 2,
                ..
            } if source_id == "connector-a"
        ));

        let live = app
            .world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage");
        assert!(usd_bevy::authoring::prim_exists(&live.stage, "/World/Box"));
        assert!(matches!(
            live.stage.prim(openusd::sdf::path("/World/Box").unwrap()).attribute("size").get::<openusd::sdf::Value>().unwrap(),
            Some(openusd::sdf::Value::Double(v)) if (v - 3.0).abs() < f64::EPSILON
        ));

        app.world_mut()
            .get_non_send_mut::<usd_bevy::LiveStage>()
            .expect("live stage")
            .drain_change_batch();
        let stale_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::ApplyRuntimeMutationBatch {
                batch: batch.clone(),
            },
        );
        app.update();
        let rejected = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("stale runtime batch should be rejected");
        assert_eq!(rejected.request_id.as_deref(), Some(stale_request.as_str()));
        assert!(matches!(
            rejected.event,
            ViewportEvent::CommandRejected { reason, .. } if reason.contains("stale runtime base revision")
        ));

        let duplicate_request = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::ApplyRuntimeMutationBatch {
                batch: RuntimeMutationBatch {
                    base_revision: 1,
                    ..batch
                },
            },
        );
        app.update();
        let duplicate = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("duplicate runtime batch should be rejected");
        assert_eq!(
            duplicate.request_id.as_deref(),
            Some(duplicate_request.as_str())
        );
        assert!(matches!(
            duplicate.event,
            ViewportEvent::CommandRejected { reason, .. } if reason.contains("not newer than the last accepted sequence")
        ));
    }

    #[test]
    fn runtime_attribute_batch_reaches_bevy_and_semantic_worker() -> anyhow::Result<()> {
        let project_root = tempfile::tempdir()?;
        let mut app = runtime_semantic_test_app(project_root.path().to_path_buf());
        let stage = openusd::usd::Stage::builder()
            .in_memory("bridge_runtime_semantic_test.usda")
            .unwrap();
        stage
            .define_prim("/World")
            .unwrap()
            .set_type_name("Xform")
            .unwrap();
        stage
            .define_prim("/World/Box")
            .unwrap()
            .set_type_name("Cube")
            .unwrap();
        app.world_mut()
            .insert_non_send(usd_bevy::LiveStage::new(stage));

        let mut initial_snapshot_loaded = false;
        for _ in 0..200 {
            app.update();
            for response in app
                .world()
                .resource::<SemanticWorkingStore>()
                .drain_responses()
            {
                if matches!(response, SemanticResponse::SnapshotLoaded { .. }) {
                    initial_snapshot_loaded = true;
                }
            }
            if initial_snapshot_loaded {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            initial_snapshot_loaded,
            "initial semantic snapshot did not load"
        );

        let request_id = app.world_mut().resource_mut::<ViewportCommandInbox>().send(
            ViewportCommand::ApplyRuntimeMutationBatch {
                batch: RuntimeMutationBatch {
                    source_id: "connector-a".to_owned(),
                    sequence: 1,
                    base_revision: 0,
                    operations: vec![RuntimeMutation::SetAttribute {
                        prim_path: "/World/Box".to_owned(),
                        name: "Comments".to_owned(),
                        type_name: "string".to_owned(),
                        value: serde_json::json!("external"),
                    }],
                },
            },
        );
        app.update();
        let accepted = app
            .world_mut()
            .resource_mut::<ViewportEventOutbox>()
            .pop()
            .expect("runtime batch should publish an acceptance event");
        assert_eq!(accepted.request_id.as_deref(), Some(request_id.as_str()));
        assert!(matches!(
            accepted.event,
            ViewportEvent::RuntimeMutationBatchAccepted { .. }
        ));

        let mut semantic_delta_applied = false;
        for _ in 0..200 {
            app.update();
            for response in app
                .world()
                .resource::<SemanticWorkingStore>()
                .drain_responses()
            {
                if matches!(response, SemanticResponse::DeltaApplied { .. }) {
                    semantic_delta_applied = true;
                }
            }
            if semantic_delta_applied {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(semantic_delta_applied, "semantic delta did not apply");
        assert!(
            app.world()
                .resource::<usd_bevy::PrimEntities>()
                .entity("/World/Box")
                .is_some(),
            "Bevy prim index should retain the externally edited prim"
        );

        let store = app.world().resource::<SemanticWorkingStore>();
        assert!(store.submit_query(
            "runtime-query",
            SemanticQuery {
                filters: vec![SemanticFilter::PropertyTextEquals {
                    name: "Comments".to_owned(),
                    value: "external".to_owned(),
                }],
                limit: 10,
                ..default()
            },
        ));
        let mut matched = false;
        for _ in 0..200 {
            app.update();
            for response in app
                .world()
                .resource::<SemanticWorkingStore>()
                .drain_responses()
            {
                if let SemanticResponse::QueryResult { request_id, result } = response
                    && request_id == "runtime-query"
                {
                    matched = result.total == 1
                        && result.rows.iter().any(|row| row.prim_path == "/World/Box");
                }
            }
            if matched {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matched, "semantic query should see the external attribute");

        let session_id = app
            .world()
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage should remain available")
            .session_id();
        let recovery = RecoveryStore::new(project_root.path(), session_id)?;
        let recovered = recovery
            .restore()?
            .expect("runtime mutation should create a recovery checkpoint");
        assert!(usd_bevy::authoring::prim_exists(
            &recovered.stage,
            "/World/Box"
        ));
        Ok(())
    }
}
