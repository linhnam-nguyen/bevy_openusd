use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;

use crate::stream::{MAX_FPS, MIN_FPS};
use crate::{PROTOCOL_VERSION, ProtocolValidationError, SessionId};

/// Stable, renderer-neutral identity for a logical USD target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if self.prim_path.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField {
                field: "selection.target.prim_path",
            });
        }
        if !self.prim_path.starts_with('/') || self.prim_path.contains('\0') {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.target.prim_path",
            });
        }
        if self
            .instance_context
            .as_deref()
            .is_some_and(|context| context.contains('\0'))
        {
            return Err(ProtocolValidationError::InvalidInput {
                field: "selection.target.instance_context",
            });
        }
        if let Some(session_id) = &self.session_id {
            session_id.validate()?;
        }
        Ok(())
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionReadModel {
    /// The complete authoritative selection set, in deterministic order.
    pub targets: Vec<SceneAnchor>,
    /// The active primary target, which must be a member of [`Self::targets`].
    pub primary: Option<SceneAnchor>,
}

impl SelectionReadModel {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        let mut seen = HashSet::with_capacity(self.targets.len());
        for target in &self.targets {
            target.validate()?;
            if !seen.insert(target) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.targets",
                });
            }
        }

        if let Some(primary) = &self.primary {
            primary.validate()?;
            if !self.targets.contains(primary) {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "selection.primary",
                });
            }
        }
        Ok(())
    }

    pub fn canonicalize(&mut self) -> Result<(), ProtocolValidationError> {
        self.validate()?;
        self.targets.sort();
        Ok(())
    }

    pub fn from_legacy_target(target: Option<SceneAnchor>) -> Self {
        let Some(target) = target else {
            return Self::default();
        };
        Self {
            targets: vec![target.clone()],
            primary: Some(target),
        }
    }
}

#[derive(Serialize)]
struct SelectionReadModelWire<'a> {
    targets: &'a [SceneAnchor],
    primary: &'a Option<SceneAnchor>,
}

#[derive(Deserialize)]
struct SelectionReadModelInput {
    #[serde(default)]
    targets: Vec<SceneAnchor>,
    #[serde(default)]
    primary: Option<SceneAnchor>,
    #[serde(default)]
    target: Option<SceneAnchor>,
}

impl Serialize for SelectionReadModel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut canonical = self.clone();
        canonical
            .canonicalize()
            .map_err(serde::ser::Error::custom)?;
        SelectionReadModelWire {
            targets: &canonical.targets,
            primary: &canonical.primary,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SelectionReadModel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = SelectionReadModelInput::deserialize(deserializer)?;
        let has_new_fields = !input.targets.is_empty() || input.primary.is_some();
        if input.target.is_some() && has_new_fields {
            return Err(D::Error::custom(
                "selection cannot contain both legacy target and multi-selection fields",
            ));
        }

        let mut selection = if input.target.is_some() {
            Self::from_legacy_target(input.target)
        } else {
            Self {
                targets: input.targets,
                primary: input.primary,
            }
        };
        selection.canonicalize().map_err(D::Error::custom)?;
        Ok(selection)
    }
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
            selection: SelectionReadModel::default(),
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
