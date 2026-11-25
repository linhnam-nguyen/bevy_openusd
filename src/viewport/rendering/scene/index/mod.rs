//! Session-local mapping between logical scene anchors and Bevy entities.
//!
//! The map is the only place where a product-facing prim identity meets an
//! ECS entity. It never leaves the viewport process.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;

use bevy::prelude::*;
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

mod dense;
mod hierarchy;
mod incremental;
mod lookup;
mod occurrence;
mod rebuild;
mod reconcile;
mod refresh;

use occurrence::SceneOccurrenceIndex;

pub(crate) use refresh::refresh_scene_anchor_index;
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
    point_occurrences: occurrence::ScenePointOccurrences,
    nodes: Vec<PrimNodeReadModel>,
    visual_unload_rows: HashMap<SceneAnchor, PrimNodeReadModel>,
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

const SCENE_INDEX_ADMISSION_BUDGET: usize = 256;
#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/scene/index/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/scene/index/lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "../../../../../tests/unit/viewport/scene/index/test_support.rs"]
mod test_support;
