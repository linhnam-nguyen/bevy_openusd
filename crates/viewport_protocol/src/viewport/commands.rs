use serde::{Deserialize, Serialize};

use crate::{PROTOCOL_VERSION, RequestId};

use super::bim::{BimPropertyMutation, ClassificationRecipe};
use super::editor::{EditorValue, RuntimeMutationBatch};
use super::hierarchy::{ClassificationColorEntry, HierarchyNodeId, HierarchySource};
use super::read_models::{
    CameraSource, CurveTuning, FocusMode, GroundGridOrigin, OverlayKind, RendererConfiguration,
    SamplingPreference, SceneAnchor, SelectionPresentationSettings, StandardView,
    ViewerEnvironmentSettings,
};

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
    RequestHierarchyChildren {
        source: HierarchySource,
        parent_id: Option<HierarchyNodeId>,
        page: u32,
        page_size: u32,
    },
    SearchHierarchy {
        source: HierarchySource,
        query: String,
        offset: u32,
        limit: u32,
    },
    /// Selects the provider for the single hierarchy panel. A BIM provider
    /// must carry a non-empty classification recipe; Prim must not carry one.
    SetHierarchySource {
        source: HierarchySource,
        classification_recipe: Option<ClassificationRecipe>,
    },
    /// Applies a temporary classification presentation plan to real scene
    /// anchors. An empty plan clears the override and never authors USD.
    SetClassificationColorPlan {
        generation: u64,
        entries: Vec<ClassificationColorEntry>,
    },
    ReloadSession,
    SelectTarget {
        target: Option<SceneAnchor>,
    },
    /// Replaces the complete selection set. `primary`, when present, must be
    /// one of the supplied targets.
    ReplaceSelection {
        targets: Vec<SceneAnchor>,
        primary: Option<SceneAnchor>,
    },
    /// Adds one target while preserving the existing set and optionally
    /// making the new target primary.
    AddSelectionTarget {
        target: SceneAnchor,
        make_primary: bool,
    },
    /// Adds a batch of targets in one authoritative selection transaction.
    /// When present, `primary` must be one of the added targets.
    AddSelectionTargets {
        targets: Vec<SceneAnchor>,
        primary: Option<SceneAnchor>,
    },
    /// Removes one target. If it was primary, the server chooses the first
    /// remaining canonical target as the new primary.
    RemoveSelectionTarget {
        target: SceneAnchor,
    },
    /// Removes a batch of targets in one authoritative selection transaction.
    RemoveSelectionTargets {
        targets: Vec<SceneAnchor>,
    },
    /// Clears the complete authoritative selection in one transaction.
    ClearSelection,
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
    SetStandardView {
        view: StandardView,
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
    SetRendererConfiguration {
        configuration: RendererConfiguration,
    },
    /// Sets supplementary environment values not owned by presentation state.
    /// Renderer configuration and grid origin use their existing authorities.
    SetEnvironmentSettings {
        settings: ViewerEnvironmentSettings,
    },
    /// Sets only the user's vendor-neutral sampling intent. The active
    /// provider is selected and reported by the server.
    SetSamplingPreference {
        preference: SamplingPreference,
    },
    SetSelectionPresentationSettings {
        settings: SelectionPresentationSettings,
    },
    /// Enables or disables the one aggregate Section Box for the current
    /// authoritative selection set. Transform details are deferred.
    SetSectionBox {
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
    EditBimProperty {
        mutation: BimPropertyMutation,
    },
    EditBimProperties {
        selection_revision: u64,
        mutations: Vec<BimPropertyMutation>,
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
    SaveStage,
    SaveStageAs {
        filename: String,
    },
    ExportStage,
    QueryPrim {
        prim_path: String,
    },
    ApplyRuntimeMutationBatch {
        batch: RuntimeMutationBatch,
    },
}

/// Versioned viewport command envelope.
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
