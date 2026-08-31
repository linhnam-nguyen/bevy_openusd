use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdHierarchyTarget, UsdPrimRef, UsdTransparentHierarchyNode};

use crate::viewport::session::Spawned;
use crate::viewport::session::StagePresentationContext;

use super::SceneAnchorIndex;

/// Rebuilds only after stage entities or tree-visible data changes. This keeps
/// the protocol boundary from traversing a large scene every frame.
pub(crate) fn refresh_scene_anchor_index(
    spawned: Res<Spawned>,
    changed_prims: Query<
        Entity,
        (
            With<UsdPrimRef>,
            Or<(
                Added<UsdPrimRef>,
                Changed<UsdPrimRef>,
                Changed<UsdDisplayName>,
                Added<UsdHierarchyTarget>,
                Changed<UsdHierarchyTarget>,
                Added<UsdTransparentHierarchyNode>,
                Changed<UsdTransparentHierarchyNode>,
                Changed<Visibility>,
                Changed<Children>,
            )>,
        ),
    >,
    prims: Query<(
        Entity,
        &UsdPrimRef,
        Option<&UsdDisplayName>,
        Option<&UsdHierarchyTarget>,
        Option<&UsdTransparentHierarchyNode>,
        Option<&Visibility>,
        Option<&Children>,
    )>,
    mut removed_prims: RemovedComponents<UsdPrimRef>,
    mut removed_transparent: RemovedComponents<UsdTransparentHierarchyNode>,
    mut index: ResMut<SceneAnchorIndex>,
    presentation: Option<Res<StagePresentationContext>>,
) {
    // ScenePatch materialization can happen across a frame boundary after
    // Spawned flips to true. Treat that lifecycle transition as a rebuild
    // trigger as well; otherwise a static stage can publish an empty tree
    // before its projected prim entities are visible to this query.
    let changed = spawned.is_changed()
        || !changed_prims.is_empty()
        || removed_prims.read().next().is_some()
        || removed_transparent.read().next().is_some();
    if !index.initialized && prims.is_empty() {
        index.initialized = true;
        return;
    }
    if changed || !index.initialized {
        index.rebuild(&prims, presentation.as_deref());
        let root_count = index
            .nodes
            .iter()
            .filter(|node| node.parent.is_none())
            .count();
        info!(
            "[viewport-scene-index] rebuilt revision={} prims={} roots={}",
            index.revision,
            index.nodes.len(),
            root_count
        );
    }
}
