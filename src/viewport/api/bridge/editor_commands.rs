use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_bevy::authoring::AttributeEdit;
use usd_model::SemanticSnapshot;
use viewport_protocol::bim::validate_bim_mutation_batch;
use viewport_protocol::{
    BimPropertyEditOutcome, BimPropertyEditStatus, BimPropertyMutation, EditorOperation,
    ViewportCommand, ViewportEvent, ViewportEventEnvelope,
};

use super::convert::editor_value_to_usd;
use super::helpers::{
    emit_editor_completed, emit_editor_export, emit_runtime_mutation_accepted, reject,
};
use super::mutations::apply_runtime_mutations;
use super::state::{EditorHistories, EditorHistoryDomain, RuntimeMutationCoordinator};
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::bim::authoring::{
    BimAuthoringError, canonical_value_for_comparison, current_bim_value, prepare_bim_value,
    resolve_bim_authoring_locator,
};

/// Handles authoring-class commands (DefinePrim, SetAttribute, Undo, Export, …)
/// that require a loaded `LiveStage`. The return value is `true` when the
/// command was consumed and `false` when it should be handled by the caller.
pub(super) fn apply_editor_command(
    command: ViewportCommand,
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    histories: &mut EditorHistories,
    runtime_mutations: &mut RuntimeMutationCoordinator,
    semantic_snapshot: Option<&SemanticSnapshot>,
    stage: Option<&LiveStage>,
) -> bool {
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
            stage.mark_authored(path.clone());
            if let Err(error) = histories.authoring.define(&stage.stage, &path, &type_name) {
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
            stage.mark_authored(path.clone());
            let removed = match usd_bevy::authoring::remove_prim(&stage.stage, &path) {
                Ok(r) => r,
                Err(e) => {
                    reject(outbox, request_id, e.to_string());
                    return true;
                }
            };
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
            stage.mark_authored(path.clone());
            if let Err(e) = histories.authoring.rename(&stage.stage, &path, &new_name) {
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
            stage.mark_authored(path.clone());
            if let Err(e) = histories
                .authoring
                .reparent(&stage.stage, &path, &new_parent)
            {
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
            stage.mark_authored(old_path.clone());
            if let Err(e) = usd_bevy::authoring::move_prim(&stage.stage, &old_path, &new_path) {
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
            stage.mark_authored(prim_path.clone());
            if let Err(e) =
                histories
                    .authoring
                    .set_attr(&stage.stage, &prim_path, &name, &type_name, value)
            {
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
        ViewportCommand::EditBimProperty { mutation } => {
            let stage = require_stage!();
            let outcome = if let Err(error) = mutation.validate() {
                rejected_bim_mutation(&mutation, error.to_string())
            } else {
                apply_bim_property_mutation(stage, histories, semantic_snapshot, &mutation)
            };
            emit_bim_property_completed(
                outbox,
                request_id,
                outcome,
                stage.current_revision().0,
                histories,
            );
        }
        ViewportCommand::EditBimProperties { mutations } => {
            let stage = require_stage!();
            let (outcomes, applied) =
                apply_bim_property_mutations(stage, histories, semantic_snapshot, &mutations);
            emit_bim_property_batch_completed(
                outbox,
                request_id,
                outcomes,
                applied,
                stage.current_revision().0,
                histories,
            );
        }
        ViewportCommand::ClearAttribute { prim_path, name } => {
            let stage = require_stage!();
            stage.mark_authored(prim_path.clone());
            if let Err(e) = usd_bevy::authoring::clear_attribute(&stage.stage, &prim_path, &name) {
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
            stage.mark_authored(prim_path.clone());
            let transform = Transform {
                translation: Vec3::from_array(translation),
                rotation: Quat::from_array(rotation),
                scale: Vec3::from_array(scale),
            };
            if let Err(e) = histories
                .transforms
                .author(&stage.stage, &prim_path, transform)
            {
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
            let stage = require_stage!();
            if let Err(e) = usd_bevy::authoring::save_stage_as(&stage.stage, &filename) {
                reject(outbox, request_id, e.to_string());
                return true;
            }
            emit_editor_completed(
                outbox,
                request_id,
                EditorOperation::SaveStageAs,
                Vec::new(),
                histories,
            );
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

fn apply_bim_property_mutation(
    stage: &LiveStage,
    histories: &mut EditorHistories,
    semantic_snapshot: Option<&SemanticSnapshot>,
    mutation: &BimPropertyMutation,
) -> BimPropertyEditOutcome {
    let prepared = match prepare_bim_mutation(stage, semantic_snapshot, mutation) {
        Ok(prepared) => prepared,
        Err(error) => return rejected_bim_error(mutation, error),
    };
    stage.mark_authored(prepared.locator.prim_path.clone());
    if let Err(error) = histories.authoring.set_attr(
        &stage.stage,
        &prepared.locator.prim_path,
        &prepared.locator.property_key,
        prepared.locator.type_name.as_deref().unwrap_or_default(),
        prepared.authored.clone(),
    ) {
        return rejected_bim_error(mutation, BimAuthoringError::Stage(error.to_string()));
    }
    histories.record(EditorHistoryDomain::Authoring);
    BimPropertyEditOutcome {
        target: mutation.target.clone(),
        property: mutation.property.clone(),
        status: BimPropertyEditStatus::Applied,
        old_value: Some(prepared.current),
        new_value: Some(prepared.new_value),
        reason: None,
    }
}

struct PreparedBimMutation {
    locator: crate::viewport::bim::authoring::BimAuthoringLocator,
    authored: openusd::sdf::Value,
    current: usd_model::CanonicalValue,
    new_value: usd_model::CanonicalValue,
}

fn prepare_bim_mutation(
    stage: &LiveStage,
    semantic_snapshot: Option<&SemanticSnapshot>,
    mutation: &BimPropertyMutation,
) -> Result<PreparedBimMutation, BimAuthoringError> {
    let locator =
        resolve_bim_authoring_locator(&stage.stage, &mutation.target, &mutation.property)?;
    let current_raw = current_bim_value(&stage.stage, &locator)?;
    let measurement = semantic_snapshot.and_then(|snapshot| {
        snapshot
            .entities
            .values()
            .find(|entity| entity.prim_path == mutation.target.prim_path)
            .and_then(|entity| {
                entity
                    .properties
                    .iter()
                    .find(|property| property.name == mutation.property)
            })
            .and_then(|property| property.measurement.as_ref())
    });
    let current = canonical_value_for_comparison(current_raw, measurement)?;
    if current != mutation.expected_old_value {
        return Err(BimAuthoringError::ExpectedValueMismatch {
            property_key: mutation.property.clone(),
            expected: mutation.expected_old_value.clone(),
            current,
        });
    }
    let (authored, new_value) = prepare_bim_value(
        &locator,
        &mutation.value,
        mutation.input_unit.as_ref(),
        measurement,
    )?;
    Ok(PreparedBimMutation {
        locator,
        authored,
        current: mutation.expected_old_value.clone(),
        new_value,
    })
}

fn apply_bim_property_mutations(
    stage: &LiveStage,
    histories: &mut EditorHistories,
    semantic_snapshot: Option<&SemanticSnapshot>,
    mutations: &[BimPropertyMutation],
) -> (Vec<BimPropertyEditOutcome>, bool) {
    if let Err(error) = validate_bim_mutation_batch(mutations) {
        return (
            mutations
                .iter()
                .map(|mutation| rejected_bim_mutation(mutation, error.to_string()))
                .collect(),
            false,
        );
    }

    let prepared = mutations
        .iter()
        .map(|mutation| prepare_bim_mutation(stage, semantic_snapshot, mutation))
        .collect::<Vec<_>>();
    let Some(first_error) = prepared.iter().find_map(|result| result.as_ref().err()) else {
        let prepared = prepared.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        let edits = prepared
            .iter()
            .map(|mutation| AttributeEdit {
                prim: mutation.locator.prim_path.clone(),
                name: mutation.locator.property_key.clone(),
                type_name: mutation
                    .locator
                    .type_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_owned(),
                value: mutation.authored.clone(),
            })
            .collect::<Vec<_>>();
        for mutation in &prepared {
            stage.mark_authored(mutation.locator.prim_path.clone());
        }
        if let Err(error) = histories.authoring.set_attrs_atomic(&stage.stage, &edits) {
            let reason = format!("BIM batch authoring failed atomically: {error}");
            return (
                mutations
                    .iter()
                    .map(|mutation| rejected_bim_mutation(mutation, reason.clone()))
                    .collect(),
                false,
            );
        }
        histories.record(EditorHistoryDomain::Authoring);
        return (
            mutations
                .iter()
                .zip(prepared)
                .map(|(mutation, prepared)| BimPropertyEditOutcome {
                    target: mutation.target.clone(),
                    property: mutation.property.clone(),
                    status: BimPropertyEditStatus::Applied,
                    old_value: Some(prepared.current),
                    new_value: Some(prepared.new_value),
                    reason: None,
                })
                .collect(),
            true,
        );
    };

    let reason = format!("BIM batch rejected atomically: {first_error}");
    (
        mutations
            .iter()
            .zip(prepared)
            .map(|(mutation, result)| match result {
                Ok(_) => rejected_bim_mutation(mutation, reason.clone()),
                Err(error) => rejected_bim_error(mutation, error),
            })
            .collect(),
        false,
    )
}

fn rejected_bim_error(
    mutation: &BimPropertyMutation,
    error: BimAuthoringError,
) -> BimPropertyEditOutcome {
    let old_value = match &error {
        BimAuthoringError::ExpectedValueMismatch { current, .. } => Some(current.clone()),
        _ => None,
    };
    BimPropertyEditOutcome {
        target: mutation.target.clone(),
        property: mutation.property.clone(),
        status: BimPropertyEditStatus::Rejected,
        old_value,
        new_value: None,
        reason: Some(error.to_string()),
    }
}

fn rejected_bim_mutation(mutation: &BimPropertyMutation, reason: String) -> BimPropertyEditOutcome {
    BimPropertyEditOutcome {
        target: mutation.target.clone(),
        property: mutation.property.clone(),
        status: BimPropertyEditStatus::Rejected,
        old_value: None,
        new_value: None,
        reason: Some(reason),
    }
}

fn emit_bim_property_completed(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    outcome: BimPropertyEditOutcome,
    live_revision: u64,
    histories: &EditorHistories,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::BimPropertyEditCompleted {
            outcome,
            live_revision,
            state: histories.state(),
        },
    ));
}

fn emit_bim_property_batch_completed(
    outbox: &mut ViewportEventOutbox,
    request_id: String,
    outcomes: Vec<BimPropertyEditOutcome>,
    applied: bool,
    live_revision: u64,
    histories: &EditorHistories,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::BimPropertyBatchEditCompleted {
            outcomes,
            applied,
            live_revision,
            state: histories.state(),
        },
    ));
}
