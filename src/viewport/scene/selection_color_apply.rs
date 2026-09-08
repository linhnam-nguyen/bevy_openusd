use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

use super::{
    MAX_PRESENTATION_ENTITIES_PER_UPDATE, PresentationOwner, SelectionBaseMaterial,
    SelectionColorOverride, SelectionColorOverrideState, presentation_owner,
};

pub(super) fn apply_pending_color_work(
    state: &mut SelectionColorOverrideState,
    commands: &mut Commands,
    meshes: &mut Query<(
        Entity,
        &mut MeshMaterial3d<StandardMaterial>,
        Option<&SelectionBaseMaterial>,
        Option<&SelectionColorOverride>,
    )>,
    selection_handle: &Handle<StandardMaterial>,
    hover_handle: &Handle<StandardMaterial>,
) {
    let Some(mut work) = state.pending.take() else {
        return;
    };

    if work.reconcile_all {
        let mut budget = MAX_PRESENTATION_ENTITIES_PER_UPDATE;
        while budget > 0 && work.offset < work.affected.len() {
            let entity = work.affected[work.offset];
            work.offset += 1;
            budget -= 1;
            apply_color_entity(
                state,
                entity,
                commands,
                meshes,
                selection_handle,
                hover_handle,
            );
        }
        if work.offset < work.affected.len() {
            state.pending = Some(work);
            return;
        }

        while budget > 0 {
            let entity = match work.reconcile_phase {
                0 => state.applied_order.get(work.reconcile_offset),
                1 => state.selected_order.get(work.reconcile_offset),
                _ => state.hovered_order.get(work.reconcile_offset),
            };
            let Some(entity) = entity else {
                if work.reconcile_phase >= 2 {
                    break;
                }
                work.reconcile_phase += 1;
                work.reconcile_offset = 0;
                continue;
            };

            budget -= 1;
            let phase = work.reconcile_phase;
            apply_color_entity(
                state,
                entity,
                commands,
                meshes,
                selection_handle,
                hover_handle,
            );
            if phase == 0 && !state.applied_owners.contains_key(&entity) {
                // O(1) swap_remove moved the next live entry into this offset.
            } else {
                work.reconcile_offset += 1;
            }
        }

        if work.reconcile_phase < 2
            || work.reconcile_offset
                < match work.reconcile_phase {
                    0 => state.applied_order.len(),
                    1 => state.selected_order.len(),
                    _ => state.hovered_order.len(),
                }
        {
            state.pending = Some(work);
            return;
        }

        state.last_selection_revision = Some(work.key.selection_revision);
        state.last_scene_revision = work.key.scene_revision;
        state.last_projection_generation = work.key.projection_generation;
        state.last_presentation = Some(work.key.presentation);
        return;
    }

    let start = work.offset;
    let end = (start + MAX_PRESENTATION_ENTITIES_PER_UPDATE).min(work.affected.len());
    for entity in &work.affected[start..end] {
        let entity = *entity;
        let Ok((_, material, base, marker)) = meshes.get_mut(entity) else {
            if state.applied_owners.remove(&entity).is_some() {
                debug_assert!(state.applied_order.remove(entity));
            }
            continue;
        };
        apply_color_entity_with_parts(
            state,
            entity,
            commands,
            material,
            base,
            marker,
            selection_handle,
            hover_handle,
        );
    }
    work.offset = end;
    state.last_affected_entities = end - start;
    if work.offset < work.affected.len() {
        state.pending = Some(work);
        return;
    }
    state.last_selection_revision = Some(work.key.selection_revision);
    state.last_scene_revision = work.key.scene_revision;
    state.last_projection_generation = work.key.projection_generation;
    state.last_presentation = Some(work.key.presentation);
}

fn apply_color_entity(
    state: &mut SelectionColorOverrideState,
    entity: Entity,
    commands: &mut Commands,
    meshes: &mut Query<(
        Entity,
        &mut MeshMaterial3d<StandardMaterial>,
        Option<&SelectionBaseMaterial>,
        Option<&SelectionColorOverride>,
    )>,
    selection_handle: &Handle<StandardMaterial>,
    hover_handle: &Handle<StandardMaterial>,
) {
    let Ok((_, material, base, marker)) = meshes.get_mut(entity) else {
        if state.applied_owners.remove(&entity).is_some() {
            debug_assert!(state.applied_order.remove(entity));
        }
        return;
    };
    apply_color_entity_with_parts(
        state,
        entity,
        commands,
        material,
        base,
        marker,
        selection_handle,
        hover_handle,
    );
}

fn apply_color_entity_with_parts(
    state: &mut SelectionColorOverrideState,
    entity: Entity,
    commands: &mut Commands,
    mut material: Mut<MeshMaterial3d<StandardMaterial>>,
    base: Option<&SelectionBaseMaterial>,
    marker: Option<&SelectionColorOverride>,
    selection_handle: &Handle<StandardMaterial>,
    hover_handle: &Handle<StandardMaterial>,
) {
    let desired_owner = presentation_owner(
        state.selected_meshes.contains(&entity),
        state.hovered_meshes.contains(&entity),
    );
    if let Some(desired_owner) = desired_owner {
        let desired_handle = match desired_owner {
            PresentationOwner::Selection => selection_handle,
            PresentationOwner::Hover => hover_handle,
        };
        if let (Some(base), Some(_marker)) = (base, marker)
            && material.0 != *selection_handle
            && material.0 != *hover_handle
            && material.0 != base.0
        {
            commands
                .entity(entity)
                .insert(SelectionBaseMaterial(material.0.clone()));
        }
        if marker.is_none() || base.is_none() {
            commands.entity(entity).insert((
                SelectionColorOverride,
                SelectionBaseMaterial(material.0.clone()),
            ));
        }
        if material.0 != *desired_handle {
            material.0 = desired_handle.clone();
        }
        if state.applied_owners.insert(entity, desired_owner).is_none() {
            debug_assert!(state.applied_order.insert(entity));
        }
    } else if let (Some(base), Some(_marker)) = (base, marker) {
        material.0 = base.0.clone();
        commands
            .entity(entity)
            .remove::<(SelectionColorOverride, SelectionBaseMaterial)>();
        if state.applied_owners.remove(&entity).is_some() {
            debug_assert!(state.applied_order.remove(entity));
        }
    } else if state.applied_owners.remove(&entity).is_some() {
        debug_assert!(state.applied_order.remove(entity));
    }
}
