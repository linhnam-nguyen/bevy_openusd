use viewport_protocol::{SceneAnchor, SelectionReadModel, ViewportEvent, ViewportEventEnvelope};

use super::super::helpers::{reject, resolve_anchor};
use crate::viewport::api::{SceneAnchorIndex, ViewportEventOutbox};
use crate::viewport::scene::{SelectedPrim, SelectedTargets};

pub(super) fn select_target(
    request_id: String,
    target: Option<SceneAnchor>,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
    scene_index: &SceneAnchorIndex,
) {
    match target {
        None => {
            selected_prim.0 = None;
            selection
                .replace(SelectionReadModel::default())
                .expect("empty selection must satisfy the protocol invariant");
        }
        Some(anchor) => match resolve_anchor(&anchor, scene_index) {
            Ok(entity) => {
                selected_prim.0 = Some(entity);
                selection
                    .replace(SelectionReadModel::from_legacy_target(Some(anchor)))
                    .expect("resolved selection must satisfy the protocol invariant");
            }
            Err(reason) => {
                reject(outbox, request_id, reason);
                return;
            }
        },
    }
    emit_selection_changed(request_id, &selection.0, outbox);
}

pub(super) fn replace_selection(
    request_id: String,
    targets: Vec<SceneAnchor>,
    primary: Option<SceneAnchor>,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
    scene_index: &SceneAnchorIndex,
) {
    let mut next_selection = SelectionReadModel { targets, primary };
    if let Err(error) = next_selection.canonicalize() {
        reject(outbox, request_id, error.to_string());
        return;
    }

    let mut resolved_primary = None;
    for target in &next_selection.targets {
        let Ok(entity) = resolve_anchor(target, scene_index) else {
            reject(
                outbox,
                request_id,
                format!(
                    "target {} is not present in the active scene",
                    target.prim_path
                ),
            );
            return;
        };
        if next_selection.primary.as_ref() == Some(target) {
            resolved_primary = Some(entity);
        }
    }

    selection
        .replace(next_selection)
        .expect("validated selection must satisfy the protocol invariant");
    selected_prim.0 = resolved_primary;
    emit_selection_changed(request_id, &selection.0, outbox);
}

pub(super) fn add_selection_target(
    request_id: String,
    target: SceneAnchor,
    make_primary: bool,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
    scene_index: &SceneAnchorIndex,
) {
    if resolve_anchor(&target, scene_index).is_err() {
        reject(
            outbox,
            request_id,
            format!(
                "target {} is not present in the active scene",
                target.prim_path
            ),
        );
        return;
    }
    if let Err(error) = selection.add(target, make_primary) {
        reject(outbox, request_id, error.to_string());
        return;
    }
    selected_prim.0 = selection
        .0
        .primary
        .as_ref()
        .and_then(|primary| scene_index.resolve(primary));
    emit_selection_changed(request_id, &selection.0, outbox);
}

pub(super) fn remove_selection_target(
    request_id: String,
    target: SceneAnchor,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
    scene_index: &SceneAnchorIndex,
) {
    if let Err(error) = selection.remove(&target) {
        reject(outbox, request_id, error.to_string());
        return;
    }
    selected_prim.0 = selection
        .0
        .primary
        .as_ref()
        .and_then(|primary| scene_index.resolve(primary));
    emit_selection_changed(request_id, &selection.0, outbox);
}

fn emit_selection_changed(
    request_id: String,
    selection: &SelectionReadModel,
    outbox: &mut ViewportEventOutbox,
) {
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id.clone()),
        ViewportEvent::SelectionChanged {
            selection: selection.clone(),
        },
    ));
}
