//! Session-local mapping between logical scene anchors and Bevy entities.
//!
//! The map is the only place where a product-facing prim identity meets an
//! ECS entity. It never leaves the viewport process.

use std::collections::HashMap;
use std::ops::Range;

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdPrimRef};
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

#[cfg(test)]
use viewport_protocol::MAX_SCENE_PAGE_SIZE;

use super::hierarchy::CurrentHierarchyProjection;
use super::scene_occurrence_index::SceneOccurrenceIndex;
use crate::viewport::session::Spawned;

#[path = "scene_index_dense.rs"]
mod dense;
#[path = "scene_index_hierarchy.rs"]
mod hierarchy;
#[path = "scene_index_lookup.rs"]
mod lookup;
#[path = "scene_index_rebuild.rs"]
mod rebuild;

/// Returns the current prim-tree node name for a prim path.
pub(crate) fn prim_name(path: &str) -> &str {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
}

/// Logical tree and private entity mapping for the active stage.
#[derive(Resource, Default)]
pub(crate) struct SceneAnchorIndex {
    by_anchor: HashMap<SceneAnchor, Entity>,
    by_entity: HashMap<Entity, SceneAnchor>,
    occurrence_index: SceneOccurrenceIndex,
    nodes: Vec<PrimNodeReadModel>,
    dense: DenseSceneIndex,
    initialized: bool,
    revision: u64,
}

/// Dense, session-local scene topology. Protocol strings remain cold fields on
/// each node, while parent/child and occurrence queries use integer ranges.
/// The structure is rebuilt only when projected scene rows change.
#[derive(Clone, Debug, Default)]
pub(super) struct DenseSceneIndex {
    nodes: Vec<DenseSceneNode>,
    by_anchor: HashMap<SceneAnchor, usize>,
    by_entity: HashMap<Entity, usize>,
    by_path: HashMap<String, Vec<usize>>,
    child_ranges: Vec<Range<usize>>,
    child_order: Vec<usize>,
}

#[derive(Clone, Debug)]
struct DenseSceneNode {
    entity: Option<Entity>,
    anchor: SceneAnchor,
    parent: Option<usize>,
    first_child: usize,
    child_count: usize,
    sibling_index: usize,
    label: String,
    display_name: Option<String>,
    visible: bool,
    has_children: bool,
}

#[cfg(test)]
impl SceneAnchorIndex {
    pub(crate) fn from_test_nodes(nodes: Vec<PrimNodeReadModel>) -> Self {
        let dense = DenseSceneIndex::from_nodes(&nodes, &HashMap::new());
        Self {
            nodes,
            dense,
            initialized: true,
            revision: 1,
            ..Default::default()
        }
    }

    pub(crate) fn from_test_entity(anchor: SceneAnchor, entity: Entity) -> Self {
        let mut occurrence_index = SceneOccurrenceIndex::default();
        occurrence_index.insert(&anchor.prim_path, entity);
        Self {
            by_anchor: HashMap::from([(anchor.clone(), entity)]),
            by_entity: HashMap::from([(entity, anchor)]),
            occurrence_index,
            initialized: true,
            revision: 1,
            ..Default::default()
        }
    }

    pub(crate) fn from_test_entities(entries: Vec<(SceneAnchor, Entity)>) -> Self {
        let mut occurrence_index = SceneOccurrenceIndex::default();
        for (anchor, entity) in &entries {
            occurrence_index.insert(&anchor.prim_path, *entity);
        }
        let by_anchor = entries
            .iter()
            .cloned()
            .collect::<HashMap<SceneAnchor, Entity>>();
        let by_entity = entries
            .into_iter()
            .map(|(anchor, entity)| (entity, anchor))
            .collect::<HashMap<Entity, SceneAnchor>>();
        Self {
            by_anchor,
            by_entity,
            occurrence_index,
            initialized: true,
            revision: 1,
            ..Default::default()
        }
    }
}

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
                Changed<Visibility>,
                Changed<Children>,
            )>,
        ),
    >,
    prims: Query<(
        Entity,
        &UsdPrimRef,
        Option<&UsdDisplayName>,
        Option<&Visibility>,
        Option<&Children>,
    )>,
    mut removed_prims: RemovedComponents<UsdPrimRef>,
    mut index: ResMut<SceneAnchorIndex>,
    mut current_projection: ResMut<CurrentHierarchyProjection>,
    provider: Option<Res<super::ActiveHierarchyProvider>>,
) {
    // ScenePatch materialization can happen across a frame boundary after
    // Spawned flips to true. Treat that lifecycle transition as a rebuild
    // trigger as well; otherwise a static stage can publish an empty tree
    // before its projected prim entities are visible to this query.
    let changed =
        spawned.is_changed() || !changed_prims.is_empty() || removed_prims.read().next().is_some();
    if !index.initialized && prims.is_empty() {
        index.initialized = true;
        *current_projection = CurrentHierarchyProjection::default();
        return;
    }
    if changed || !index.initialized {
        let prim_projection = index.rebuild(&prims);
        if provider
            .as_ref()
            .is_none_or(|provider| provider.source() == viewport_protocol::HierarchySource::Prim)
        {
            *current_projection = prim_projection;
        }
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

#[cfg(test)]
#[path = "scene_index_tests.rs"]
mod tests;
