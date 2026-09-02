use std::path::Path;
use std::sync::Arc;

use usd_model::SemanticSnapshot;
use viewport_protocol::{
    BimPropertiesReadModel, BimPropertyProvenanceReadModel, BimPropertyProvenanceStatus,
    SceneAnchor, SelectionReadModel, ViewportEvent, ViewportEventEnvelope,
};

use crate::viewport::api::BimProvenanceService;
use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::scene::SelectedTargets;
use crate::viewport::semantic::{SemanticDiffState, SemanticSyncState};

pub(super) fn dispatch(
    request_id: String,
    selection: Option<&SelectedTargets>,
    semantic: Option<&SemanticSyncState>,
    semantic_diff: Option<&SemanticDiffState>,
    outbox: &mut ViewportEventOutbox,
) {
    let Some(selection) = selection else {
        reject(
            outbox,
            request_id,
            "BIM selection state is unavailable".to_owned(),
        );
        return;
    };
    let Some(snapshot) = semantic.and_then(|state| state.snapshot()) else {
        reject(
            outbox,
            request_id,
            "BIM semantic snapshot is unavailable".to_owned(),
        );
        return;
    };
    let Some(index) = semantic.and_then(SemanticSyncState::shared_bim_index) else {
        reject(
            outbox,
            request_id,
            "BIM semantic index is unavailable".to_owned(),
        );
        return;
    };

    let selection_model = &selection.0;
    let diff = semantic_diff.and_then(|state| state.bim_property_diff(&selection_model.targets));
    let properties = match read_properties(snapshot, index, selection_model, selection.revision()) {
        Ok(properties) => properties,
        Err(_error)
            if diff.as_ref().is_some_and(|value| {
                value.status == viewport_protocol::BimPropertyDiffStatus::Deleted
            }) =>
        {
            BimPropertiesReadModel {
                targets: selection_model.targets.clone(),
                selection_revision: selection.revision(),
                groups: Vec::new(),
            }
        }
        Err(error) => {
            reject(outbox, request_id, error);
            return;
        }
    };

    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::BimPropertiesRead { properties, diff },
    ));
}

fn read_properties(
    snapshot: &SemanticSnapshot,
    index: Arc<crate::viewport::bim::BimReadIndex>,
    selection: &SelectionReadModel,
    selection_revision: u64,
) -> Result<BimPropertiesReadModel, String> {
    crate::viewport::bim::BimReadService::with_index(snapshot, index)
        .read_properties(
            selection,
            selection_revision,
            crate::viewport::bim::BimReadPolicy {
                allow_value_edit: true,
            },
        )
        .map_err(|error| error.to_string())
}

fn reject(outbox: &mut ViewportEventOutbox, request_id: String, reason: String) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id.clone()),
        ViewportEvent::CommandRejected { request_id, reason },
    ));
}

pub(super) fn submit_provenance(
    request_id: String,
    target: SceneAnchor,
    property: String,
    history_head: String,
    semantic: Option<&SemanticSyncState>,
    semantic_diff: Option<&SemanticDiffState>,
    stage_path: &Path,
    activation_generation: u64,
    service: &BimProvenanceService,
    outbox: &mut ViewportEventOutbox,
) {
    let Some(snapshot) = semantic.and_then(SemanticSyncState::snapshot) else {
        emit_unavailable(request_id, target, property, history_head, outbox);
        return;
    };
    let Some(diff) =
        semantic_diff.and_then(|state| state.bim_property_diff(std::slice::from_ref(&target)))
    else {
        emit_unavailable(request_id, target, property, history_head, outbox);
        return;
    };
    if diff.base_git_oid != history_head {
        emit_unavailable(request_id, target, property, history_head, outbox);
        return;
    }
    if !diff.properties.iter().any(|row| row.key == property) {
        emit_unavailable(request_id, target, property, history_head, outbox);
        return;
    }
    let Some(entity) = snapshot
        .entities
        .values()
        .find(|entity| entity.prim_path == target.prim_path)
    else {
        emit_unavailable(request_id, target, property, history_head, outbox);
        return;
    };

    if service.submit(
        request_id.clone(),
        target.clone(),
        property.clone(),
        entity.key.clone(),
        usd_git::RevisionId::new(history_head),
        stage_path.to_owned(),
        activation_generation,
    ) {
        return;
    }
    reject(
        outbox,
        request_id,
        "BIM property provenance worker is unavailable".to_owned(),
    );
}

pub(super) fn emit_unavailable(
    request_id: String,
    target: SceneAnchor,
    property: String,
    history_head: String,
    outbox: &mut ViewportEventOutbox,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::BimPropertyProvenanceRead {
            provenance: BimPropertyProvenanceReadModel {
                target,
                property,
                history_head,
                status: BimPropertyProvenanceStatus::Unavailable,
                commit_id: None,
                commit_message: None,
                author_name: None,
                author_email: None,
                authored_at_seconds: None,
                old_value: None,
                new_value: None,
            },
        },
    ));
}
