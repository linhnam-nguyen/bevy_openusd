use viewport_protocol::{SceneAnchor, SelectionReadModel, ViewportEvent, ViewportEventEnvelope};

use super::super::helpers::{emit_viewer_settings_changed, reject, resolve_anchor};
use super::SelectionState;
use crate::viewport::api::{SceneAnchorIndex, ViewportEventOutbox};

pub(super) fn select_target(
    request_id: String,
    target: Option<SceneAnchor>,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
    scene_index: &SceneAnchorIndex,
) {
    let next_selection = match target {
        None => {
            selection_state.p0().0 = None;
            SelectionReadModel::default()
        }
        Some(anchor) => match resolve_anchor(&anchor, scene_index) {
            Ok(entity) => {
                selection_state.p0().0 = Some(entity);
                SelectionReadModel::from_legacy_target(Some(anchor))
            }
            Err(reason) => {
                reject(outbox, request_id, reason);
                return;
            }
        },
    };
    selection_state
        .p1()
        .replace(next_selection.clone())
        .expect("resolved selection must satisfy the protocol invariant");
    emit_selection_changed(request_id, next_selection, outbox, selection_state);
}

pub(super) fn replace_selection(
    request_id: String,
    targets: Vec<SceneAnchor>,
    primary: Option<SceneAnchor>,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
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

    selection_state
        .p1()
        .replace(next_selection.clone())
        .expect("validated selection must satisfy the protocol invariant");
    selection_state.p0().0 = resolved_primary;
    emit_selection_changed(request_id, next_selection, outbox, selection_state);
}

pub(super) fn add_selection_target(
    request_id: String,
    target: SceneAnchor,
    make_primary: bool,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
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
    if let Err(error) = selection_state.p1().add(target, make_primary) {
        reject(outbox, request_id, error.to_string());
        return;
    }
    let next_selection = selection_state.p1().0.clone();
    selection_state.p0().0 = next_selection
        .primary
        .as_ref()
        .and_then(|primary| scene_index.resolve(primary));
    emit_selection_changed(request_id, next_selection, outbox, selection_state);
}

pub(super) fn remove_selection_target(
    request_id: String,
    target: SceneAnchor,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
    scene_index: &SceneAnchorIndex,
) {
    if let Err(error) = selection_state.p1().remove(&target) {
        reject(outbox, request_id, error.to_string());
        return;
    }
    let next_selection = selection_state.p1().0.clone();
    selection_state.p0().0 = next_selection
        .primary
        .as_ref()
        .and_then(|primary| scene_index.resolve(primary));
    emit_selection_changed(request_id, next_selection, outbox, selection_state);
}

fn emit_selection_changed(
    request_id: String,
    selection: SelectionReadModel,
    outbox: &mut ViewportEventOutbox,
    selection_state: &mut SelectionState<'_, '_>,
) {
    let settings_changed = {
        let mut settings = selection_state.p2();
        settings
            .sync_section_box_selection(&selection)
            .then(|| settings.0.clone())
    };
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id.clone()),
        ViewportEvent::SelectionChanged { selection },
    ));
    if let Some(settings) = settings_changed {
        emit_viewer_settings_changed(outbox, request_id, &settings);
    }
}
