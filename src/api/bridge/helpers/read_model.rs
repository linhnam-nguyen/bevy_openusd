use crate::animation::UsdStageTime;
use crate::viewport::navigation::camera::CameraMount;
use crate::viewport::rendering::scene::{CurrentHierarchyProjection, SceneAnchorIndex};
use crate::viewport::rendering::scene::visualization::DisplayToggles;
use crate::viewport::stage::{LoaderTuning, StageInfo};
use viewport_protocol::{
    CameraOrientationReadModel, CameraSource, CurveTuning as ProtocolCurveTuning, HierarchySource,
    PROTOCOL_VERSION, PresentationReadModel, RenderMode, SelectionReadModel, StageReadModel,
    TimelineReadModel, ViewerSettingsReadModel, ViewportReadModel,
};

#[allow(clippy::too_many_arguments)]
pub(in crate::api::bridge) fn build_read_model(
    stage_info: &StageInfo,
    stage_loaded: bool,
    selection: &SelectionReadModel,
    selection_revision: u64,
    settings: &ViewerSettingsReadModel,
    scene_index: &SceneAnchorIndex,
    current_projection: Option<&CurrentHierarchyProjection>,
    camera_mount: &CameraMount,
    camera_orientation: &CameraOrientationReadModel,
    clock: &UsdStageTime,
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
    physics_running: bool,
) -> ViewportReadModel {
    let scene = current_projection
        .filter(|projection| {
            projection.source() == HierarchySource::Prim
                && (projection.revision() > 0 || !scene_index.has_published_nodes())
        })
        .and_then(CurrentHierarchyProjection::prim_roots_read_model)
        .unwrap_or_else(|| scene_index.roots_read_model());
    ViewportReadModel {
        protocol_version: PROTOCOL_VERSION,
        stage: StageReadModel {
            activation_generation: stage_info.activation_generation,
            display_name: stage_info.path.clone(),
            loaded: stage_loaded,
        },
        scene,
        selection: selection.clone(),
        selection_revision,
        viewer_settings: settings.clone(),
        camera_source: match camera_mount {
            CameraMount::Arcball => CameraSource::Arcball,
            CameraMount::Mounted { prim_path } => CameraSource::Authored {
                prim_path: prim_path.clone(),
            },
        },
        camera_orientation: *camera_orientation,
        timeline: timeline_read_model(clock),
        presentation: presentation_read_model(toggles, tuning),
        physics_running,
    }
}

pub(in crate::api::bridge) fn timeline_read_model(
    clock: &UsdStageTime,
) -> TimelineReadModel {
    TimelineReadModel {
        seconds: clock.seconds,
        playing: clock.playing,
        start_time_code: clock.start_time_code,
        end_time_code: clock.end_time_code,
        time_codes_per_second: clock.time_codes_per_second,
    }
}

pub(in crate::api::bridge) fn presentation_read_model(
    toggles: &DisplayToggles,
    tuning: &LoaderTuning,
) -> PresentationReadModel {
    PresentationReadModel {
        renderer: toggles.renderer,
        ground_grid: toggles.renderer.grid,
        ground_grid_origin: toggles.ground_grid_origin,
        world_axes: toggles.show_world_axes,
        prim_markers: toggles.show_prim_markers,
        prim_marker_bias: toggles.prim_marker_bias,
        skeleton: toggles.show_skeleton,
        physics: toggles.show_physics,
        colliders: toggles.show_colliders,
        wireframe: toggles.renderer.render_mode == RenderMode::Wireframe,
        light_intensity_scale: toggles.light_intensity_scale,
        curve_tuning: ProtocolCurveTuning {
            default_radius: tuning.curves.default_radius,
            ring_segments: tuning.curves.ring_segments,
            point_scale: tuning.curves.point_scale,
        },
    }
}
