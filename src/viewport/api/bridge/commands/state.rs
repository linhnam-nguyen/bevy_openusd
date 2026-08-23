use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use usd_bevy::LiveStage;

use super::super::state::{EditorHistories, RuntimeMutationCoordinator};
use crate::viewport::animation::UsdStageTime;
use crate::viewport::api::{SceneAnchorIndex, ViewportTreeCommandInbox};
use crate::viewport::app::cadence::RendererCadence;
use crate::viewport::camera::CameraMount;
use crate::viewport::physics::PhysicsActive;
use crate::viewport::rendering::sampling::{
    DlssCameraActivation, DlssCapability, FsrVulkanCapability, SamplingCoordinatorState,
};
use crate::viewport::scene::visualization::DisplayToggles;
use crate::viewport::scene::{SelectedPrim, SelectedTargets, SolariCapability};
use crate::viewport::session::{LoaderTuning, ReloadRequest, Spawned, StageInfo};

/// Groups the command system's independently-owned resources into one
/// [`SystemParam`], keeping the system within Bevy's top-level parameter
/// limit as command responsibilities grow.
#[derive(SystemParam)]
pub(in crate::viewport::api::bridge) struct ApplyViewportCommandState<'w, 's> {
    pub reload: ResMut<'w, ReloadRequest>,
    pub selected_prim: ResMut<'w, SelectedPrim>,
    pub selected_targets: ResMut<'w, SelectedTargets>,
    pub viewer_settings: ResMut<'w, super::super::ViewerSettingsState>,
    pub sampling: ResMut<'w, SamplingCoordinatorState>,
    pub dlss: Res<'w, DlssCapability>,
    pub fsr: Res<'w, FsrVulkanCapability>,
    pub dlss_camera: ResMut<'w, DlssCameraActivation>,
    pub scene_index: Res<'w, SceneAnchorIndex>,
    pub tree_commands: ResMut<'w, ViewportTreeCommandInbox>,
    pub camera_mount: ResMut<'w, CameraMount>,
    pub clock: ResMut<'w, UsdStageTime>,
    pub toggles: ResMut<'w, DisplayToggles>,
    pub solari: Option<Res<'w, SolariCapability>>,
    pub tuning: ResMut<'w, LoaderTuning>,
    pub physics: ResMut<'w, PhysicsActive>,
    pub histories: ResMut<'w, EditorHistories>,
    pub runtime_mutations: ResMut<'w, RuntimeMutationCoordinator>,
    pub configuration: ParamSet<'w, 's, (Res<'w, StageInfo>, Option<ResMut<'w, RendererCadence>>)>,
    pub stage: Option<NonSend<'w, LiveStage>>,
    pub spawned: Res<'w, Spawned>,
}
