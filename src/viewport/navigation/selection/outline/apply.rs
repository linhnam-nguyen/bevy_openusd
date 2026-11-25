use bevy::prelude::*;
use bevy_mod_outline::{OutlineStencil, OutlineVolume};

use super::{
    MAX_PRESENTATION_ENTITIES_PER_UPDATE, SelectionOutline, SelectionOutlineState, color_from_rgb8,
};

pub(super) fn apply_pending_outline_work(
    state: &mut SelectionOutlineState,
    commands: &mut Commands,
    owned_outlines: &Query<(), With<SelectionOutline>>,
) {
    let Some(mut work) = state.pending.take() else {
        return;
    };
    let outline = OutlineVolume {
        visible: work.key.boundary.0,
        width: super::SELECTION_OUTLINE_WIDTH,
        colour: color_from_rgb8(work.key.boundary.1),
    };
    let mut budget = MAX_PRESENTATION_ENTITIES_PER_UPDATE;

    while budget > 0 && work.removed_offset < work.removed.len() {
        let entity = work.removed[work.removed_offset];
        work.removed_offset += 1;
        budget -= 1;
        if owned_outlines.get(entity).is_ok() {
            commands
                .entity(entity)
                .remove::<(SelectionOutline, OutlineVolume, OutlineStencil)>();
        }
        if state.applied_entities.remove(&entity) {
            debug_assert!(state.applied_order.remove(entity));
        }
        state.last_removed += 1;
    }

    while budget > 0 && work.updated_offset < work.updated.len() {
        let entity = work.updated[work.updated_offset];
        work.updated_offset += 1;
        budget -= 1;
        commands.entity(entity).insert((
            SelectionOutline,
            outline.clone(),
            OutlineStencil::default(),
        ));
        if state.applied_entities.insert(entity) {
            debug_assert!(state.applied_order.insert(entity));
            state.last_added += 1;
        } else {
            state.last_updated += 1;
        }
    }

    if work.removed_offset < work.removed.len() || work.updated_offset < work.updated.len() {
        state.pending = Some(work);
        return;
    }

    if work.reconcile_all {
        while budget > 0 {
            if work.reconcile_phase == 0 {
                let Some(entity) = state.applied_order.get(work.reconcile_offset) else {
                    work.reconcile_phase = 1;
                    work.reconcile_offset = 0;
                    continue;
                };
                budget -= 1;
                if state.desired_entities.contains(&entity) {
                    commands.entity(entity).insert((
                        SelectionOutline,
                        outline.clone(),
                        OutlineStencil::default(),
                    ));
                    work.reconcile_offset += 1;
                } else {
                    if owned_outlines.get(entity).is_ok() {
                        commands
                            .entity(entity)
                            .remove::<(SelectionOutline, OutlineVolume, OutlineStencil)>();
                    }
                    let removed = state.applied_entities.remove(&entity);
                    let ordered = state.applied_order.remove(entity);
                    debug_assert_eq!(removed, ordered);
                    state.last_removed += 1;
                    // swap_remove moved a new entity into this same offset.
                }
                continue;
            }

            let Some(entity) = state.desired_order.get(work.reconcile_offset) else {
                break;
            };
            work.reconcile_offset += 1;
            budget -= 1;
            if state.desired_entities.contains(&entity) && state.applied_entities.insert(entity) {
                debug_assert!(state.applied_order.insert(entity));
                commands.entity(entity).insert((
                    SelectionOutline,
                    outline.clone(),
                    OutlineStencil::default(),
                ));
                state.last_added += 1;
            }
        }

        if work.reconcile_phase == 0 || work.reconcile_offset < state.desired_order.len() {
            state.pending = Some(work);
            return;
        }
    }

    state.last_boundary = Some(work.key.boundary);
    state.last_projection_generation = work.key.projection_generation;
    state.last_selection_revision = Some(work.key.selection_revision);
    state.last_scene_revision = work.key.scene_revision;
    state.last_coarse = Some(work.key.coarse);
}
