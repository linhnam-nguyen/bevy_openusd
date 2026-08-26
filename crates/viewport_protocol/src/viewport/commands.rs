use serde::{Deserialize, Serialize};

use crate::{PROTOCOL_VERSION, RequestId};

use super::constants::MAX_EDITOR_TEXT_BYTES;
use super::editor::{EditorValue, RuntimeMutationBatch};
use super::read_models::{
    CameraSource, CurveTuning, FocusMode, GroundGridOrigin, OverlayKind, RendererConfiguration,
    SamplingPreference, SceneAnchor, SelectionPresentationSettings, SelectionReadModel,
    StandardView, ViewerEnvironmentSettings,
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
    /// Removes one target. If it was primary, the server chooses the first
    /// remaining canonical target as the new primary.
    RemoveSelectionTarget {
        target: SceneAnchor,
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
            Self::SelectTarget { target } => {
                if let Some(target) = target {
                    target.validate()?;
                }
            }
            Self::ReplaceSelection { targets, primary } => {
                SelectionReadModel::validate_parts(targets, primary.as_ref())?;
            }
            Self::AddSelectionTarget { target, .. } | Self::RemoveSelectionTarget { target } => {
                target.validate()?
            }
            Self::SetEnvironmentSettings { .. }
            | Self::SetSamplingPreference { .. }
            | Self::SetSectionBox { .. } => {}
            Self::SetSelectionPresentationSettings { settings } => settings.validate()?,
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
            Self::ApplyRuntimeMutationBatch { batch } => batch.validate()?,
            Self::SetRendererConfiguration { configuration } => configuration.validate()?,
            Self::SaveStageAs { filename } => text("editor.filename", filename)?,
            Self::SetVariantSelection { .. }
            | Self::ResetVariantSelection { .. }
            | Self::RequestSnapshot
            | Self::RequestSceneChildren { .. }
            | Self::SearchScene { .. }
            | Self::ReloadSession
            | Self::FocusTarget { .. }
            | Self::SetSubtreeVisibility { .. }
            | Self::SetCameraSource { .. }
            | Self::SetStandardView { .. }
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
