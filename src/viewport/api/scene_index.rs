//! Session-local mapping between logical scene anchors and Bevy entities.
//!
//! The map is the only place where a product-facing prim identity meets an
//! ECS entity. It never leaves the viewport process.

use std::collections::HashMap;
use std::ops::Range;

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdPrimRef};
use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, HierarchyVisibilityState, MAX_SCENE_PAGE_SIZE, PrimNodeReadModel,
    SceneAnchor, SceneChildrenPage, ScenePageReference, SceneReadModel, SceneSearchMatch,
};

use super::hierarchy::CurrentHierarchyProjection;
use super::scene_occurrence_index::SceneOccurrenceIndex;
use crate::viewport::session::Spawned;

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

impl DenseSceneIndex {
    pub(super) fn from_nodes(
        nodes: &[PrimNodeReadModel],
        entities: &HashMap<SceneAnchor, Entity>,
    ) -> Self {
        let by_anchor = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.anchor.clone(), index))
            .collect::<HashMap<_, _>>();
        let mut dense_nodes = nodes
            .iter()
            .map(|node| DenseSceneNode {
                entity: entities.get(&node.anchor).copied(),
                anchor: node.anchor.clone(),
                parent: node
                    .parent
                    .as_ref()
                    .and_then(|parent| by_anchor.get(parent).copied()),
                first_child: 0,
                child_count: 0,
                sibling_index: 0,
                label: node.label.clone(),
                display_name: node.display_name.clone(),
                visible: node.visible,
                has_children: node.has_children,
            })
            .collect::<Vec<_>>();

        let by_entity = dense_nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.entity.map(|entity| (entity, index)))
            .collect::<HashMap<_, _>>();
        let mut by_path: HashMap<String, Vec<usize>> = HashMap::new();
        let mut children_by_parent = vec![Vec::new(); dense_nodes.len() + 1];
        for (index, node) in dense_nodes.iter().enumerate() {
            by_path
                .entry(node.anchor.prim_path.clone())
                .or_default()
                .push(index);
            let slot = node.parent.map_or(0, |parent| parent + 1);
            children_by_parent[slot].push(index);
        }
        for children in &mut children_by_parent {
            children.sort_unstable_by(|left, right| {
                dense_nodes[*left].anchor.cmp(&dense_nodes[*right].anchor)
            });
        }

        let mut child_order = Vec::with_capacity(dense_nodes.len());
        let mut child_ranges = Vec::with_capacity(children_by_parent.len());
        for (parent_slot, children) in children_by_parent.into_iter().enumerate() {
            let start = child_order.len();
            for (sibling_index, child) in children.into_iter().enumerate() {
                dense_nodes[child].sibling_index = sibling_index;
                child_order.push(child);
            }
            let end = child_order.len();
            if parent_slot > 0 {
                let parent = parent_slot - 1;
                dense_nodes[parent].first_child = start;
                dense_nodes[parent].child_count = end - start;
            }
            child_ranges.push(start..end);
        }

        Self {
            nodes: dense_nodes,
            by_anchor,
            by_entity,
            by_path,
            child_ranges,
            child_order,
        }
    }

    fn node(&self, index: usize) -> Option<&DenseSceneNode> {
        self.nodes.get(index)
    }

    fn children(&self, parent: Option<usize>) -> &[usize] {
        let range = match parent {
            Some(parent) => self
                .node(parent)
                .map(|node| node.first_child..node.first_child.saturating_add(node.child_count)),
            None => self.child_ranges.first().cloned(),
        };
        range
            .map(|range| &self.child_order[range])
            .unwrap_or_default()
    }

    fn protocol_node(&self, index: usize) -> Option<PrimNodeReadModel> {
        let node = self.node(index)?;
        Some(PrimNodeReadModel {
            anchor: node.anchor.clone(),
            parent: node
                .parent
                .and_then(|parent| self.node(parent).map(|node| node.anchor.clone())),
            label: node.label.clone(),
            display_name: node.display_name.clone(),
            visible: node.visible,
            has_children: node.has_children,
        })
    }
}

impl SceneAnchorIndex {
    pub(crate) fn prim_projection(&self) -> CurrentHierarchyProjection {
        CurrentHierarchyProjection::from_prim_nodes(&self.nodes, self.revision)
    }

    pub(crate) fn resolve(&self, anchor: &SceneAnchor) -> Option<Entity> {
        self.dense
            .by_anchor
            .get(anchor)
            .and_then(|index| self.dense.node(*index))
            .and_then(|node| node.entity)
            .or_else(|| self.by_anchor.get(anchor).copied())
    }

    /// Resolves every current scene occurrence for a semantic prim path.
    /// Semantic classification entries intentionally carry path identity only;
    /// native-instance projection may expose the same path under multiple
    /// scene-local instance contexts.
    pub(crate) fn resolve_all_by_prim_path(&self, prim_path: &str) -> &[Entity] {
        self.occurrence_index.resolve(prim_path)
    }

    pub(crate) fn visibility_for_anchor(&self, anchor: &SceneAnchor) -> HierarchyVisibilityState {
        self.dense
            .by_anchor
            .get(anchor)
            .and_then(|index| self.dense.node(*index))
            .map_or(HierarchyVisibilityState::Visible, |node| {
                HierarchyVisibilityState::from_visible(node.visible)
            })
    }

    pub(crate) fn visibility_for_prim_path(&self, prim_path: &str) -> HierarchyVisibilityState {
        let mut states = self
            .dense
            .by_path
            .get(prim_path)
            .into_iter()
            .flat_map(|indices| indices.iter())
            .filter_map(|index| self.dense.node(*index))
            .map(|node| HierarchyVisibilityState::from_visible(node.visible));
        let Some(first) = states.next() else {
            return HierarchyVisibilityState::Visible;
        };
        if states.all(|state| state == first) {
            first
        } else {
            HierarchyVisibilityState::Mixed
        }
    }

    pub(crate) fn anchor_for(&self, entity: Entity) -> Option<SceneAnchor> {
        self.dense
            .by_entity
            .get(&entity)
            .and_then(|index| self.dense.node(*index))
            .map(|node| node.anchor.clone())
            .or_else(|| self.by_entity.get(&entity).cloned())
    }

    /// Returns the bounded initial tree payload. Descendants stay in the
    /// authoritative server index and are requested by the client when a
    /// parent is expanded.
    pub(crate) fn roots_read_model(&self) -> SceneReadModel {
        let page = self.children_page(None, 0, DEFAULT_SCENE_PAGE_SIZE);
        SceneReadModel {
            prims: page.nodes,
            total_prims: self.nodes.len() as u32,
            total_roots: page.total,
            root_page_size: page.page_size,
        }
    }

    pub(crate) fn children_page(
        &self,
        parent: Option<&SceneAnchor>,
        page: u32,
        page_size: u32,
    ) -> SceneChildrenPage {
        let page_size = if page_size == 0 {
            DEFAULT_SCENE_PAGE_SIZE
        } else {
            page_size.min(MAX_SCENE_PAGE_SIZE)
        };
        let children = match parent {
            Some(parent) => self
                .dense
                .by_anchor
                .get(parent)
                .map(|index| self.dense.children(Some(*index)))
                .unwrap_or_default(),
            None => self.dense.children(None),
        };
        let total = children.len() as u32;
        let start = (page as usize).saturating_mul(page_size as usize);
        let page_nodes = children
            .into_iter()
            .skip(start)
            .take(page_size as usize)
            .filter_map(|index| self.dense.protocol_node(*index))
            .collect();

        SceneChildrenPage {
            parent: parent.cloned(),
            page,
            page_size,
            total,
            nodes: page_nodes,
        }
    }

    #[cfg(test)]
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    /// Resolves an existing prim row into the current runtime tree representation.
    ///
    /// This index owns the session-local anchor, visibility, hierarchy, and
    /// reveal-page information required by the viewport protocol.
    pub(crate) fn search_match_for_path(&self, prim_path: &str) -> Option<SceneSearchMatch> {
        let node_index = self.dense.by_path.get(prim_path)?.first().copied()?;
        let node = self.dense.node(node_index)?;
        let mut ancestry = Vec::new();
        let mut current = Some(node_index);
        while let Some(index) = current {
            let node = self.dense.node(index)?;
            ancestry.push(index);
            current = node.parent;
        }

        let reveal_pages = ancestry
            .into_iter()
            .rev()
            .filter_map(|index| {
                let node = self.dense.node(index)?;
                Some(ScenePageReference {
                    parent: node
                        .parent
                        .and_then(|parent| self.dense.node(parent))
                        .map(|node| node.anchor.clone()),
                    page: (node.sibling_index as u32) / DEFAULT_SCENE_PAGE_SIZE,
                })
            })
            .collect();

        Some(SceneSearchMatch {
            anchor: node.anchor.clone(),
            parent: node
                .parent
                .and_then(|parent| self.dense.node(parent))
                .map(|node| node.anchor.clone()),
            label: node.label.clone(),
            breadcrumb: node.anchor.prim_path.clone(),
            visible: node.visible,
            has_children: node.has_children,
            reveal_pages,
        })
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
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
