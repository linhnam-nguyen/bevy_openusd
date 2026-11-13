//! Cache-backed hierarchy presentation and the immutable project lookup.

use bevy::prelude::*;
use std::path::Path;
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::SceneCachePresentation;
use crate::project::cache_contract::{
    ProjectCacheLookup, SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheActivation,
    SceneCacheEntryKind, SceneCacheIndex,
};
use crate::viewport::api::CurrentHierarchyProjection;

pub(super) fn publish_scene_cache_presentation(
    world: &mut World,
    activation: &SceneCacheActivation,
    project_root: Option<&Path>,
) {
    let mut child_indexes = vec![false; activation.index.entries.len()];
    for entry in &activation.index.entries {
        if let Some(parent) = entry.parent
            && let Some(has_children) = child_indexes.get_mut(parent as usize)
        {
            *has_children = true;
        }
    }
    let mut nodes = Vec::new();
    for (index, entry) in activation.index.entries.iter().enumerate() {
        let SceneCacheEntryKind::OwnedPrim { prim_path } = &entry.kind else {
            continue;
        };
        let anchor = SceneAnchor::active_session(prim_path.clone());
        let parent = entry.parent.and_then(|parent| {
            activation
                .index
                .entries
                .get(parent as usize)
                .and_then(|entry| match &entry.kind {
                    SceneCacheEntryKind::OwnedPrim { prim_path } => {
                        Some(SceneAnchor::active_session(prim_path.clone()))
                    }
                    _ => None,
                })
        });
        let label = prim_path
            .rsplit('/')
            .find(|segment| !segment.is_empty())
            .unwrap_or(prim_path)
            .to_owned();
        nodes.push(PrimNodeReadModel {
            anchor,
            parent,
            label,
            display_name: entry.semantic_key.clone(),
            visible: true,
            has_children: child_indexes[index],
        });
    }
    if let Some(mut projection) = world.get_resource_mut::<CurrentHierarchyProjection>() {
        *projection =
            CurrentHierarchyProjection::from_prim_nodes(&nodes, activation.descriptor.generation);
    }
    install_project_cache_lookup(world, activation, project_root);
    world.insert_resource(SceneCachePresentation::from_activation(activation));
}

fn install_project_cache_lookup(
    world: &mut World,
    activation: &SceneCacheActivation,
    project_root: Option<&Path>,
) {
    let mut lookup = world
        .remove_resource::<ProjectCacheLookup>()
        .or_else(|| {
            project_root.and_then(|root| {
                crate::project::cache_warm_runtime::load_published_project_cache_lookup(root)
                    .ok()
                    .flatten()
            })
        })
        .unwrap_or_default();
    let active = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id: activation.descriptor.scene_id,
        generation: activation.descriptor.generation,
        entries: activation.index.entries.clone(),
    };
    if let Err(error) = lookup.replace_scene_index(&active) {
        bevy::log::warn!(
            "[project-cache] could not install active Scene lookup for {}: {error:#}",
            activation.descriptor.scene_id
        );
    }
    world.insert_resource(lookup);
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/session/lifecycle_scene_presentation_inline_tests.rs"]
mod tests;
