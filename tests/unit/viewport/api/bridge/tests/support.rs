use bevy::prelude::*;

use crate::project::recovery::RecoverySettings;
use crate::project::recovery_worker::{RecoveryRuntime, drain_recovery_results};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::api::bridge::commands::apply_viewport_commands;
use crate::viewport::api::bridge::plugin::checkpoint_recovery;
use crate::viewport::api::bridge::state::{EditorHistories, RuntimeMutationCoordinator};
use crate::viewport::api::{
    CurrentHierarchyProjection, SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox,
    ViewportTreeCommandInbox,
};
use crate::viewport::camera::{CameraMount, CameraOrientationState, FlyTo};
use crate::viewport::physics::PhysicsActive;
use crate::viewport::rendering::sampling::{
    DlssCameraActivation, DlssCapability, SamplingCoordinatorState,
};
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::scene::{SelectedPrim, SelectedTargets};
use crate::viewport::semantic::synchronize_live_stage;
use crate::viewport::semantic::{SemanticDiffState, SemanticSyncState, SemanticWorkingStore};
use crate::viewport::session::{LoaderTuning, ReloadRequest, Spawned, StageInfo};

use super::super::ViewerSettingsState;

pub(super) fn command_test_app() -> App {
    let mut app = App::new();
    app.init_resource::<ViewportCommandInbox>()
        .init_resource::<ViewportEventOutbox>()
        .init_resource::<ViewportTreeCommandInbox>()
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<ReloadRequest>()
        .init_resource::<SelectedPrim>()
        .init_resource::<SelectedTargets>()
        .init_resource::<ViewerSettingsState>()
        .init_resource::<SamplingCoordinatorState>()
        .init_resource::<DlssCapability>()
        .init_resource::<DlssCameraActivation>()
        .init_resource::<CameraMount>()
        .init_resource::<CameraOrientationState>()
        .init_resource::<FlyTo>()
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

pub(super) fn runtime_semantic_test_app(project_root: std::path::PathBuf) -> App {
    let mut app = App::new();
    app.add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin)
        .init_resource::<ViewportCommandInbox>()
        .init_resource::<ViewportEventOutbox>()
        .init_resource::<ViewportTreeCommandInbox>()
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<ReloadRequest>()
        .init_resource::<SelectedPrim>()
        .init_resource::<SelectedTargets>()
        .init_resource::<ViewerSettingsState>()
        .init_resource::<SamplingCoordinatorState>()
        .init_resource::<DlssCapability>()
        .init_resource::<DlssCameraActivation>()
        .init_resource::<CameraMount>()
        .init_resource::<CameraOrientationState>()
        .init_resource::<FlyTo>()
        .init_resource::<UsdStageTime>()
        .init_resource::<DisplayToggles>()
        .init_resource::<LoaderTuning>()
        .init_resource::<PhysicsActive>()
        .init_resource::<EditorHistories>()
        .init_resource::<RuntimeMutationCoordinator>()
        .init_resource::<Spawned>()
        .init_resource::<SemanticWorkingStore>()
        .insert_resource(SemanticSyncState::with_config(
            usd_semantic::SemanticConfig::for_nvidia_revit_connector(),
        ))
        .init_resource::<crate::viewport::bim::BimClassificationFieldCatalogueState>()
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
