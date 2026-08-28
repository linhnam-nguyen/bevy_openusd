use usd_model::SemanticSnapshot;
use viewport_protocol::{
    BimPropertiesReadModel, SelectionReadModel, ViewportEvent, ViewportEventEnvelope,
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
                properties: Vec::new(),
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
