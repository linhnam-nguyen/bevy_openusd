use serde::{Deserialize, Serialize};

use super::commands::ViewportCommand;
use super::constants::{MAX_RUNTIME_MUTATIONS, MAX_RUNTIME_SOURCE_ID_BYTES};

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
    EditBimProperty,
    EditBimProperties,
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
    ApplyRuntimeMutationBatch,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorStateReadModel {
    pub can_undo: bool,
    pub can_redo: bool,
}

/// A connector-originated group of model mutations applied by the active
/// runtime writer. `base_revision` is the last drained live-stage revision;
/// it is deliberately a plain integer so this protocol crate stays
/// independent of `usd_bevy`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeMutationBatch {
    pub source_id: String,
    pub sequence: u64,
    pub base_revision: u64,
    pub operations: Vec<RuntimeMutation>,
}

/// Renderer-neutral model mutations accepted from an external runtime
/// connector. These mirror the existing authoring operations but are grouped
/// under one source sequence and base revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum RuntimeMutation {
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
    SetVariantSelection {
        prim_path: String,
        set_name: String,
        option: String,
    },
}

impl RuntimeMutationBatch {
    pub fn validate(&self) -> Result<(), crate::ProtocolValidationError> {
        if self.source_id.trim().is_empty() {
            return Err(crate::ProtocolValidationError::EmptyField {
                field: "runtime.source_id",
            });
        }
        if self.source_id.len() > MAX_RUNTIME_SOURCE_ID_BYTES {
            return Err(crate::ProtocolValidationError::InvalidInput {
                field: "runtime.source_id.length",
            });
        }
        if self.sequence == 0 {
            return Err(crate::ProtocolValidationError::InvalidSequence);
        }
        if self.operations.is_empty() || self.operations.len() > MAX_RUNTIME_MUTATIONS {
            return Err(crate::ProtocolValidationError::InvalidInput {
                field: "runtime.operations.length",
            });
        }
        for operation in &self.operations {
            operation.as_viewport_command().validate()?;
        }
        Ok(())
    }
}

impl RuntimeMutation {
    fn as_viewport_command(&self) -> ViewportCommand {
        match self {
            Self::DefinePrim { path, type_name } => ViewportCommand::DefinePrim {
                path: path.clone(),
                type_name: type_name.clone(),
            },
            Self::RemovePrim { path } => ViewportCommand::RemovePrim { path: path.clone() },
            Self::RenamePrim { path, new_name } => ViewportCommand::RenamePrim {
                path: path.clone(),
                new_name: new_name.clone(),
            },
            Self::ReparentPrim { path, new_parent } => ViewportCommand::ReparentPrim {
                path: path.clone(),
                new_parent: new_parent.clone(),
            },
            Self::MovePrim { old_path, new_path } => ViewportCommand::MovePrim {
                old_path: old_path.clone(),
                new_path: new_path.clone(),
            },
            Self::SetAttribute {
                prim_path,
                name,
                type_name,
                value,
            } => ViewportCommand::SetAttribute {
                prim_path: prim_path.clone(),
                name: name.clone(),
                type_name: type_name.clone(),
                value: value.clone(),
            },
            Self::ClearAttribute { prim_path, name } => ViewportCommand::ClearAttribute {
                prim_path: prim_path.clone(),
                name: name.clone(),
            },
            Self::SetTransform {
                prim_path,
                translation,
                rotation,
                scale,
            } => ViewportCommand::SetTransform {
                prim_path: prim_path.clone(),
                translation: *translation,
                rotation: *rotation,
                scale: *scale,
            },
            Self::SetVariantSelection {
                prim_path,
                set_name,
                option,
            } => ViewportCommand::SetVariantSelection {
                prim_path: prim_path.clone(),
                set_name: set_name.clone(),
                option: option.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorPrimReadModel {
    pub prim_path: String,
    pub exists: bool,
}
