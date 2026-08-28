use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_model::SemanticSnapshot;
use viewport_protocol::{EditorOperation, ViewportCommand, ViewportEvent, ViewportEventEnvelope};

use super::convert::editor_value_to_usd;
use super::helpers::{
    emit_editor_completed, emit_editor_export, emit_runtime_mutation_accepted, reject,
};
use super::mutations::apply_runtime_mutations;
use super::save;
use super::state::{EditorHistories, EditorHistoryDomain, RuntimeMutationCoordinator};
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::scene::SelectedTargets;
pub(super) fn apply_editor_command(
    command: ViewportCommand,
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    runtime_mutations: &mut RuntimeMutationCoordinator,
    semantic_snapshot: Option<&SemanticSnapshot>,
    stage: Option<&LiveStage>,
    save_stage_path: Option<&std::path::Path>,
    selected_targets: &SelectedTargets,
) -> bool {
    let (command, request_id) = match super::bim_commands::try_apply_bim_command(
        command,
        request_id,
        outbox,
        histories,
        semantic_snapshot,
        stage,
        selected_targets,
    ) {
        Ok(result) => return result,
        Err(command_and_request) => command_and_request,
    };
    macro_rules! require_stage {
        () => {
            match stage {
                Some(s) => s,
                None => {
                    reject(outbox, request_id, "stage is not loaded".to_owned());
                    return true;
                }
            }
        };
    }
    match command {
        ViewportCommand::DefinePrim { path, type_name } => {
            let stage = require_stage!();
            if let Err(error) = stage.with_authored(path.clone(), || {
                histories.authoring.define(&stage.stage, &path, &type_name)
            }) {
                reject(outbox, request_id, error.to_string());
                return true;
            }
            histories.record(EditorHistoryDomain::Authoring);
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::DefinePrim,
                vec![path],
                histories,
            );
        }
        ViewportCommand::RemovePrim { path } => {
            let stage = require_stage!();
            let suppression = stage.mark_authored_guard(path.clone());
            let removed = match usd_bevy::authoring::remove_prim(&stage.stage, &path) {
                Ok(r) => r,
                Err(e) => {
                    reject(outbox, request_id, e.to_string());
                    return true;
                }
            };
            if removed {
                suppression.commit();
            }
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::RemovePrim,
                removed.then_some(path).into_iter().collect(),
                histories,
            );
        }
        ViewportCommand::RenamePrim { path, new_name } => {
            let stage = require_stage!();
            if let Err(e) = stage.with_authored(path.clone(), || {
                histories.authoring.rename(&stage.stage, &path, &new_name)
            }) {
                reject(outbox, request_id, e.to_string());
                return true;
            }
            histories.record(EditorHistoryDomain::Authoring);
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::RenamePrim,
                vec![path],
                histories,
            );
        }
        ViewportCommand::ReparentPrim { path, new_parent } => {
            let stage = require_stage!();
            if let Err(e) = stage.with_authored(path.clone(), || {
                histories
                    .authoring
                    .reparent(&stage.stage, &path, &new_parent)
            }) {
                reject(outbox, request_id, e.to_string());
                return true;
            }
            histories.record(EditorHistoryDomain::Authoring);
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::ReparentPrim,
                vec![path, new_parent],
                histories,
            );
        }
        ViewportCommand::MovePrim { old_path, new_path } => {
            let stage = require_stage!();
            if let Err(e) = stage.with_authored(old_path.clone(), || {
                usd_bevy::authoring::move_prim(&stage.stage, &old_path, &new_path)
            }) {
                reject(outbox, request_id, e.to_string());
                return true;
            }
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::MovePrim,
                vec![old_path, new_path],
                histories,
            );
        }
        ViewportCommand::SetAttribute {
            prim_path,
            name,
            type_name,
            value,
        } => {
            let stage = require_stage!();
            let value = match editor_value_to_usd(&type_name, &value) {
                Ok(v) => v,
                Err(e) => {
                    reject(outbox, request_id, e);
                    return true;
                }
            };
            if let Err(e) = stage.with_authored(prim_path.clone(), || {
                histories
                    .authoring
                    .set_attr(&stage.stage, &prim_path, &name, &type_name, value)
            }) {
                reject(outbox, request_id, e.to_string());
                return true;
            }
            histories.record(EditorHistoryDomain::Authoring);
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::SetAttribute,
                vec![format!("{prim_path}.{name}")],
                histories,
            );
        }
        ViewportCommand::ClearAttribute { prim_path, name } => {
            let stage = require_stage!();
            if let Err(e) = stage.with_authored(prim_path.clone(), || {
                usd_bevy::authoring::clear_attribute(&stage.stage, &prim_path, &name)
            }) {
                reject(outbox, request_id, e.to_string());
                return true;
            }
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::ClearAttribute,
                vec![format!("{prim_path}.{name}")],
                histories,
            );
        }
        ViewportCommand::SetTransform {
            prim_path,
            translation,
            rotation,
            scale,
        } => {
            let stage = require_stage!();
            let transform = Transform {
                translation: Vec3::from_array(translation),
                rotation: Quat::from_array(rotation),
                scale: Vec3::from_array(scale),
            };
            if let Err(e) = stage.with_authored(prim_path.clone(), || {
                histories
                    .transforms
                    .author(&stage.stage, &prim_path, transform)
            }) {
                reject(outbox, request_id, e.to_string());
                return true;
            }
            histories.record(EditorHistoryDomain::Transform);
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::SetTransform,
                vec![prim_path],
                histories,
            );
        }
        ViewportCommand::LoadPayload { prim_path } => {
            let stage = require_stage!();
            if !usd_bevy::authoring::prim_exists(&stage.stage, &prim_path) {
                reject(
                    outbox,
                    request_id,
                    format!("prim {prim_path} does not exist"),
                );
                return true;
            }
            stage.load_payload(&prim_path);
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::LoadPayload,
                vec![prim_path],
                histories,
            );
        }
        ViewportCommand::UnloadPayload { prim_path } => {
            let stage = require_stage!();
            if !usd_bevy::authoring::prim_exists(&stage.stage, &prim_path) {
                reject(
                    outbox,
                    request_id,
                    format!("prim {prim_path} does not exist"),
                );
                return true;
            }
            stage.unload_payload(&prim_path);
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::UnloadPayload,
                vec![prim_path],
                histories,
            );
        }
        ViewportCommand::UndoEditor => {
            let stage = require_stage!();
            let Some(domain) = histories.undo_domains.pop() else {
                reject(outbox, request_id, "editor history is empty".to_owned());
                return true;
            };
            let result = match domain {
                EditorHistoryDomain::Authoring => histories.authoring.undo(&stage.stage),
                EditorHistoryDomain::Transform => histories.transforms.undo(&stage.stage),
            };
            match result {
                Ok(true) => {
                    histories.redo_domains.push(domain);
                    emit_editor_completed(
                        outbox,
                        request_id,
                        EditorOperation::Undo,
                        Vec::new(),
                        histories,
                    );
                }
                Ok(false) => reject(outbox, request_id, "editor history is empty".to_owned()),
                Err(e) => {
                    histories.undo_domains.push(domain);
                    reject(outbox, request_id, e.to_string());
                }
            }
        }
        ViewportCommand::RedoEditor => {
            let stage = require_stage!();
            let Some(domain) = histories.redo_domains.pop() else {
                reject(
                    outbox,
                    request_id,
                    "editor redo history is empty".to_owned(),
                );
                return true;
            };
            let result = match domain {
                EditorHistoryDomain::Authoring => histories.authoring.redo(&stage.stage),
                EditorHistoryDomain::Transform => histories.transforms.redo(&stage.stage),
            };
            match result {
                Ok(true) => {
                    histories.undo_domains.push(domain);
                    emit_editor_completed(
                        outbox,
                        request_id,
                        EditorOperation::Redo,
                        Vec::new(),
                        histories,
                    );
                }
                Ok(false) => reject(
                    outbox,
                    request_id,
                    "editor redo history is empty".to_owned(),
                ),
                Err(e) => {
                    histories.redo_domains.push(domain);
                    reject(outbox, request_id, e.to_string());
                }
            }
        }
        ViewportCommand::SaveStageAs { filename } => {
            save::save_stage_as(request_id, outbox, histories, stage, &filename);
        }
        ViewportCommand::SaveStage => {
            save::save_current_stage(request_id, outbox, histories, stage, save_stage_path);
        }
        ViewportCommand::ExportStage => {
            let stage = require_stage!();
            let content = match usd_bevy::authoring::export_stage_string(&stage.stage) {
                Ok(c) => c,
                Err(e) => {
                    reject(outbox, request_id, e.to_string());
                    return true;
                }
            };
            emit_editor_export(outbox, &request_id, &content);
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::ExportStage,
                Vec::new(),
                histories,
            );
        }
        ViewportCommand::ApplyRuntimeMutationBatch { batch } => {
            let stage = require_stage!();
            if let Err(e) = batch.validate() {
                reject(outbox, request_id, e.to_string());
                return true;
            }
            if let Err(e) = runtime_mutations.admit(stage, &batch) {
                reject(outbox, request_id, e);
                return true;
            }
            let changed_paths = match apply_runtime_mutations(stage, histories, &batch) {
                Ok(p) => p,
                Err(e) => {
                    reject(outbox, request_id, e);
                    return true;
                }
            };
            runtime_mutations.record(&batch);
            emit_runtime_mutation_accepted(outbox, request_id, &batch, changed_paths, histories);
        }
        ViewportCommand::QueryPrim { prim_path } => {
            let stage = require_stage!();
            outbox.push(ViewportEventEnvelope::new(
                Some(request_id),
                ViewportEvent::EditorPrimState {
                    prim: viewport_protocol::EditorPrimReadModel {
                        prim_path: prim_path.clone(),
                        exists: usd_bevy::authoring::prim_exists(&stage.stage, &prim_path),
                    },
                },
            ));
        }
        // All other commands are handled by apply_viewport_commands.
        _ => return false,
    }
    true
}
