//! Authoritative BIM property edit commands.

use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_bevy::authoring::AttributeEdit;
use usd_model::SemanticSnapshot;
use viewport_protocol::bim::validate_bim_mutation_batch;
use viewport_protocol::{
    BimPropertyEditOutcome, BimPropertyEditStatus, BimPropertyMutation, ViewportEvent,
    ViewportEventEnvelope,
};

use super::state::{EditorHistories, EditorHistoryDomain};
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::bim::authoring::{
    BimAuthoringError, BimAuthoringLocator, canonical_value_for_comparison, current_bim_value,
    prepare_bim_value, resolve_bim_authoring_locator,
};

pub(super) fn apply_bim_property_mutation(
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
    locator: BimAuthoringLocator,
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

pub(super) fn apply_bim_property_mutations(
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

pub(super) fn emit_bim_property_completed(
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

pub(super) fn emit_bim_property_batch_completed(
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
