//! Existing semantic viewport commands, events, and read models.
//!
//! These definitions are intentionally moved without changing their serde
//! representation. The legacy stdio adapter and the current UI therefore keep
//! their protocol-version-1 wire shape while the new session contract grows
//! around them.

use serde::{Deserialize, Serialize};

use crate::{PROTOCOL_VERSION, RequestId, SessionId};

pub const DEFAULT_SCENE_PAGE_SIZE: u32 = 64;
pub const MAX_SCENE_PAGE_SIZE: u32 = 256;
pub const DEFAULT_SCENE_SEARCH_PAGE_SIZE: u32 = 30;
pub const MAX_SCENE_SEARCH_RESULTS: u32 = 256;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ViewportCommand {
    RequestSnapshot,
    RequestSceneChildren {
        parent: Option<SceneAnchor>,
        page: u32,
        page_size: u32,
    },
    SearchScene {
        query: String,
        offset: u32,
        limit: u32,
    },
    ReloadSession,
    SelectTarget {
        target: Option<SceneAnchor>,
    },
    FocusTarget {
        target: SceneAnchor,
        mode: FocusMode,
    },
    SetSubtreeVisibility {
        target: SceneAnchor,
        visible: bool,
    },
    SetVariantSelection {
        prim_path: String,
        set_name: String,
        option: String,
    },
    ResetVariantSelection {
        prim_path: String,
        set_name: String,
    },
    SetCameraSource {
        source: CameraSource,
    },
    SetPlayback {
        playing: bool,
    },
    Seek {
        seconds: f64,
    },
    SetOverlay {
        overlay: OverlayKind,
        enabled: bool,
    },
    SetPrimMarkerBias {
        bias: f32,
    },
    SetLightIntensity {
        scale: f32,
    },
    SetCurveTuning {
        tuning: CurveTuning,
    },
    SetPhysicsRunning {
        running: bool,
    },
}

/// Legacy command envelope retained byte/schema compatible with version 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportCommandEnvelope {
    pub protocol_version: u16,
    pub request_id: RequestId,
    pub command: ViewportCommand,
}

impl ViewportCommandEnvelope {
    pub fn new(request_id: impl Into<RequestId>, command: ViewportCommand) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            command,
        }
    }

    pub fn validate(&self) -> Result<(), crate::ProtocolValidationError> {
        crate::envelope::validate_protocol_version(self.protocol_version)?;
        if self.request_id.trim().is_empty() {
            return Err(crate::ProtocolValidationError::EmptyField { field: "request_id" });
        }
        Ok(())
    }
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
    pub ground_grid: bool,
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
                ground_grid: false,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ViewportEvent {
    Ready { protocol_version: u16 },
    Snapshot { state: ViewportReadModel },
    SceneChildren { page: SceneChildrenPage },
    SearchResults {
        query: String,
        offset: u32,
        total: u32,
        matches: Vec<SceneSearchMatch>,
        has_more: bool,
    },
    StageLoadStateChanged { state: StageLoadState },
    SelectionChanged { selection: SelectionReadModel },
    CameraTransitionStarted { target: SceneAnchor, mode: FocusMode },
    PrimVisibilityChanged { target: SceneAnchor, visible: bool },
    CameraSourceChanged { source: CameraSource },
    TimelineChanged { timeline: TimelineReadModel },
    PresentationChanged { presentation: PresentationReadModel },
    PhysicsChanged { running: bool },
    CommandRejected { request_id: RequestId, reason: String },
}

/// Legacy event envelope retained byte/schema compatible with version 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportEventEnvelope {
    pub protocol_version: u16,
    pub request_id: Option<RequestId>,
    pub event: ViewportEvent,
}

impl ViewportEventEnvelope {
    pub fn new(request_id: Option<RequestId>, event: ViewportEvent) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            event,
        }
    }
}

/// Legacy JSON Lines direction marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ViewportWireMessage {
    Command(ViewportCommandEnvelope),
    Event(ViewportEventEnvelope),
}
