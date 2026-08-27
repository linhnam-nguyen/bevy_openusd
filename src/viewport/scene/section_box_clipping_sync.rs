use std::collections::HashSet;

use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::{Commands, Res, Vec3};

use super::super::section_box::SectionBoxState;
use super::super::selection_hover::HoveredTarget;
use super::super::visualization::DisplayToggles;
use super::support::{
    apply_composed_route, clear_recovered_clip_diagnostics, compose_clipped_route,
    ensure_clip_material, prune_material_cache, selected_meshes,
};
use super::{
    SectionClipMaterial, SectionClipPresentationKey, SectionClipSystemParam,
    SectionClipUnderlyingMaterial, SectionClipUniform,
};
use crate::viewport::api::{SceneAnchorIndex, ViewerSettingsState};

#[allow(clippy::type_complexity)]
pub(in crate::viewport) fn sync_section_box_clipping(
    state: Res<SectionBoxState>,
    settings: Res<ViewerSettingsState>,
    toggles: Res<DisplayToggles>,
    hovered_target: Res<HoveredTarget>,
    scene_index: Res<SceneAnchorIndex>,
    mut commands: Commands,
    params: SectionClipSystemParam,
) {
    let SectionClipSystemParam {
        selection_material,
        hover_material,
        uniform_material,
        mut projection,
        selected_projection,
        mut diagnostics,
        standard_materials,
        mut clip_materials,
        mesh_hierarchy,
        renderables,
        changed_clipped,
        mut standard_material_events,
    } = params;
    let selection = settings.selection();
    let presentation = SectionClipPresentationKey {
        render_mode: toggles.renderer.render_mode,
        selection_color_enabled: selection.color_change_enabled,
        selection_color: selection.selection_color,
        hover_color_enabled: selection.hover_color_change_enabled,
        hover_color: selection.hover_color,
        hovered_anchor: hovered_target.anchor.clone(),
    };
    let state_revision_changed = projection.last_state_revision != Some(state.revision);
    let scene_revision_changed = projection.last_scene_revision != Some(scene_index.revision());
    let selected_projection_generation = selected_projection
        .as_ref()
        .map(|projection| projection.generation());
    let selected_projection_changed =
        projection.last_projection_generation != selected_projection_generation;
    let selection_set_changed = projection.last_targets.as_ref() != Some(&state.targets)
        || scene_revision_changed
        || selected_projection_changed;
    let hover_set_changed = projection.last_hovered_anchor.as_ref()
        != hovered_target.anchor.as_ref()
        || projection.last_hover_enabled != Some(selection.hover_color_change_enabled)
        || scene_revision_changed;
    let presentation_changed = projection.last_presentation.as_ref() != Some(&presentation);
    let clipped_route_changed = !changed_clipped.is_empty();
    let material_asset_changed = standard_material_events
        .as_mut()
        .is_some_and(|events| events.read().next().is_some());

    if !state_revision_changed
        && !scene_revision_changed
        && !presentation_changed
        && !clipped_route_changed
        && !material_asset_changed
    {
        return;
    }

    if selection_set_changed {
        projection.selected_meshes = selected_projection.as_ref().map_or_else(
            || selected_meshes(&state.targets, &scene_index, &mesh_hierarchy),
            |selected_projection| selected_projection.renderables().clone(),
        );
    }
    if hover_set_changed {
        projection.hovered_meshes = if selection.hover_color_change_enabled {
            hovered_target
                .anchor
                .as_ref()
                .map(|target| {
                    selected_meshes(std::slice::from_ref(target), &scene_index, &mesh_hierarchy)
                })
                .unwrap_or_default()
        } else {
            HashSet::new()
        };
    }

    let desired = if state.enabled && state.visible {
        projection.selected_meshes.clone()
    } else {
        HashSet::new()
    };
    diagnostics
        .unsupported_entities
        .retain(|entity| desired.contains(entity));
    diagnostics
        .missing_material_entities
        .retain(|entity| desired.contains(entity));

    let uniform = SectionClipUniform {
        world_to_box: state.transform.to_matrix().inverse(),
        enabled: 1,
        _padding: Vec3::ZERO,
    };
    let selection_handle = selection_material.as_ref().map(|material| &material.0);
    let hover_handle = hover_material.as_ref().map(|material| &material.0);
    let uniform_handle = uniform_material.as_ref().map(|material| &material.0);
    let mut deferred_clip_ids = HashSet::new();
    let stale_entities = projection
        .active_entities
        .difference(&desired)
        .copied()
        .collect::<Vec<_>>();
    for entity in stale_entities {
        if let Ok((
            _,
            _,
            Some(clip),
            Some(underlying),
            original,
            selection_base,
            selection_override,
        )) = renderables.get(entity)
        {
            deferred_clip_ids.insert(clip.0.id());
            let composed = compose_clipped_route(
                underlying.0.clone(),
                original,
                selection_base,
                selection_override.is_some(),
                selection.color_change_enabled,
                selection_handle,
                projection.selected_meshes.contains(&entity),
                selection.hover_color_change_enabled,
                hover_handle,
                projection.hovered_meshes.contains(&entity),
                toggles.renderer.render_mode,
                uniform_handle,
            );
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<SectionClipMaterial>>()
                .insert(MeshMaterial3d(composed.route.clone()))
                .remove::<SectionClipUnderlyingMaterial>();
            apply_composed_route(
                &mut commands,
                entity,
                &composed,
                original,
                selection_base,
                selection_override.is_some(),
            );
        }
        projection.active_entities.remove(&entity);
    }

    let mut next_active = HashSet::new();
    let mut used_base_ids = HashSet::new();

    for entity in &desired {
        let Ok((_, standard, clip, underlying, original, selection_base, selection_override)) =
            renderables.get(*entity)
        else {
            continue;
        };
        let Some(current_route) = underlying
            .map(|material| material.0.clone())
            .or_else(|| standard.map(|material| material.0.clone()))
        else {
            if diagnostics.unsupported_entities.insert(*entity) {
                bevy::log::warn!(
                    "[section-box] selected renderable {entity:?} has no StandardMaterial route; clipping skipped"
                );
            }
            continue;
        };

        let composed = compose_clipped_route(
            current_route.clone(),
            original,
            selection_base,
            selection_override.is_some(),
            selection.color_change_enabled,
            selection_handle,
            projection.selected_meshes.contains(entity),
            selection.hover_color_change_enabled,
            hover_handle,
            projection.hovered_meshes.contains(entity),
            toggles.renderer.render_mode,
            uniform_handle,
        );
        let Some(clip_handle) = ensure_clip_material(
            &composed.route,
            &standard_materials,
            &mut clip_materials,
            &mut projection.material_cache,
            uniform,
        ) else {
            if clip.is_some() {
                next_active.insert(*entity);
            } else if diagnostics.missing_material_entities.insert(*entity) {
                bevy::log::warn!(
                    "[section-box] selected renderable {entity:?} references a missing StandardMaterial; clipping skipped"
                );
            }
            continue;
        };

        clear_recovered_clip_diagnostics(&mut diagnostics, *entity);
        used_base_ids.insert(composed.route.id());
        commands
            .entity(*entity)
            .insert((
                MeshMaterial3d(clip_handle),
                SectionClipUnderlyingMaterial(composed.route.clone()),
            ))
            .remove::<MeshMaterial3d<StandardMaterial>>();
        apply_composed_route(
            &mut commands,
            *entity,
            &composed,
            original,
            selection_base,
            selection_override.is_some(),
        );
        next_active.insert(*entity);
    }

    projection.active_entities = next_active;
    projection.last_targets = Some(state.targets.clone());
    projection.last_hovered_anchor = hovered_target.anchor.clone();
    projection.last_hover_enabled = Some(selection.hover_color_change_enabled);
    projection.last_state_revision = Some(state.revision);
    projection.last_scene_revision = Some(scene_index.revision());
    projection.last_projection_generation = selected_projection_generation;
    projection.last_presentation = Some(presentation);

    let mut in_use_clip_ids = deferred_clip_ids;
    for entity in &projection.active_entities {
        if let Ok((_, _, Some(clip), ..)) = renderables.get(*entity) {
            in_use_clip_ids.insert(clip.0.id());
        }
    }
    prune_material_cache(
        &mut projection.material_cache,
        &mut clip_materials,
        &used_base_ids,
        &in_use_clip_ids,
    );
}
