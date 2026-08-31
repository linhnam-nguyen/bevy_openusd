//! Session-local mapping between logical scene anchors and Bevy entities.
//!
//! The map is the only place where a product-facing prim identity meets an
//! ECS entity. It never leaves the viewport process.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdPrimRef, UsdTransparentHierarchyNode};
use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, MAX_SCENE_PAGE_SIZE, PrimNodeReadModel, SceneAnchor,
    SceneChildrenPage, ScenePageReference, SceneReadModel, SceneSearchMatch,
};

use super::hierarchy::HierarchyReadModel;

#[path = "scene_index_refresh.rs"]
mod refresh;
pub(crate) use refresh::refresh_scene_anchor_index;

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
    nodes: Vec<PrimNodeReadModel>,
    node_by_anchor: HashMap<SceneAnchor, usize>,
    first_node_by_path: HashMap<String, usize>,
    children_by_parent: HashMap<Option<SceneAnchor>, Vec<usize>>,
    page_by_anchor: HashMap<SceneAnchor, u32>,
    hierarchy: Arc<HierarchyReadModel>,
    initialized: bool,
    revision: u64,
}

impl SceneAnchorIndex {
    pub(crate) fn resolve(&self, anchor: &SceneAnchor) -> Option<Entity> {
        self.by_anchor.get(anchor).copied()
    }

    pub(crate) fn anchor_for(&self, entity: Entity) -> Option<SceneAnchor> {
        self.by_entity.get(&entity).cloned()
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
        let parent = parent.cloned();
        let children = self.children_by_parent.get(&parent);
        let total = children.map_or(0, |children| children.len()) as u32;
        let start = (page as usize).saturating_mul(page_size as usize);
        let page_nodes = children
            .into_iter()
            .flat_map(|children| children.iter().skip(start).take(page_size as usize))
            .filter_map(|index| self.nodes.get(*index).cloned())
            .collect();

        SceneChildrenPage {
            parent,
            page,
            page_size,
            total,
            nodes: page_nodes,
        }
    }

    /// Returns the immutable hierarchy projection built with the scene index.
    /// Search consumes this projection rather than inspecting USD paths or
    /// semantic storage itself. Cloning the `Arc` is constant-time.
    pub(crate) fn hierarchy_snapshot(&self) -> Arc<HierarchyReadModel> {
        Arc::clone(&self.hierarchy)
    }

    #[cfg(test)]
    pub(crate) fn from_test_nodes(nodes: Vec<PrimNodeReadModel>) -> Self {
        let mut index = Self {
            nodes,
            initialized: true,
            revision: 1,
            ..Default::default()
        };
        index.hierarchy = Arc::new(HierarchyReadModel::from_prim_nodes(&index.nodes));
        index.rebuild_read_indexes();
        index
    }

    /// Resolves an existing prim row into the current runtime tree representation.
    ///
    /// This index owns the session-local anchor, visibility, hierarchy, and
    /// reveal-page information required by the viewport protocol.
    pub(crate) fn search_match_for_path(&self, prim_path: &str) -> Option<SceneSearchMatch> {
        let node_index = *self.first_node_by_path.get(prim_path)?;
        let node = self.nodes.get(node_index)?;

        let mut ancestry = Vec::new();
        let mut current = Some(node_index);
        while let Some(index) = current {
            let node = self.nodes.get(index)?;
            ancestry.push(node);
            current = node
                .parent
                .as_ref()
                .and_then(|parent| self.node_by_anchor.get(parent).copied());
        }

        let reveal_pages = ancestry
            .into_iter()
            .rev()
            .map(|node| ScenePageReference {
                parent: node.parent.clone(),
                page: self.sibling_page(node),
            })
            .collect();

        Some(SceneSearchMatch {
            anchor: node.anchor.clone(),
            parent: node.parent.clone(),
            label: node
                .display_name
                .clone()
                .unwrap_or_else(|| node.label.clone()),
            breadcrumb: node.anchor.prim_path.clone(),
            visible: node.visible,
            has_children: node.has_children,
            reveal_pages,
        })
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    fn sibling_page(&self, node: &PrimNodeReadModel) -> u32 {
        self.page_by_anchor
            .get(&node.anchor)
            .copied()
            .unwrap_or_default()
    }

    fn rebuild_read_indexes(&mut self) {
        self.node_by_anchor.clear();
        self.first_node_by_path.clear();
        self.children_by_parent.clear();
        self.page_by_anchor.clear();
        for (index, node) in self.nodes.iter().enumerate() {
            self.node_by_anchor.insert(node.anchor.clone(), index);
            self.first_node_by_path
                .entry(node.anchor.prim_path.clone())
                .or_insert(index);
            self.children_by_parent
                .entry(node.parent.clone())
                .or_default()
                .push(index);
        }
        for children in self.children_by_parent.values() {
            for (index, node_index) in children.iter().enumerate() {
                if let Some(node) = self.nodes.get(*node_index) {
                    self.page_by_anchor.insert(
                        node.anchor.clone(),
                        (index as u32) / DEFAULT_SCENE_PAGE_SIZE,
                    );
                }
            }
        }
    }

    fn rebuild(
        &mut self,
        prims: &Query<(
            Entity,
            &UsdPrimRef,
            Option<&UsdDisplayName>,
            Option<&UsdTransparentHierarchyNode>,
            Option<&Visibility>,
            Option<&Children>,
        )>,
    ) {
        #[derive(Debug)]
        struct Candidate {
            entity: Entity,
            path: String,
            name: String,
            display_name: Option<String>,
            transparent: bool,
            visible: bool,
            children: Vec<Entity>,
        }

        let prim_entities: HashSet<Entity> = prims
            .iter()
            .filter(|(_, prim, ..)| prim.path != "/")
            .map(|(entity, ..)| entity)
            .collect();
        let mut candidates: Vec<Candidate> = prims
            .iter()
            .filter(|(_, prim, ..)| prim.path != "/")
            .map(
                |(entity, prim, display_name, transparent, visibility, children)| {
                    let display_name = display_name.map(|display_name| display_name.0.clone());
                    Candidate {
                        entity,
                        path: prim.path.clone(),
                        name: prim_name(&prim.path).to_owned(),
                        display_name,
                        transparent: transparent.is_some(),
                        visible: !matches!(visibility, Some(Visibility::Hidden)),
                        children: children
                            .map(|children| {
                                children
                                    .iter()
                                    .filter(|child| prim_entities.contains(child))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    }
                },
            )
            .collect();

        let mut parent_by_child = HashMap::new();
        for candidate in &candidates {
            for child in &candidate.children {
                parent_by_child.insert(*child, candidate.entity);
            }
        }

        let transparent_by_entity: HashMap<Entity, bool> = candidates
            .iter()
            .map(|candidate| (candidate.entity, candidate.transparent))
            .collect();
        let visual_parent = |entity: Entity| {
            let mut parent = parent_by_child.get(&entity).copied();
            while let Some(candidate) = parent {
                if !transparent_by_entity
                    .get(&candidate)
                    .copied()
                    .unwrap_or(false)
                {
                    break;
                }
                parent = parent_by_child.get(&candidate).copied();
            }
            parent
        };
        let mut visual_child_counts: HashMap<Entity, usize> = HashMap::new();
        for candidate in &candidates {
            if candidate.transparent {
                continue;
            }
            if let Some(parent) = visual_parent(candidate.entity) {
                *visual_child_counts.entry(parent).or_default() += 1;
            }
        }

        let mut path_counts: HashMap<String, usize> = HashMap::new();
        for candidate in &candidates {
            *path_counts.entry(candidate.path.clone()).or_default() += 1;
        }
        candidates.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
        });

        let mut next_occurrence: HashMap<String, usize> = HashMap::new();
        let mut by_anchor = HashMap::with_capacity(candidates.len());
        let mut by_entity = HashMap::with_capacity(candidates.len());
        for candidate in &candidates {
            let count = path_counts[&candidate.path];
            let occurrence = next_occurrence.entry(candidate.path.clone()).or_default();
            let instance_context = if count > 1 {
                let context = format!("occurrence-{occurrence}");
                *occurrence += 1;
                Some(context)
            } else {
                None
            };
            let anchor = SceneAnchor {
                session_id: None,
                prim_path: candidate.path.clone(),
                instance_context,
            };
            by_anchor.insert(anchor.clone(), candidate.entity);
            by_entity.insert(candidate.entity, anchor);
        }

        let mut nodes: Vec<PrimNodeReadModel> = candidates
            .into_iter()
            .filter_map(|candidate| {
                if candidate.transparent {
                    return None;
                }
                let anchor = by_entity.get(&candidate.entity)?.clone();
                let parent = visual_parent(candidate.entity)
                    .and_then(|entity| by_entity.get(&entity))
                    .cloned();
                Some(PrimNodeReadModel {
                    anchor,
                    parent,
                    label: candidate.name,
                    display_name: candidate.display_name,
                    visible: candidate.visible,
                    has_children: visual_child_counts
                        .get(&candidate.entity)
                        .copied()
                        .unwrap_or_default()
                        > 0,
                })
            })
            .collect();
        nodes.sort_by(|left, right| {
            left.anchor
                .prim_path
                .cmp(&right.anchor.prim_path)
                .then_with(|| {
                    left.anchor
                        .instance_context
                        .cmp(&right.anchor.instance_context)
                })
        });

        self.by_anchor = by_anchor;
        self.by_entity = by_entity;
        self.nodes = nodes;
        self.rebuild_read_indexes();
        self.hierarchy = Arc::new(HierarchyReadModel::from_prim_nodes(&self.nodes));
        self.initialized = true;
        self.revision = self.revision.saturating_add(1);
    }
}

#[cfg(test)]
#[path = "scene_index_tests.rs"]
mod tests;
