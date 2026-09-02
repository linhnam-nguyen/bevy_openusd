//! Current provider-neutral hierarchy projection.
//!
//! The scene index owns source-specific rows, while this adapter owns the
//! single immutable projection consumed by hierarchy search and future
//! virtual providers. The projection is UI-neutral and never exposes Bevy
//! entities.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use bevy::prelude::Resource;
use viewport_protocol::{
    ClassificationRecipe, HierarchyChildrenPage, HierarchyNodeId, HierarchyNodeReadModel,
    HierarchyReadModel, HierarchySource, MAX_SCENE_PAGE_SIZE, PrimNodeReadModel, SceneAnchor,
};

/// Session-local provider selection. The recipe is retained with the
/// selection so a semantic snapshot refresh can rebuild the same projection.
#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct ActiveHierarchyProvider {
    source: HierarchySource,
    classification_recipe: Option<ClassificationRecipe>,
}

impl ActiveHierarchyProvider {
    pub(crate) fn source(&self) -> HierarchySource {
        self.source
    }

    pub(crate) fn classification_recipe(&self) -> Option<&ClassificationRecipe> {
        self.classification_recipe.as_ref()
    }

    pub(crate) fn set(&mut self, source: HierarchySource, recipe: Option<ClassificationRecipe>) {
        self.source = source;
        self.classification_recipe = recipe;
    }
}

/// Shared immutable hierarchy projection for the currently selected provider.
#[derive(Resource, Clone, Debug)]
pub(crate) struct CurrentHierarchyProjection {
    pub(super) read_model: Arc<HierarchyReadModel>,
    pub(super) page_index: HierarchyPageIndex,
    pub(super) visibility_index: HierarchyVisibilityIndex,
}

/// Backend-only target set for provider-neutral hierarchy visibility actions.
/// Semantic BIM paths are resolved to all current native-instance occurrences
/// by [`SceneAnchorIndex`] at command application time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HierarchyVisibilityTarget {
    SceneAnchor(SceneAnchor),
    PrimPath(String),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct HierarchyVisibilityIndex {
    targets_by_node: HashMap<HierarchyNodeId, Vec<HierarchyVisibilityTarget>>,
    nodes_by_prim_path: HashMap<String, Vec<HierarchyNodeId>>,
}

impl HierarchyVisibilityIndex {
    pub(crate) fn from_read_model(read_model: &HierarchyReadModel) -> Self {
        let targets_by_node = read_model
            .nodes
            .iter()
            .filter_map(|node| {
                node.anchor.as_ref().map(|anchor| {
                    (
                        node.id.clone(),
                        vec![HierarchyVisibilityTarget::SceneAnchor(anchor.clone())],
                    )
                })
            })
            .collect();
        Self::from_targets(targets_by_node)
    }

    pub(crate) fn from_targets(
        targets_by_node: HashMap<HierarchyNodeId, Vec<HierarchyVisibilityTarget>>,
    ) -> Self {
        let mut nodes_by_prim_path: HashMap<String, Vec<HierarchyNodeId>> = HashMap::new();
        for (node_id, targets) in &targets_by_node {
            for target in targets {
                if let HierarchyVisibilityTarget::PrimPath(path) = target {
                    nodes_by_prim_path
                        .entry(path.clone())
                        .or_default()
                        .push(node_id.clone());
                }
            }
        }
        Self {
            targets_by_node,
            nodes_by_prim_path,
        }
    }

    pub(crate) fn targets_for(
        &self,
        node_id: &HierarchyNodeId,
    ) -> Option<&[HierarchyVisibilityTarget]> {
        self.targets_by_node.get(node_id).map(Vec::as_slice)
    }

    pub(crate) fn nodes_for_prim_paths<'a>(
        &'a self,
        paths: impl IntoIterator<Item = &'a str>,
    ) -> impl Iterator<Item = &'a HierarchyNodeId> {
        paths
            .into_iter()
            .flat_map(|path| self.nodes_by_prim_path.get(path).into_iter().flatten())
    }
}

/// Reusable indexes for one immutable hierarchy read model.
///
/// Provider caches may prepare a candidate read model and its paging index,
/// but only [`CurrentHierarchyProjection`] is installed as the active
/// application resource consumed by hierarchy commands and search.
#[derive(Clone, Debug)]
pub(crate) struct HierarchyPageIndex {
    by_id: HashMap<HierarchyNodeId, usize>,
    /// One range per node plus slot zero for roots. The range points into the
    /// single dense child-order array, so a page never rebuilds a parent map.
    child_ranges: Vec<Range<usize>>,
    child_order: Vec<usize>,
}

impl Default for CurrentHierarchyProjection {
    fn default() -> Self {
        Self::from_prim_nodes(&[], 0)
    }
}

impl CurrentHierarchyProjection {
    pub(crate) fn from_prim_nodes(nodes: &[PrimNodeReadModel], revision: u64) -> Self {
        let ids: HashMap<SceneAnchor, HierarchyNodeId> = nodes
            .iter()
            .map(|node| (node.anchor.clone(), prim_node_id(&node.anchor)))
            .collect();

        let nodes = nodes
            .iter()
            .map(|node| {
                let parent_id = node
                    .parent
                    .as_ref()
                    .and_then(|parent| ids.get(parent).cloned());
                HierarchyNodeReadModel::scene(
                    ids[&node.anchor].clone(),
                    parent_id,
                    node.label.clone(),
                    node.anchor.prim_path.clone(),
                    node.anchor.clone(),
                    node.parent.clone(),
                    node.visible,
                    node.has_children,
                )
            })
            .collect();

        Self::from_read_model(HierarchyReadModel {
            source: HierarchySource::Prim,
            revision,
            nodes,
        })
    }

    pub(crate) fn from_read_model(read_model: HierarchyReadModel) -> Self {
        let page_index = HierarchyPageIndex::from_read_model(&read_model);
        let visibility_index = HierarchyVisibilityIndex::from_read_model(&read_model);
        Self {
            read_model: Arc::new(read_model),
            page_index,
            visibility_index,
        }
    }

    /// Installs provider-owned immutable parts without cloning the read model.
    /// The page index must have been built from the same `Arc` contents.
    pub(crate) fn from_shared_parts(
        read_model: Arc<HierarchyReadModel>,
        page_index: HierarchyPageIndex,
    ) -> Self {
        let visibility_index = HierarchyVisibilityIndex::from_read_model(&read_model);
        Self::from_shared_parts_with_visibility(read_model, page_index, visibility_index)
    }

    pub(crate) fn from_shared_parts_with_visibility(
        read_model: Arc<HierarchyReadModel>,
        page_index: HierarchyPageIndex,
        visibility_index: HierarchyVisibilityIndex,
    ) -> Self {
        Self {
            read_model,
            page_index,
            visibility_index,
        }
    }

    pub(crate) fn source(&self) -> HierarchySource {
        self.read_model.source
    }

    pub(crate) fn snapshot(&self) -> Arc<HierarchyReadModel> {
        Arc::clone(&self.read_model)
    }

    pub(crate) fn children_page(
        &self,
        parent_id: Option<&HierarchyNodeId>,
        page: u32,
        page_size: u32,
    ) -> Result<HierarchyChildrenPage, String> {
        self.page_index
            .children_page(&self.read_model, parent_id, page, page_size)
    }
}

impl HierarchyPageIndex {
    pub(crate) fn from_read_model(read_model: &HierarchyReadModel) -> Self {
        let by_id = read_model
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<HashMap<_, _>>();
        // Build a compact topology once at publication time. Queries use only
        // the parent node id, a range lookup, and the requested page slice.
        let mut children_by_parent = vec![Vec::new(); read_model.nodes.len() + 1];
        for (index, node) in read_model.nodes.iter().enumerate() {
            let slot = node
                .parent_id
                .as_ref()
                .and_then(|parent| by_id.get(parent).copied())
                .map_or(0, |parent| parent + 1);
            children_by_parent[slot].push(index);
        }
        for children in &mut children_by_parent {
            children.sort_unstable_by_key(|index| read_model.nodes[*index].id.clone());
        }

        let mut child_order = Vec::with_capacity(read_model.nodes.len());
        let mut child_ranges = Vec::with_capacity(children_by_parent.len());
        for children in children_by_parent {
            let start = child_order.len();
            child_order.extend(children);
            child_ranges.push(start..child_order.len());
        }

        Self {
            by_id,
            child_ranges,
            child_order,
        }
    }

    pub(crate) fn children_page(
        &self,
        read_model: &HierarchyReadModel,
        parent_id: Option<&HierarchyNodeId>,
        page: u32,
        page_size: u32,
    ) -> Result<HierarchyChildrenPage, String> {
        if let Some(parent_id) = parent_id
            && !self.by_id.contains_key(parent_id)
        {
            return Err(format!(
                "unknown hierarchy parent id `{}`",
                parent_id.as_str()
            ));
        }

        let parent_slot = parent_id
            .map(|parent_id| {
                self.by_id
                    .get(parent_id)
                    .copied()
                    .map(|index| index + 1)
                    .ok_or_else(|| format!("unknown hierarchy parent id `{}`", parent_id.as_str()))
            })
            .transpose()?
            .unwrap_or(0);
        let page_size = page_size.clamp(1, MAX_SCENE_PAGE_SIZE);
        let child_range = &self.child_ranges[parent_slot];
        let total = child_range.len() as u32;
        let start = (page as usize).saturating_mul(page_size as usize);
        let nodes = self.child_order[child_range.clone()]
            .iter()
            .skip(start)
            .take(page_size as usize)
            .map(|index| read_model.nodes[*index].clone())
            .collect();

        Ok(HierarchyChildrenPage {
            source: read_model.source,
            parent_id: parent_id.cloned(),
            page,
            page_size,
            total,
            has_more: start.saturating_add(page_size as usize) < total as usize,
            nodes,
        })
    }
}

/// Stable generic identity for a real prim row.
pub(crate) fn prim_node_id(anchor: &SceneAnchor) -> HierarchyNodeId {
    let instance = anchor.instance_context.as_deref().unwrap_or("single");
    HierarchyNodeId::new(format!("prim:{}:{instance}", anchor.prim_path))
}
