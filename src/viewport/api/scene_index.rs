//! Session-local mapping between logical scene anchors and Bevy entities.
//!
//! The map is the only place where a product-facing prim identity meets an
//! ECS entity. It never leaves the viewport process.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;

use bevy::prelude::*;
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

#[cfg(test)]
use viewport_protocol::MAX_SCENE_PAGE_SIZE;

use super::scene_occurrence_index::SceneOccurrenceIndex;

#[path = "scene_index_dense.rs"]
mod dense;
#[path = "scene_index_hierarchy.rs"]
mod hierarchy;
#[path = "scene_index_incremental.rs"]
mod incremental;
#[path = "scene_index_lookup.rs"]
mod lookup;
#[path = "scene_index_rebuild.rs"]
mod rebuild;
#[path = "scene_index_reconcile.rs"]
mod reconcile;
#[path = "scene_index_refresh.rs"]
mod refresh;

pub(in crate::viewport) use refresh::refresh_scene_anchor_index;
pub(crate) use refresh::register_scene_index_observers;

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
    node_index_by_anchor: HashMap<SceneAnchor, usize>,
    dense: DenseSceneIndex,
    initialized: bool,
    derived_dirty: bool,
    revision: u64,
    rebuild_count: u64,
    incremental_work: SceneIndexWorkCounters,
    parent_by_entity: HashMap<Entity, Entity>,
    transparent_by_entity: HashMap<Entity, bool>,
    pending_additions: VecDeque<Entity>,
    queued_additions: HashSet<Entity>,
    reconcile: reconcile::SceneIndexReconcileState,
    last_refresh_admitted: usize,
}

/// Cumulative work performed by the progressive scene-index path.
///
/// The counters describe rows actually admitted, reindexed, and materialized
/// into the immutable projection; they do not substitute a row count for
/// derived work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SceneIndexWorkCounters {
    pub(crate) admitted_rows: u64,
    pub(crate) reindexed_rows: u64,
    pub(crate) projected_rows: u64,
    pub(crate) derived_flushes: u64,
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
        let node_index_by_anchor = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.anchor.clone(), index))
            .collect();
        Self {
            nodes,
            node_index_by_anchor,
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

    pub(crate) fn rebuild_count(&self) -> u64 {
        self.rebuild_count
    }

    pub(crate) fn incremental_work(&self) -> SceneIndexWorkCounters {
        self.incremental_work
    }

    pub(crate) fn pending_addition_count(&self) -> usize {
        self.pending_additions.len()
    }

    pub(crate) fn last_refresh_admitted(&self) -> usize {
        self.last_refresh_admitted
    }

    pub(crate) fn last_reconcile_capture_work(&self) -> usize {
        self.reconcile.last_capture_work()
    }

    pub(crate) fn reconcile_is_pending(&self) -> bool {
        self.reconcile.is_pending()
    }
}

const SCENE_INDEX_ADMISSION_BUDGET: usize = 256;
#[cfg(test)]
#[path = "scene_index_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "scene_index_lifecycle_tests.rs"]
mod lifecycle_tests;
