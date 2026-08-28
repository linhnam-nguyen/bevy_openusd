use std::path::Path;

use usd_model::SemanticSnapshot;
use viewport_protocol::{
    BimPropertiesReadModel, BimPropertyProvenanceReadModel, BimPropertyProvenanceStatus,
    SceneAnchor, SelectionReadModel, ViewportEvent, ViewportEventEnvelope,
};

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

    let selection_model = &selection.0;
    let diff = semantic_diff.and_then(|state| state.bim_property_diff(&selection_model.targets));
    let properties = match read_properties(snapshot, selection_model, selection.revision()) {
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
    selection: &SelectionReadModel,
    selection_revision: u64,
) -> Result<BimPropertiesReadModel, String> {
    crate::viewport::bim::BimReadService::new(snapshot)
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

pub(super) fn dispatch_provenance(
    request_id: String,
    target: SceneAnchor,
    property: String,
    semantic_diff: Option<&SemanticDiffState>,
    stage_path: &Path,
    outbox: &mut ViewportEventOutbox,
) {
    let Some(diff) =
        semantic_diff.and_then(|state| state.bim_property_diff(std::slice::from_ref(&target)))
    else {
        emit_unavailable(request_id, target, property, outbox);
        return;
    };
    let Some(row) = diff.properties.iter().find(|row| row.key == property) else {
        emit_unavailable(request_id, target, property, outbox);
        return;
    };

    let repository = match usd_git::Repository::open(stage_path) {
        Ok(repository) => repository,
        Err(error) => {
            reject(
                outbox,
                request_id,
                format!("BIM property provenance is unavailable: {error}"),
            );
            return;
        }
    };
    let commit = match usd_git::GitRepository::read_commit(
        &repository,
        &usd_git::RevisionId::new(diff.base_git_oid.clone()),
    ) {
        Ok(commit) => commit,
        Err(error) => {
            reject(
                outbox,
                request_id,
                format!("BIM property provenance commit could not be read: {error}"),
            );
            return;
        }
    };

    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::BimPropertyProvenanceRead {
            provenance: BimPropertyProvenanceReadModel {
                target,
                property,
                status: BimPropertyProvenanceStatus::Available,
                commit_id: Some(commit.id.to_string()),
                commit_message: Some(commit.message),
                author_name: Some(commit.author.name),
                author_email: Some(commit.author.email),
                authored_at_seconds: Some(commit.author.time_seconds),
                old_value: row.old_value.clone(),
                new_value: row.new_value.clone(),
            },
        },
    ));
}

pub(super) fn emit_unavailable(
    request_id: String,
    target: SceneAnchor,
    property: String,
    outbox: &mut ViewportEventOutbox,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::BimPropertyProvenanceRead {
            provenance: BimPropertyProvenanceReadModel {
                target,
                property,
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
