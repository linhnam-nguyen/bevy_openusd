use bevy::prelude::*;

use crate::viewport::animation::UsdStageTime;
use crate::viewport::api::bridge::commands::apply_viewport_commands;
use crate::viewport::api::bridge::state::{EditorHistories, RuntimeMutationCoordinator};
use crate::viewport::api::{
    SceneAnchorIndex, ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommandInbox,
};
use crate::viewport::camera::CameraMount;
use crate::viewport::physics::PhysicsActive;
use crate::viewport::rendering::sampling::{
    DlssCameraActivation, DlssCapability, FsrVulkanCapability, SamplingCoordinatorState,
};
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::scene::{SelectedPrim, SelectedTargets};
use crate::viewport::session::{LoaderTuning, ReloadRequest, Spawned, StageInfo};

use super::super::ViewerSettingsState;

pub(super) fn command_test_app() -> App {
    let mut app = App::new();
    app.init_resource::<ViewportCommandInbox>()
        .init_resource::<ViewportEventOutbox>()
        .init_resource::<ViewportTreeCommandInbox>()
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<ReloadRequest>()
        .init_resource::<SelectedPrim>()
        .init_resource::<SelectedTargets>()
        .init_resource::<ViewerSettingsState>()
        .init_resource::<SamplingCoordinatorState>()
        .init_resource::<DlssCapability>()
        .init_resource::<FsrVulkanCapability>()
        .init_resource::<DlssCameraActivation>()
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
