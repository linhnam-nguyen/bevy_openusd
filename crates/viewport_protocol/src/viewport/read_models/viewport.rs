use serde::{Deserialize, Serialize};

use super::identity::{CameraSource, GroundGridOrigin};
use super::scene::{CurveTuning, SceneReadModel, StageReadModel};
use super::selection::SelectionReadModel;
use super::settings::{RendererConfiguration, ViewerSettingsReadModel};
use crate::PROTOCOL_VERSION;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineReadModel {
    pub seconds: f64,
    pub playing: bool,
    pub start_time_code: f64,
    pub end_time_code: f64,
    pub time_codes_per_second: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationReadModel {
    #[serde(default)]
    pub renderer: RendererConfiguration,
    pub ground_grid: bool,
    #[serde(default)]
    pub ground_grid_origin: GroundGridOrigin,
    pub world_axes: bool,
    pub prim_markers: bool,
    pub prim_marker_bias: f32,
    pub skeleton: bool,
    pub physics: bool,
    pub colliders: bool,
    pub wireframe: bool,
    pub light_intensity_scale: f32,
    pub curve_tuning: CurveTuning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportReadModel {
    pub protocol_version: u16,
    pub stage: StageReadModel,
    pub scene: SceneReadModel,
    pub selection: SelectionReadModel,
    #[serde(default)]
    pub viewer_settings: ViewerSettingsReadModel,
    pub camera_source: CameraSource,
    pub timeline: TimelineReadModel,
    pub presentation: PresentationReadModel,
    pub physics_running: bool,
}

impl ViewportReadModel {
    /// Creates the honest pre-stage snapshot used while the render server is
    /// connected but the USD stage has not completed loading yet.
    pub fn unloaded(display_name: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            stage: StageReadModel {
                display_name: display_name.into(),
                loaded: false,
            },
            scene: SceneReadModel::default(),
            selection: SelectionReadModel::default(),
            viewer_settings: ViewerSettingsReadModel::default(),
            camera_source: CameraSource::Arcball,
            timeline: TimelineReadModel {
                seconds: 0.0,
                playing: false,
                start_time_code: 0.0,
                end_time_code: 0.0,
                time_codes_per_second: 24.0,
            },
            presentation: PresentationReadModel {
                renderer: RendererConfiguration::default(),
                ground_grid: false,
                ground_grid_origin: GroundGridOrigin::LoadedScene,
                world_axes: false,
                prim_markers: false,
                prim_marker_bias: 1.0,
                skeleton: false,
                physics: false,
                colliders: false,
                wireframe: false,
                light_intensity_scale: 1.0,
                curve_tuning: CurveTuning {
                    default_radius: 0.02,
                    ring_segments: 6,
                    point_scale: 1.0,
                },
            },
            physics_running: false,
        }
    }
}
