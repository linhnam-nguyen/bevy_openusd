//! Cache-backed hierarchy presentation and the immutable project lookup.

use std::collections::HashMap;
use std::path::Path;

use bevy::prelude::*;
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::super::{SceneCachePresentation, StagePresentationContext, resolve_prim_display_name};
use crate::cache::contract::{
    ProjectCacheLookup, SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheActivation,
    SceneCacheEntryKind, SceneCacheIndex, SceneCacheState,
};
use crate::cache::scene::hierarchy::{
    build_logical_hierarchy_from_cache, build_logical_hierarchy_from_stage,
    cache_has_populated_hierarchy,
};
use crate::viewport::rendering::scene::{
    ActiveHierarchyProvider, CurrentHierarchyProjection, SceneAnchorIndex, prim_name,
};

pub(super) fn publish_scene_cache_presentation(
    world: &mut World,
    activation: &SceneCacheActivation,
    project_root: Option<&Path>,
) {
    let presentation = world.get_resource::<StagePresentationContext>().cloned();
    let activation_generation = world
        .get_resource::<crate::viewport::stage::StageInfo>()
        .map_or(0, |info| info.activation_generation);
    if cache_has_populated_hierarchy(activation) {
        match build_logical_hierarchy_from_cache(
            activation,
            project_root,
            presentation.as_ref(),
        ) {
            Ok(records) => {
                install_logical_hierarchy_in_world(world, records, activation_generation);
            }
            Err(error) => {
                bevy::log::warn!(
                    "[project-cache] cached logical hierarchy invalid; deferring to canonical stage: {error:#}"
                );
                if let Some(mut index) = world.get_resource_mut::<SceneAnchorIndex>() {
                    index.mark_logical_hierarchy_unavailable(activation_generation);
                }
                if let Some(mut projection) = world.get_resource_mut::<CurrentHierarchyProjection>()
                {
                    *projection = CurrentHierarchyProjection::from_prim_nodes(&[], 0);
                }
            }
        }
    } else {
        // Legacy or unpopulated cache entries cannot prove authored names
        // before the canonical composed Stage supplies presentation metadata.
        if let Some(mut index) = world.get_resource_mut::<SceneAnchorIndex>() {
            index.mark_logical_hierarchy_unavailable(activation_generation);
        }
        if let Some(mut projection) = world.get_resource_mut::<CurrentHierarchyProjection>() {
            *projection = CurrentHierarchyProjection::from_prim_nodes(&[], 0);
        }
    }
    install_project_cache_lookup(world, activation, project_root);
    world.remove_resource::<crate::viewport::stage::VerifiedSceneVisualInputs>();
    world.insert_resource(SceneCachePresentation::from_activation(activation));
}

/// Publishes the rendering-independent logical hierarchy directly from the
/// canonical composed USD Stage before ECS entities or meshes are projected.
pub(super) fn publish_canonical_logical_hierarchy(
    world: &mut World,
    source: &openusd::usd::Stage,
    activation_generation: u64,
) {
    let presentation = world.get_resource::<StagePresentationContext>().cloned();
    match build_logical_hierarchy_from_stage(source, presentation.as_ref()) {
        Ok(records) => {
            install_logical_hierarchy_in_world(world, records, activation_generation);
        }
        Err(error) => {
            bevy::log::warn!(
                "[viewport-hierarchy] canonical logical hierarchy unavailable: {error:#}"
            );
            if let Some(mut index) = world.get_resource_mut::<SceneAnchorIndex>() {
                index.mark_logical_hierarchy_unavailable(activation_generation);
            }
            if let Some(mut current) = world.get_resource_mut::<CurrentHierarchyProjection>() {
                *current = CurrentHierarchyProjection::empty(viewport_protocol::HierarchySource::Prim, 0);
            }
        }
    }
}

fn install_logical_hierarchy_in_world(
    world: &mut World,
    records: crate::cache::scene::hierarchy::LogicalHierarchyCandidate,
    activation_generation: u64,
) {
    let prim_provider_active = world
        .get_resource::<ActiveHierarchyProvider>()
        .is_none_or(|provider| provider.source() == viewport_protocol::HierarchySource::Prim);
    let projection = if let Some(mut index) = world.get_resource_mut::<SceneAnchorIndex>() {
        index.install_logical_hierarchy(records, activation_generation)
    } else {
        let nodes: Vec<PrimNodeReadModel> = records.records
            .iter()
            .map(|record| record.to_prim_read_model(None))
            .collect();
        CurrentHierarchyProjection::from_prim_nodes(&nodes, activation_generation.max(1))
    };
    if prim_provider_active
        && let Some(mut current) = world.get_resource_mut::<CurrentHierarchyProjection>()
    {
        *current = projection;
    }
}

/// Publish cached rows only after resolving the same authoritative metadata
/// used by live projection. No persistent cache migration or geometry work is
/// required. If the source cannot validate a row, leave publication to live.
pub(super) fn publish_cached_hierarchy(
    world: &mut World,
    activation: &SceneCacheActivation,
    source: &openusd::usd::Stage,
) {
    let presentation = world.get_resource::<StagePresentationContext>().cloned();
    let projection = match build_cached_prim_nodes(activation, presentation.as_ref(), source) {
        Ok(nodes) => {
            CurrentHierarchyProjection::from_prim_nodes(&nodes, activation.descriptor.generation)
        }
        Err(error) => {
            bevy::log::warn!(
                "[project-cache] hierarchy metadata unavailable; using live hierarchy: {error:#}"
            );
            CurrentHierarchyProjection::from_prim_nodes(&[], 0)
        }
    };
    if let Some(mut current) = world.get_resource_mut::<CurrentHierarchyProjection>() {
        *current = projection;
    }
}

fn build_cached_prim_nodes(
    activation: &SceneCacheActivation,
    presentation: Option<&StagePresentationContext>,
    source: &openusd::usd::Stage,
) -> anyhow::Result<Vec<PrimNodeReadModel>> {
    let mut entry_paths: Vec<Option<String>> = Vec::with_capacity(activation.index.entries.len());
    for entry in &activation.index.entries {
        let path = match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { prim_path } => Some(prim_path.clone()),
            SceneCacheEntryKind::ChildScene { member_id, .. }
            | SceneCacheEntryKind::ChildModel { member_id, .. } => {
                Some(crate::project::scene::authoring::scene_member_path(*member_id))
            }
        };
        entry_paths.push(path);
    }

    let mut path_to_entry: HashMap<&str, usize> = HashMap::with_capacity(entry_paths.len());
    for (index, path) in entry_paths.iter().enumerate() {
        if let Some(path) = path.as_deref() {
            path_to_entry.insert(path, index);
        }
    }

    let mut has_children = vec![false; activation.index.entries.len()];
    let mut owned_prim_count = 0usize;
    let mut has_member_rows = false;
    for (index, entry) in activation.index.entries.iter().enumerate() {
        match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { .. } => {
                owned_prim_count += 1;
            }
            SceneCacheEntryKind::ChildScene { .. } | SceneCacheEntryKind::ChildModel { .. } => {
                has_member_rows = true;
                // A parent Scene cache does not expand child Scene/Model
                // internals; mark unknown child state as expandable rather than
                // falsely claiming the member is a leaf.
                has_children[index] = true;
            }
        }
        if let Some(parent_index) = resolve_entry_parent_index(
            index,
            entry.parent,
            entry_paths.len(),
            entry_paths[index].as_deref(),
            &path_to_entry,
        ) && let Some(flag) = has_children.get_mut(parent_index)
        {
            *flag = true;
        }
    }

    let cache_incomplete = activation.descriptor.state != SceneCacheState::Ready
        || (activation.descriptor.prim_count as usize > owned_prim_count && !has_member_rows);

    let mut nodes = Vec::with_capacity(activation.index.entries.len());
    for (index, entry) in activation.index.entries.iter().enumerate() {
        let Some(prim_path) = entry_paths[index].as_deref() else {
            continue;
        };
        let parent = resolve_entry_parent_index(
            index,
            entry.parent,
            entry_paths.len(),
            Some(prim_path),
            &path_to_entry,
        )
        .and_then(|parent_index| entry_paths.get(parent_index).and_then(|path| path.as_deref()))
        .map(|parent_path| SceneAnchor::active_session(parent_path.to_owned()));
        let metadata = usd_bevy::route::geom::read_authored_hierarchy_metadata(source, prim_path)?;
        let expected_target = match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { .. } => None,
            SceneCacheEntryKind::ChildScene { scene_id, .. } => {
                Some(("scene", scene_id.to_string()))
            }
            SceneCacheEntryKind::ChildModel { model_id, .. } => {
                Some(("model", model_id.to_string()))
            }
        };
        // A member row must describe the same manifest target as the composed
        // prim. Do not invent a cache-only override if canonical metadata is
        // missing or points at a different target.
        if let Some((kind, id)) = expected_target.as_ref() {
            anyhow::ensure!(
                metadata.target.as_ref().is_some_and(|target| {
                    target.kind.as_str() == *kind && target.id.as_str() == id.as_str()
                }),
                "cached hierarchy member {prim_path} disagrees with the composed Stage target"
            );
        }
        let is_member = expected_target.is_some();
        let display_name = resolve_prim_display_name(
            presentation,
            prim_path,
            metadata.display_name.as_deref(),
            metadata
                .target
                .as_ref()
                .map(|target| (target.kind.as_str(), target.id.as_str())),
        );
        let is_presentation_root = presentation
            .and_then(|context| context.root_path.as_deref())
            .is_some_and(|root_path| root_path == prim_path);
        let expandable = has_children[index]
            || is_member
            || (cache_incomplete && (parent.is_none() || is_presentation_root));
        nodes.push(PrimNodeReadModel {
            anchor: SceneAnchor::active_session(prim_path.to_owned()),
            parent,
            label: prim_name(prim_path).to_owned(),
            display_name,
            visible: true,
            has_children: expandable,
        });
    }
    Ok(nodes)
}

fn resolve_entry_parent_index(
    entry_index: usize,
    explicit_parent: Option<u32>,
    entry_count: usize,
    prim_path: Option<&str>,
    path_to_entry: &HashMap<&str, usize>,
) -> Option<usize> {
    if let Some(parent) = explicit_parent
        .map(|parent| parent as usize)
        .filter(|parent| *parent < entry_count && *parent != entry_index)
    {
        return Some(parent);
    }
    let parent_path = prim_path?.rsplit_once('/').map(|(parent, _)| parent)?;
    if parent_path.is_empty() {
        return None;
    }
    path_to_entry
        .get(parent_path)
        .copied()
        .filter(|parent| *parent < entry_count && *parent != entry_index)
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
                crate::cache::warming::runtime::load_published_project_cache_lookup(root)
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
#[path = "../../../../tests/unit/viewport/stage/lifecycle/scene_presentation_tests.rs"]
mod tests;
