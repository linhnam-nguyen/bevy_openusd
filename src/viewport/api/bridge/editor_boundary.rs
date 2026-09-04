//! Authoring policy for composed Project Model boundaries.

use openusd::{sdf::Value, usd::Stage};
use usd_project::ModelId;
use viewport_protocol::{RuntimeMutation, ViewportCommand};

const MODEL_ID_METADATA: &str = "usdhub:modelId";
const TARGET_KIND_METADATA: &str = "usdhub:targetKind";
const TARGET_ID_METADATA: &str = "usdhub:targetId";

pub(super) fn validate_command(stage: &Stage, command: &ViewportCommand) -> Result<(), String> {
    match command {
        ViewportCommand::DefinePrim { path, .. }
        | ViewportCommand::RemovePrim { path }
        | ViewportCommand::RenamePrim { path, .. }
        | ViewportCommand::SetVariantSelection {
            prim_path: path, ..
        } => reject_if_model_source(stage, path, false),
        ViewportCommand::ReparentPrim { path, new_parent } => {
            reject_if_model_source(stage, path, false)?;
            reject_if_model_source(stage, new_parent, false)
        }
        ViewportCommand::MovePrim { old_path, new_path } => {
            reject_if_model_source(stage, old_path, false)?;
            reject_if_model_source(stage, new_path, false)
        }
        ViewportCommand::SetAttribute { prim_path, .. }
        | ViewportCommand::ClearAttribute { prim_path, .. } => {
            reject_if_model_source(stage, prim_path, true)
        }
        ViewportCommand::SetTransform { prim_path, .. } => {
            reject_if_model_source(stage, prim_path, true)
        }
        ViewportCommand::ApplyRuntimeMutationBatch { batch } => {
            for mutation in &batch.operations {
                validate_runtime_mutation(stage, mutation)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_runtime_mutation(stage: &Stage, mutation: &RuntimeMutation) -> Result<(), String> {
    match mutation {
        RuntimeMutation::DefinePrim { path, .. }
        | RuntimeMutation::RemovePrim { path }
        | RuntimeMutation::RenamePrim { path, .. } => reject_if_model_source(stage, path, false),
        RuntimeMutation::ReparentPrim { path, new_parent } => {
            reject_if_model_source(stage, path, false)?;
            reject_if_model_source(stage, new_parent, false)
        }
        RuntimeMutation::MovePrim { old_path, new_path } => {
            reject_if_model_source(stage, old_path, false)?;
            reject_if_model_source(stage, new_path, false)
        }
        RuntimeMutation::SetAttribute { prim_path, .. }
        | RuntimeMutation::ClearAttribute { prim_path, .. }
        | RuntimeMutation::SetTransform { prim_path, .. }
        | RuntimeMutation::SetVariantSelection { prim_path, .. } => {
            reject_if_model_source(stage, prim_path, true)
        }
    }
}

fn reject_if_model_source(
    stage: &Stage,
    path: &str,
    allow_model_member_root: bool,
) -> Result<(), String> {
    for ancestor in ancestors(path) {
        let prim = stage.prim(ancestor.as_str());
        if !prim.is_defined().map_err(|error| error.to_string())? {
            continue;
        }
        let Some(Value::Dictionary(data)) =
            prim.custom_data().map_err(|error| error.to_string())?
        else {
            continue;
        };
        if data
            .get(MODEL_ID_METADATA)
            .and_then(Value::as_str)
            .and_then(|value| ModelId::parse(value).ok())
            .is_some()
        {
            return Err(
                "Model source is immutable; edit its Scene member placement instead".to_owned(),
            );
        }
        let is_model_member = data.get(TARGET_KIND_METADATA).and_then(Value::as_str)
            == Some("model")
            && data
                .get(TARGET_ID_METADATA)
                .and_then(Value::as_str)
                .and_then(|value| ModelId::parse(value).ok())
                .is_some();
        if is_model_member && (!allow_model_member_root || ancestor != path) {
            return Err(
                "Model source is immutable; edit its Scene member placement instead".to_owned(),
            );
        }
    }
    Ok(())
}

fn ancestors(path: &str) -> Vec<String> {
    let mut ancestors = vec!["/".to_owned()];
    let mut current = String::new();
    for component in path.split('/').filter(|component| !component.is_empty()) {
        current.push('/');
        current.push_str(component);
        ancestors.push(current.clone());
    }
    ancestors.reverse();
    ancestors
}

#[cfg(test)]
#[path = "editor_boundary_tests.rs"]
mod tests;
