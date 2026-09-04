use viewport_protocol::{SceneAnchor, SelectionReadModel};

use super::super::helpers::{emit_selection_delta, reject};
use crate::viewport::api::{SceneAnchorIndex, ViewportEventOutbox};
use crate::viewport::scene::{SelectedPrim, SelectedTargets};

pub(crate) fn select_target(
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
                .clear()
                .expect("empty selection must satisfy the protocol invariant");
        }
        Some(anchor) => {
            if let Err(error) = anchor.validate() {
                reject(outbox, request_id, error.to_string());
                return;
            }
            selected_prim.0 = scene_index.resolve(&anchor);
            selection
                .replace(SelectionReadModel::from_legacy_target(Some(anchor)))
                .expect("validated selection must satisfy the protocol invariant");
        }
    }
    emit_selection_delta(request_id, selection, outbox);
}

pub(crate) fn replace_selection(
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

    selected_prim.0 = next_selection
        .primary
        .as_ref()
        .and_then(|target| scene_index.resolve(target));
    selection
        .replace(next_selection)
        .expect("validated selection must satisfy the protocol invariant");
    emit_selection_delta(request_id, selection, outbox);
}

pub(crate) fn add_selection_target(
    request_id: String,
    target: SceneAnchor,
    make_primary: bool,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
    scene_index: &SceneAnchorIndex,
) {
    add_selection_targets(
        request_id,
        vec![target.clone()],
        make_primary.then_some(target),
        outbox,
        selected_prim,
        selection,
        scene_index,
    );
}

pub(crate) fn add_selection_targets(
    request_id: String,
    targets: Vec<SceneAnchor>,
    primary: Option<SceneAnchor>,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
    scene_index: &SceneAnchorIndex,
) {
    for target in &targets {
        if let Err(error) = target.validate() {
            reject(outbox, request_id, error.to_string());
            return;
        }
    }
    if let Err(error) = selection.add_many(targets, primary) {
        reject(outbox, request_id, error.to_string());
        return;
    }
    selected_prim.0 = selection
        .0
        .primary
        .as_ref()
        .and_then(|primary| scene_index.resolve(primary));
    emit_selection_delta(request_id, selection, outbox);
}

pub(crate) fn remove_selection_target(
    request_id: String,
    target: SceneAnchor,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
    scene_index: &SceneAnchorIndex,
) {
    remove_selection_targets(
        request_id,
        vec![target],
        outbox,
        selected_prim,
        selection,
        scene_index,
    );
}

pub(crate) fn remove_selection_targets(
    request_id: String,
    targets: Vec<SceneAnchor>,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
    _scene_index: &SceneAnchorIndex,
) {
    if let Err(error) = selection.remove_many(targets) {
        reject(outbox, request_id, error.to_string());
        return;
    }
    selected_prim.0 = selection
        .0
        .primary
        .as_ref()
        .and_then(|primary| _scene_index.resolve(primary));
    emit_selection_delta(request_id, selection, outbox);
}

pub(crate) fn clear_selection(
    request_id: String,
    outbox: &mut ViewportEventOutbox,
    selected_prim: &mut SelectedPrim,
    selection: &mut SelectedTargets,
) {
    if let Err(error) = selection.clear() {
        reject(outbox, request_id, error.to_string());
        return;
    }
    selected_prim.0 = None;
    emit_selection_delta(request_id, selection, outbox);
}
