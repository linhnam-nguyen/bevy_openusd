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
    if work.reconcile_all {
        while budget > 0 && work.removed_offset < work.removed.len() {
            let entity = work.removed[work.removed_offset];
            work.removed_offset += 1;
            budget -= 1;
            if owned_outlines.get(entity).is_ok() {
                commands
                    .entity(entity)
                    .remove::<(SelectionOutline, OutlineVolume, OutlineStencil)>();
            }
            state.applied_entities.remove(&entity);
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
                state.applied_order.push(entity);
                state.last_added += 1;
            } else {
                state.last_updated += 1;
            }
        }
        if work.removed_offset < work.removed.len() || work.updated_offset < work.updated.len() {
            state.pending = Some(work);
            return;
        }
        while budget > 0 {
            if work.reconcile_phase == 0 {
                if work.reconcile_offset >= state.applied_order.len() {
                    work.reconcile_phase = 1;
                    work.reconcile_offset = 0;
                    continue;
                }
                let entity = state.applied_order[work.reconcile_offset];
                work.reconcile_offset += 1;
                budget -= 1;
                if state.desired_entities.contains(&entity) {
                    commands.entity(entity).insert((
                        SelectionOutline,
                        outline.clone(),
                        OutlineStencil::default(),
                    ));
                } else {
                    if owned_outlines.get(entity).is_ok() {
                        commands
                            .entity(entity)
                            .remove::<(SelectionOutline, OutlineVolume, OutlineStencil)>();
                    }
                    state.applied_entities.remove(&entity);
                    state.last_removed += 1;
                }
                continue;
            }
            if work.reconcile_offset >= state.desired_order.len() {
                break;
            }
            let entity = state.desired_order[work.reconcile_offset];
            work.reconcile_offset += 1;
            budget -= 1;
            if state.desired_entities.contains(&entity) && state.applied_entities.insert(entity) {
                state.applied_order.push(entity);
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
        state
            .applied_order
            .retain(|entity| state.applied_entities.contains(entity));
        state
            .desired_order
            .retain(|entity| state.desired_entities.contains(entity));
        state.last_boundary = Some(work.key.boundary);
        state.last_projection_generation = work.key.projection_generation;
        state.last_selection_revision = Some(work.key.selection_revision);
        state.last_scene_revision = Some(work.key.scene_revision);
        state.last_coarse = Some(work.key.coarse);
        return;
    }
    while budget > 0 && work.removed_offset < work.removed.len() {
        let entity = work.removed[work.removed_offset];
        work.removed_offset += 1;
        budget -= 1;
        if owned_outlines.get(entity).is_ok() {
            commands
                .entity(entity)
                .remove::<(SelectionOutline, OutlineVolume, OutlineStencil)>();
        }
        state.applied_entities.remove(&entity);
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
            state.applied_order.push(entity);
        }
        state.last_updated += 1;
        if work.added.contains(&entity) {
            state.last_added += 1;
        }
    }
    if work.removed_offset < work.removed.len() || work.updated_offset < work.updated.len() {
        state.pending = Some(work);
        return;
    }
    state.last_boundary = Some(work.key.boundary);
    state.last_projection_generation = work.key.projection_generation;
    state.last_selection_revision = Some(work.key.selection_revision);
    state.last_scene_revision = Some(work.key.scene_revision);
    state.last_coarse = Some(work.key.coarse);
}
