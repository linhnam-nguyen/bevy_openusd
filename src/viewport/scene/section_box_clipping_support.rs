use std::collections::HashSet;

use bevy::asset::Assets;
use bevy::ecs::hierarchy::Children;
use bevy::pbr::{ExtendedMaterial, StandardMaterial};
use bevy::prelude::*;
use viewport_protocol::{RenderMode, SceneAnchor};

use super::super::selection_color::SelectionBaseMaterial;
use super::super::selection_color::SelectionColorOverride;
use super::super::selection_outline::collect_mesh_descendants;
use super::super::visualization::OriginalRenderMaterial;
use crate::viewport::api::SceneAnchorIndex;

use super::{SectionClipExtension, SectionClipMaterial, SectionClipUniform};

pub(super) fn selected_meshes(
    targets: &[SceneAnchor],
    scene_index: &SceneAnchorIndex,
    mesh_hierarchy: &Query<(Option<&Mesh3d>, Option<&Children>)>,
) -> HashSet<Entity> {
    let mut selected = HashSet::new();
    for target in targets {
        let Some(root) = scene_index.resolve(target) else {
            continue;
        };
        collect_mesh_descendants(root, mesh_hierarchy, &mut selected);
    }
    selected
}

#[derive(Debug, Clone)]
pub(super) struct ComposedClipRoute {
    pub(super) route: Handle<StandardMaterial>,
    pub(super) original: Option<Handle<StandardMaterial>>,
    pub(super) selection_base: Option<Handle<StandardMaterial>>,
    pub(super) selection_override: bool,
}

impl ComposedClipRoute {
    pub(super) fn from_current(route: Handle<StandardMaterial>) -> Self {
        Self {
            route,
            original: None,
            selection_base: None,
            selection_override: false,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compose_clipped_route(
    current_route: Handle<StandardMaterial>,
    original: Option<&OriginalRenderMaterial>,
    selection_base: Option<&SelectionBaseMaterial>,
    selection_override: bool,
    selection_enabled: bool,
    selection_handle: Option<&Handle<StandardMaterial>>,
    selected: bool,
    hover_enabled: bool,
    hover_handle: Option<&Handle<StandardMaterial>>,
    hovered: bool,
    render_mode: RenderMode,
    uniform_handle: Option<&Handle<StandardMaterial>>,
) -> ComposedClipRoute {
    let mut route = current_route;
    let mut original = original.map(|material| material.0.clone());
    let mut selection_base = selection_base.map(|material| material.0.clone());
    let mut selection_override = selection_override;

    match render_mode {
        RenderMode::UniformColor => {
            if let Some(uniform) = uniform_handle {
                if selection_override {
                    if original.is_none() {
                        original = Some(selection_base.clone().unwrap_or_else(|| route.clone()));
                    }
                    selection_base = Some(uniform.clone());
                } else if route != *uniform {
                    original = Some(route.clone());
                    route = uniform.clone();
                }
            }
        }
        RenderMode::Shaded | RenderMode::Wireframe | RenderMode::RayTraced => {
            if let Some(original_route) = original.clone() {
                if selection_override {
                    selection_base = Some(original_route);
                } else {
                    route = original_route;
                    original = None;
                }
            }
        }
    }

    let desired_owner = if selection_enabled && selected {
        selection_handle.cloned()
    } else if hover_enabled && hovered {
        hover_handle.cloned()
    } else {
        None
    };
    if let Some(owner) = desired_owner {
        if !selection_override || selection_base.is_none() {
            selection_base = Some(route.clone());
            selection_override = true;
        } else if selection_handle.is_none_or(|handle| route != *handle)
            && hover_handle.is_none_or(|handle| route != *handle)
            && selection_base.as_ref() != Some(&route)
        {
            selection_base = Some(route.clone());
        }
        route = owner;
    } else if selection_override {
        if let Some(base) = selection_base.take() {
            route = base;
        }
        selection_override = false;
        if render_mode != RenderMode::UniformColor && original.as_ref() == Some(&route) {
            original = None;
        }
    }

    ComposedClipRoute {
        route,
        original,
        selection_base,
        selection_override,
    }
}

pub(super) fn ensure_clip_material(
    base_handle: &Handle<StandardMaterial>,
    standard_materials: &Assets<StandardMaterial>,
    clip_materials: &mut Assets<SectionClipMaterial>,
    material_cache: &mut std::collections::HashMap<
        AssetId<StandardMaterial>,
        Handle<SectionClipMaterial>,
    >,
    uniform: SectionClipUniform,
) -> Option<Handle<SectionClipMaterial>> {
    let base = standard_materials.get(base_handle)?.clone();
    let base_id = base_handle.id();
    if let Some(handle) = material_cache.get(&base_id).cloned()
        && let Some(mut material) = clip_materials.get_mut(&handle)
    {
        material.base = base;
        material.extension.clip = uniform;
        return Some(handle);
    }

    let handle = clip_materials.add(ExtendedMaterial {
        base,
        extension: SectionClipExtension { clip: uniform },
    });
    material_cache.insert(base_id, handle.clone());
    Some(handle)
}

pub(super) fn apply_composed_route(
    commands: &mut Commands,
    entity: Entity,
    composed: &ComposedClipRoute,
    original: Option<&OriginalRenderMaterial>,
    selection_base: Option<&SelectionBaseMaterial>,
    selection_override: bool,
) {
    if composed.original.as_ref() != original.map(|material| &material.0) {
        if let Some(original) = &composed.original {
            commands
                .entity(entity)
                .insert(OriginalRenderMaterial(original.clone()));
        } else {
            commands.entity(entity).remove::<OriginalRenderMaterial>();
        }
    }
    if composed.selection_override {
        if !selection_override
            || selection_base.map(|base| &base.0) != composed.selection_base.as_ref()
        {
            commands.entity(entity).insert((
                SelectionColorOverride,
                SelectionBaseMaterial(
                    composed
                        .selection_base
                        .clone()
                        .expect("selection override always has a base route"),
                ),
            ));
        }
    } else if selection_override || selection_base.is_some() {
        commands
            .entity(entity)
            .remove::<(SelectionColorOverride, SelectionBaseMaterial)>();
    }
}

pub(super) fn prune_material_cache(
    cache: &mut std::collections::HashMap<AssetId<StandardMaterial>, Handle<SectionClipMaterial>>,
    clip_materials: &mut Assets<SectionClipMaterial>,
    used_base_ids: &HashSet<AssetId<StandardMaterial>>,
    in_use_clip_ids: &HashSet<AssetId<SectionClipMaterial>>,
) {
    let stale = cache
        .iter()
        .filter_map(|(base_id, handle)| {
            (!used_base_ids.contains(base_id) && !in_use_clip_ids.contains(&handle.id()))
                .then_some((*base_id, handle.id()))
        })
        .collect::<Vec<_>>();
    for (base_id, clip_id) in stale {
        cache.remove(&base_id);
        clip_materials.remove(clip_id);
    }
}
