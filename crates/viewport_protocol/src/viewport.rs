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
pub const MAX_EDITOR_TEXT_BYTES: usize = 8 * 1024 * 1024;

/// JSON value used by the editor wire contract for USD attributes.
///
/// The accompanying `type_name` on [`ViewportCommand::SetAttribute`] selects
/// the USD type (`double`, `float3`, `token[]`, and so on). Keeping the value
/// JSON-native means the protocol crate remains independent of OpenUSD while
/// still allowing a frontend to author scalar, vector, matrix, and array
/// values.
pub type EditorValue = serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorOperation {
    DefinePrim,
    RemovePrim,
    RenamePrim,
    ReparentPrim,
    MovePrim,
    SetAttribute,
    ClearAttribute,
    SetVariantSelection,
    SetTransform,
    LoadPayload,
    UnloadPayload,
    Undo,
    Redo,
    SaveStageAs,
    ExportStage,
    QueryPrim,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorStateReadModel {
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorPrimReadModel {
    pub prim_path: String,
    pub exists: bool,
}

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
    SetGroundGridOrigin {
        origin: GroundGridOrigin,
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
    DefinePrim {
        path: String,
        type_name: String,
    },
    RemovePrim {
        path: String,
    },
    RenamePrim {
        path: String,
        new_name: String,
    },
    ReparentPrim {
        path: String,
        new_parent: String,
    },
    MovePrim {
        old_path: String,
        new_path: String,
    },
    SetAttribute {
        prim_path: String,
        name: String,
        type_name: String,
        value: EditorValue,
    },
    ClearAttribute {
        prim_path: String,
        name: String,
    },
    SetTransform {
        prim_path: String,
        translation: [f32; 3],
        rotation: [f32; 4],
        scale: [f32; 3],
    },
    LoadPayload {
        prim_path: String,
    },
    UnloadPayload {
        prim_path: String,
    },
    UndoEditor,
    RedoEditor,
    SaveStageAs {
        filename: String,
    },
    ExportStage,
    QueryPrim {
        prim_path: String,
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
            return Err(crate::ProtocolValidationError::EmptyField {
                field: "request_id",
            });
        }
        self.command.validate()?;
        Ok(())
    }
}

impl ViewportCommand {
    pub fn validate(&self) -> Result<(), crate::ProtocolValidationError> {
        use crate::ProtocolValidationError;

        fn path(field: &'static str, value: &str) -> Result<(), ProtocolValidationError> {
            if value.trim().is_empty() {
                return Err(ProtocolValidationError::EmptyField { field });
            }
            if !value.starts_with('/') || value.contains('\0') {
                return Err(ProtocolValidationError::InvalidInput { field });
            }
            Ok(())
        }

        fn text(field: &'static str, value: &str) -> Result<(), ProtocolValidationError> {
            if value.trim().is_empty() {
                return Err(ProtocolValidationError::EmptyField { field });
            }
            if value.len() > MAX_EDITOR_TEXT_BYTES {
                return Err(ProtocolValidationError::InvalidInput { field });
            }
            Ok(())
        }

        fn finite(field: &'static str, values: &[f32]) -> Result<(), ProtocolValidationError> {
            if values.iter().all(|value| value.is_finite()) {
                Ok(())
            } else {
                Err(ProtocolValidationError::InvalidInput { field })
            }
        }

        match self {
            Self::DefinePrim {
                path: value,
                type_name,
            } => {
                path("editor.path", value)?;
                text("editor.type_name", type_name)?;
            }
            Self::RemovePrim { path: value }
            | Self::LoadPayload { prim_path: value }
            | Self::UnloadPayload { prim_path: value }
            | Self::QueryPrim { prim_path: value } => path("editor.prim_path", value)?,
            Self::RenamePrim {
                path: value,
                new_name,
            } => {
                path("editor.path", value)?;
                text("editor.new_name", new_name)?;
                if new_name.contains('/') {
                    return Err(ProtocolValidationError::InvalidInput {
                        field: "editor.new_name",
                    });
                }
            }
            Self::ReparentPrim {
                path: value,
                new_parent,
            } => {
                path("editor.path", value)?;
                path("editor.new_parent", new_parent)?;
            }
            Self::MovePrim { old_path, new_path } => {
                path("editor.old_path", old_path)?;
                path("editor.new_path", new_path)?;
            }
            Self::SetAttribute {
                prim_path,
                name,
                type_name,
                value,
            } => {
                path("editor.prim_path", prim_path)?;
                text("editor.name", name)?;
                text("editor.type_name", type_name)?;
                if serde_json::to_vec(value)
                    .map(|bytes| bytes.len() > MAX_EDITOR_TEXT_BYTES)
                    .unwrap_or(true)
                {
                    return Err(ProtocolValidationError::InvalidInput {
                        field: "editor.value",
                    });
                }
            }
            Self::ClearAttribute { prim_path, name } => {
                path("editor.prim_path", prim_path)?;
                text("editor.name", name)?;
            }
            Self::SetTransform {
                prim_path,
                translation,
                rotation,
                scale,
            } => {
                path("editor.prim_path", prim_path)?;
                finite("editor.translation", translation)?;
                finite("editor.rotation", rotation)?;
                finite("editor.scale", scale)?;
            }
            Self::SaveStageAs { filename } => text("editor.filename", filename)?,
            Self::SetVariantSelection { .. }
            | Self::ResetVariantSelection { .. }
            | Self::RequestSnapshot
            | Self::RequestSceneChildren { .. }
            | Self::SearchScene { .. }
            | Self::ReloadSession
            | Self::SelectTarget { .. }
            | Self::FocusTarget { .. }
            | Self::SetSubtreeVisibility { .. }
            | Self::SetCameraSource { .. }
            | Self::SetPlayback { .. }
            | Self::Seek { .. }
            | Self::SetOverlay { .. }
            | Self::SetGroundGridOrigin { .. }
            | Self::SetPrimMarkerBias { .. }
            | Self::SetLightIntensity { .. }
            | Self::SetCurveTuning { .. }
            | Self::SetPhysicsRunning { .. }
            | Self::UndoEditor
            | Self::RedoEditor
            | Self::ExportStage => {}
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ViewportEvent {
    Ready {
        protocol_version: u16,
    },
    Snapshot {
        state: ViewportReadModel,
    },
    SceneChildren {
        page: SceneChildrenPage,
    },
    SearchResults {
        query: String,
        offset: u32,
        total: u32,
        matches: Vec<SceneSearchMatch>,
        has_more: bool,
    },
    StageLoadStateChanged {
        state: StageLoadState,
    },
    SelectionChanged {
        selection: SelectionReadModel,
    },
    CameraTransitionStarted {
        target: SceneAnchor,
        mode: FocusMode,
    },
    PrimVisibilityChanged {
        target: SceneAnchor,
        visible: bool,
    },
    CameraSourceChanged {
        source: CameraSource,
    },
    TimelineChanged {
        timeline: TimelineReadModel,
    },
    PresentationChanged {
        presentation: PresentationReadModel,
    },
    PhysicsChanged {
        running: bool,
    },
    CommandRejected {
        request_id: RequestId,
        reason: String,
    },
    EditorCommandCompleted {
        operation: EditorOperation,
        changed_paths: Vec<String>,
        state: EditorStateReadModel,
    },
    EditorPrimState {
        prim: EditorPrimReadModel,
    },
    EditorStageExportChunk {
        export_id: String,
        chunk_index: u32,
        chunk_count: u32,
        content: String,
    },
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
