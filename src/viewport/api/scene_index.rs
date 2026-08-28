//! Session-local mapping between logical scene anchors and Bevy entities.
//!
//! The map is the only place where a product-facing prim identity meets an
//! ECS entity. It never leaves the viewport process.

use std::collections::{HashMap, HashSet};

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdPrimRef};
use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, MAX_SCENE_PAGE_SIZE, PrimNodeReadModel, SceneAnchor,
    SceneChildrenPage, ScenePageReference, SceneReadModel, SceneSearchMatch,
};

use super::hierarchy::CurrentHierarchyProjection;
use crate::viewport::session::Spawned;

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
    initialized: bool,
    revision: u64,
}

impl SceneAnchorIndex {
    pub(crate) fn prim_projection(&self) -> CurrentHierarchyProjection {
        CurrentHierarchyProjection::from_prim_nodes(&self.nodes, self.revision)
    }

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
        let nodes: Vec<PrimNodeReadModel> = self
            .nodes
            .iter()
            .filter(|node| node.parent.as_ref() == parent)
            .cloned()
            .collect();
        let total = nodes.len() as u32;
        let start = (page as usize).saturating_mul(page_size as usize);
        let page_nodes = nodes
            .into_iter()
            .skip(start)
            .take(page_size as usize)
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
        Self {
            nodes,
            initialized: true,
            revision: 1,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn from_test_entity(anchor: SceneAnchor, entity: Entity) -> Self {
        Self {
            by_anchor: HashMap::from([(anchor.clone(), entity)]),
            by_entity: HashMap::from([(entity, anchor)]),
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
        let node = self
            .nodes
            .iter()
            .find(|node| node.anchor.prim_path == prim_path)?;
        let by_anchor: HashMap<SceneAnchor, &PrimNodeReadModel> = self
            .nodes
            .iter()
            .map(|node| (node.anchor.clone(), node))
            .collect();

        let mut ancestry = Vec::new();
        let mut current = Some(node);
        while let Some(node) = current {
            ancestry.push(node);
            current = node
                .parent
                .as_ref()
                .and_then(|parent| by_anchor.get(parent).copied());
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

    fn sibling_page(&self, node: &PrimNodeReadModel) -> u32 {
        let index = self
            .nodes
            .iter()
            .filter(|candidate| candidate.parent.as_ref() == node.parent.as_ref())
            .position(|candidate| candidate.anchor == node.anchor)
            .unwrap_or_default();
        (index as u32) / DEFAULT_SCENE_PAGE_SIZE
    }

    fn rebuild(
        &mut self,
        prims: &Query<(
            Entity,
            &UsdPrimRef,
            Option<&UsdDisplayName>,
            Option<&Visibility>,
            Option<&Children>,
        )>,
    ) -> CurrentHierarchyProjection {
        #[derive(Debug)]
        struct Candidate {
            entity: Entity,
            path: String,
            name: String,
            display_name: Option<String>,
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
            .map(|(entity, prim, display_name, visibility, children)| {
                let display_name = display_name.map(|display_name| display_name.0.clone());
                Candidate {
                    entity,
                    path: prim.path.clone(),
                    name: prim_name(&prim.path).to_owned(),
                    display_name,
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
            })
            .collect();

        let mut parent_by_child = HashMap::new();
        for candidate in &candidates {
            for child in &candidate.children {
                parent_by_child.insert(*child, candidate.entity);
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
                let anchor = by_entity.get(&candidate.entity)?.clone();
                let parent = parent_by_child
                    .get(&candidate.entity)
                    .and_then(|entity| by_entity.get(entity))
                    .cloned();
                Some(PrimNodeReadModel {
                    anchor,
                    parent,
                    label: candidate.name,
                    display_name: candidate.display_name,
                    visible: candidate.visible,
                    has_children: !candidate.children.is_empty(),
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
        self.revision = self.revision.saturating_add(1);
        self.initialized = true;
        CurrentHierarchyProjection::from_prim_nodes(&self.nodes, self.revision)
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
