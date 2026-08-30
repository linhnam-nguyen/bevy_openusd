use bevy::prelude::*;
use usd_bevy::LiveStage;
use viewport_protocol::{RuntimeMutation, RuntimeMutationBatch};

use super::convert::editor_value_to_usd;
use super::state::{EditorHistories, EditorHistoryDomain};

/// Applies every mutation in a batch against the live stage.
///
/// Returns the list of changed prim paths on success, or an error string that
/// should be surfaced as a command rejection.
pub(super) fn apply_runtime_mutations(
    stage: &LiveStage,
    histories: &mut EditorHistories,
    batch: &RuntimeMutationBatch,
) -> Result<Vec<String>, String> {
    if stage.is_authoring_frozen() {
        return Err("LiveStage authoring is leased by Project publication".to_owned());
    }
    let mut changed_paths = Vec::new();

    for mutation in &batch.operations {
        match mutation {
            RuntimeMutation::DefinePrim { path, type_name } => {
                stage.mark_authored(path.clone());
                histories
                    .authoring
                    .define(&stage.stage, path, type_name)
                    .map_err(|error| error.to_string())?;
                histories.record(EditorHistoryDomain::Authoring);
                changed_paths.push(path.clone());
            }
            RuntimeMutation::RemovePrim { path } => {
                stage.mark_authored(path.clone());
                if usd_bevy::authoring::remove_prim(&stage.stage, path)
                    .map_err(|error| error.to_string())?
                {
                    changed_paths.push(path.clone());
                }
            }
            RuntimeMutation::RenamePrim { path, new_name } => {
                stage.mark_authored(path.clone());
                histories
                    .authoring
                    .rename(&stage.stage, path, new_name)
                    .map_err(|error| error.to_string())?;
                histories.record(EditorHistoryDomain::Authoring);
                changed_paths.push(path.clone());
            }
            RuntimeMutation::ReparentPrim { path, new_parent } => {
                stage.mark_authored(path.clone());
                histories
                    .authoring
                    .reparent(&stage.stage, path, new_parent)
                    .map_err(|error| error.to_string())?;
                histories.record(EditorHistoryDomain::Authoring);
                changed_paths.extend([path.clone(), new_parent.clone()]);
            }
            RuntimeMutation::MovePrim { old_path, new_path } => {
                stage.mark_authored(old_path.clone());
                usd_bevy::authoring::move_prim(&stage.stage, old_path, new_path)
                    .map_err(|error| error.to_string())?;
                changed_paths.extend([old_path.clone(), new_path.clone()]);
            }
            RuntimeMutation::SetAttribute {
                prim_path,
                name,
                type_name,
                value,
            } => {
                let value = editor_value_to_usd(type_name, value)?;
                stage.mark_authored(prim_path.clone());
                histories
                    .authoring
                    .set_attr(&stage.stage, prim_path, name, type_name, value)
                    .map_err(|error| error.to_string())?;
                histories.record(EditorHistoryDomain::Authoring);
                changed_paths.push(format!("{prim_path}.{name}"));
            }
            RuntimeMutation::ClearAttribute { prim_path, name } => {
                stage.mark_authored(prim_path.clone());
                usd_bevy::authoring::clear_attribute(&stage.stage, prim_path, name)
                    .map_err(|error| error.to_string())?;
                changed_paths.push(format!("{prim_path}.{name}"));
            }
            RuntimeMutation::SetTransform {
                prim_path,
                translation,
                rotation,
                scale,
            } => {
                stage.mark_authored(prim_path.clone());
                histories
                    .transforms
                    .author(
                        &stage.stage,
                        prim_path,
                        Transform {
                            translation: Vec3::from_array(*translation),
                            rotation: Quat::from_array(*rotation),
                            scale: Vec3::from_array(*scale),
                        },
                    )
                    .map_err(|error| error.to_string())?;
                histories.record(EditorHistoryDomain::Transform);
                changed_paths.push(prim_path.clone());
            }
            RuntimeMutation::SetVariantSelection {
                prim_path,
                set_name,
                option,
            } => {
                stage.mark_authored(prim_path.clone());
                histories
                    .authoring
                    .set_variant(&stage.stage, prim_path, set_name, option)
                    .map_err(|error| error.to_string())?;
                histories.record(EditorHistoryDomain::Authoring);
                changed_paths.push(format!("{prim_path}.{set_name}"));
            }
        }
    }

    Ok(changed_paths)
}
