use std::collections::HashSet;

use super::bim::validate_bim_mutation_batch;
use super::commands::ViewportCommand;
use super::constants::MAX_EDITOR_TEXT_BYTES;
use super::hierarchy::HierarchyNodeId;
use super::read_models::{SceneAnchor, SelectionReadModel};

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

        fn hierarchy_id(
            field: &'static str,
            value: &HierarchyNodeId,
        ) -> Result<(), ProtocolValidationError> {
            if value.0.trim().is_empty()
                || value.0.len() > super::constants::MAX_HIERARCHY_NODE_ID_BYTES
                || value.0.contains('\0')
            {
                return Err(ProtocolValidationError::InvalidInput { field });
            }
            Ok(())
        }

        fn hierarchy_query(value: &str) -> Result<(), ProtocolValidationError> {
            if value.trim().is_empty()
                || value.len() > super::constants::MAX_HIERARCHY_SEARCH_QUERY_BYTES
                || value.contains('\0')
            {
                return Err(ProtocolValidationError::InvalidInput {
                    field: "hierarchy.query",
                });
            }
            Ok(())
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
            Self::AddSelectionTargets { targets, primary } => {
                validate_selection_delta(targets, "selection.targets")?;
                if let Some(primary) = primary {
                    primary.validate()?;
                    if !targets.contains(primary) {
                        return Err(ProtocolValidationError::InvalidInput {
                            field: "selection.primary",
                        });
                    }
                }
            }
            Self::RemoveSelectionTargets { targets } => {
                validate_selection_delta(targets, "selection.targets")?;
            }
            Self::ClearSelection => {}
            Self::SetEnvironmentSettings { .. }
            | Self::SetSamplingPreference { .. }
            | Self::SetSectionBox { .. } => {}
            Self::SetSelectionPresentationSettings { settings } => settings.validate()?,
            Self::RequestHierarchyChildren {
                parent_id,
                page_size,
                ..
            } => {
                if let Some(parent_id) = parent_id {
                    hierarchy_id("hierarchy.parent_id", parent_id)?;
                }
                if *page_size == 0 || *page_size > super::constants::MAX_SCENE_PAGE_SIZE {
                    return Err(ProtocolValidationError::InvalidInput {
                        field: "hierarchy.page_size",
                    });
                }
            }
            Self::SearchHierarchy { query, limit, .. } => {
                hierarchy_query(query)?;
                if *limit == 0 || *limit > super::constants::MAX_SCENE_SEARCH_RESULTS {
                    return Err(ProtocolValidationError::InvalidInput {
                        field: "hierarchy.limit",
                    });
                }
            }
            Self::SearchBim { query } => query.validate()?,
            Self::SetHierarchySource {
                source,
                classification_recipe,
            } => match source {
                super::hierarchy::HierarchySource::Prim => {
                    if classification_recipe.is_some() {
                        return Err(ProtocolValidationError::InvalidInput {
                            field: "hierarchy.classification_recipe",
                        });
                    }
                }
                super::hierarchy::HierarchySource::BimClassification => {
                    let Some(recipe) = classification_recipe else {
                        return Err(ProtocolValidationError::InvalidInput {
                            field: "hierarchy.classification_recipe",
                        });
                    };
                    recipe.validate()?;
                }
            },
            Self::SetClassificationColorPlan { intent } => {
                if let Some(level) = intent.active_level.as_deref() {
                    text("classification_color.active_level", level)?;
                }
                if let super::hierarchy::ClassificationColorSource::Profile(profile) =
                    &intent.source
                {
                    text("classification_color.profile", profile)?;
                }
            }
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
            Self::EditBimProperty { mutation } => mutation.validate()?,
            Self::RequestBimPropertyProvenance { target, property } => {
                target.validate()?;
                text("bim.provenance.property", property)?;
            }
            Self::EditBimProperties { mutations, .. }
            | Self::ApplyBimReplacementBatch { mutations } => {
                validate_bim_mutation_batch(mutations)?
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
            | Self::RequestBimProperties
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
            | Self::SaveStage
            | Self::ExportStage => {}
        }
        Ok(())
    }
}

fn validate_selection_delta(
    targets: &[SceneAnchor],
    field: &'static str,
) -> Result<(), crate::ProtocolValidationError> {
    use crate::ProtocolValidationError;

    if targets.is_empty() {
        return Err(ProtocolValidationError::EmptyField { field });
    }
    if targets.len() > crate::MAX_SELECTION_TARGETS {
        return Err(ProtocolValidationError::InvalidInput { field });
    }
    let mut seen = HashSet::with_capacity(targets.len());
    for target in targets {
        target.validate()?;
        if !seen.insert(target) {
            return Err(ProtocolValidationError::InvalidInput { field });
        }
    }
    Ok(())
}
