use serde::{Deserialize, Serialize};

use crate::stream::{MAX_FPS, MIN_FPS};
use crate::{PROTOCOL_VERSION, ProtocolValidationError, SessionId};

/// Stable, renderer-neutral identity for a logical USD target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SceneAnchor {
    pub session_id: Option<SessionId>,
    pub prim_path: String,
    pub instance_context: Option<String>,
}

impl SceneAnchor {
    pub fn active_session(prim_path: impl Into<String>) -> Self {
        Self {
            session_id: None,
            prim_path: prim_path.into(),
            instance_context: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraSource {
    Arcball,
    Authored { prim_path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayKind {
    GroundGrid,
    WorldAxes,
    PrimMarkers,
    Skeleton,
    Physics,
    Colliders,
    Wireframe,
}

/// Selects the reference plane used by the viewport ground grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroundGridOrigin {
    /// Follow the lowest loaded renderable geometry bound.
    #[default]
    LoadedScene,
    /// Stay on the Bevy world-origin plane (`y = 0`).
    WorldOrigin,
}

/// Selects the renderer's primary visual representation.
///
/// This deliberately contains only the modes that the renderer contract can
/// represent. Edge display is a separate option and must not be encoded as a
/// wireframe mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    /// Use one uniform material color for all rendered surfaces.
    UniformColor,
    #[default]
    Shaded,
    Wireframe,
    /// Use Bevy Solari when the negotiated renderer capabilities allow it.
    RayTraced,
}

/// Compact RGB color value that is safe to carry across the wire.
///
/// Each channel is deliberately represented as `u8`; serde therefore rejects
/// negative, fractional, and out-of-range channel values before they reach a
/// renderer or presentation adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorRgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl ColorRgb8 {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Renderer-neutral environment settings requested by the viewer.
///
/// These values describe user intent only. Renderer capability negotiation and
/// application remain server-owned concerns in later B milestones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerEnvironmentSettings {
    pub render_mode: RenderMode,
    pub shadows_enabled: bool,
    pub grid_visible: bool,
    pub grid_color: ColorRgb8,
    pub grid_origin: GroundGridOrigin,
    pub background_color: ColorRgb8,
    pub default_surface_color: ColorRgb8,
}

/// Renderer-neutral selection presentation preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionPresentationSettings {
    pub boundary_enabled: bool,
    pub boundary_color: ColorRgb8,
    pub color_change_enabled: bool,
    pub selection_color: ColorRgb8,
    pub hover_color_change_enabled: bool,
    pub hover_color: ColorRgb8,
}

/// Vendor-neutral sampling intent. The active provider is authoritative
/// read-only state and is represented separately by [`SamplingProvider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SamplingPreference {
    pub enabled: bool,
}

/// Renderer-selected sampling provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingProvider {
    #[default]
    None,
    Dlss,
    Fsr,
}

/// Transport-neutral renderer options shared by commands and future
/// authoritative presentation events.
///
/// `preferred_fps = None` intentionally means uncapped renderer cadence. The
/// server remains responsible for applying the accepted value to Bevy and may
/// reject a command before application; it must not silently substitute a
/// different mode or frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RendererConfiguration {
    pub grid: bool,
    pub shadows: bool,
    pub edges: bool,
    pub render_mode: RenderMode,
    pub preferred_fps: Option<u32>,
}

impl Default for RendererConfiguration {
    fn default() -> Self {
        Self {
            grid: true,
            shadows: true,
            edges: false,
            render_mode: RenderMode::Shaded,
            preferred_fps: Some(60),
        }
    }
}

impl RendererConfiguration {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if matches!(
            self.render_mode,
            RenderMode::UniformColor | RenderMode::RayTraced
        ) {
            return Err(ProtocolValidationError::InvalidInput {
                field: "renderer.render_mode",
            });
        }
        if let Some(fps) = self.preferred_fps
            && !(MIN_FPS..=MAX_FPS).contains(&fps)
        {
            return Err(ProtocolValidationError::InvalidFrameRate { value: fps });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    FrameTarget,
    FlyToTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimNodeReadModel {
    pub anchor: SceneAnchor,
    pub parent: Option<SceneAnchor>,
    pub label: String,
    pub visible: bool,
    pub has_children: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneReadModel {
    pub prims: Vec<PrimNodeReadModel>,
    #[serde(default)]
    pub total_prims: u32,
    #[serde(default)]
    pub total_roots: u32,
    #[serde(default)]
    pub root_page_size: u32,
}

/// A bounded page of direct children for one scene-tree parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneChildrenPage {
    pub parent: Option<SceneAnchor>,
    pub page: u32,
    pub page_size: u32,
    pub total: u32,
    pub nodes: Vec<PrimNodeReadModel>,
}

/// One page that the frontend must load to reveal a search match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenePageReference {
    pub parent: Option<SceneAnchor>,
    pub page: u32,
}

/// A compact server-side search match with enough information to reveal it in
/// a partially-loaded tree. The stable anchor is never reconstructed from the
/// display label, which may be truncated by the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneSearchMatch {
    pub anchor: SceneAnchor,
    pub parent: Option<SceneAnchor>,
    pub label: String,
    pub visible: bool,
    pub has_children: bool,
    pub reveal_pages: Vec<ScenePageReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurveTuning {
    pub default_radius: f32,
    pub ring_segments: u32,
    pub point_scale: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageReadModel {
    pub display_name: String,
    pub loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageLoadState {
    Idle,
    Loading,
    Ready,
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionReadModel {
    pub target: Option<SceneAnchor>,
}

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
            selection: SelectionReadModel { target: None },
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
