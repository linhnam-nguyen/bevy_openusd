//! Cache-backed hierarchy presentation and the immutable project lookup.

use bevy::prelude::*;
use std::path::Path;
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use crate::project::cache_contract::{
    ProjectCacheLookup, SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheActivation, SceneCacheEntryKind,
    SceneCacheIndex,
};
use crate::viewport::api::CurrentHierarchyProjection;
use super::SceneCachePresentation;

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
mod tests {
    use super::*;
    use crate::project::cache_contract::{
        CachedTransform, SceneCacheAddress, SceneCacheBlobRef, SceneCacheEntry,
        SceneCacheOccurrence, SCENE_CACHE_INDEX_SCHEMA_VERSION,
    };
    use usd_project::ScenePlacementTransform;

    #[test]
    fn child_flags_are_computed_in_one_parent_index_pass() {
        let mut app = App::new();
        app.init_resource::<CurrentHierarchyProjection>();
        let scene_id = usd_project::SceneId::new_v4();
        let root = SceneCacheEntry {
            address: SceneCacheAddress {
                scene_id,
                occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Root".into()),
            },
            parent: None,
            transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
            bounds: None,
            cacheable: false,
            bim_enabled: false,
            geometry: None,
            material: None,
            animation: None,
            semantic_key: None,
            kind: SceneCacheEntryKind::OwnedPrim {
                prim_path: "/SceneRoot/Root".into(),
            },
            content_hash: None,
        };
        let child = SceneCacheEntry {
            address: SceneCacheAddress {
                scene_id,
                occurrence: SceneCacheOccurrence::PrimPath("/SceneRoot/Root/Child".into()),
            },
            parent: Some(0),
            transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
            bounds: None,
            cacheable: true,
            bim_enabled: false,
            geometry: Some(SceneCacheBlobRef {
                blob_id: usd_model::BlobId("a".repeat(64)),
                byte_size: 1,
            }),
            material: None,
            animation: None,
            semantic_key: None,
            kind: SceneCacheEntryKind::OwnedPrim {
                prim_path: "/SceneRoot/Root/Child".into(),
            },
            content_hash: Some(usd_model::HashDigest::from_hex(&"a".repeat(64)).unwrap()),
        };
        let mut descriptor = crate::project::cache::SceneCacheDescriptorV3::invalidated(
            scene_id,
            1,
            usd_model::HashDigest::from_hex(&"b".repeat(64)).unwrap(),
        );
        descriptor.state = crate::project::cache_contract::SceneCacheState::Ready;
        descriptor.prim_count = 2;
        descriptor.cacheable_count = 1;
        let activation = SceneCacheActivation {
            descriptor,
            index: crate::project::cache_contract::SceneCacheIndex {
                schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
                scene_id,
                generation: 1,
                entries: vec![root, child],
            },
        };
        publish_scene_cache_presentation(&mut app.world_mut(), &activation, None);
        let projection = app.world().resource::<CurrentHierarchyProjection>();
        assert!(projection
            .snapshot()
            .nodes
            .iter()
            .any(|node| node.has_children));
    }
}
