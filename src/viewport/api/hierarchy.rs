//! Current provider-neutral hierarchy projection.
//!
//! The scene index owns source-specific rows, while this adapter owns the
//! single immutable projection consumed by hierarchy search and future
//! virtual providers. The projection is UI-neutral and never exposes Bevy
//! entities.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::Resource;
use viewport_protocol::{
    HierarchyChildrenPage, HierarchyNodeId, HierarchyNodeReadModel, HierarchyReadModel,
    HierarchySource, MAX_SCENE_PAGE_SIZE, PrimNodeReadModel, SceneAnchor,
};

/// Shared immutable hierarchy projection for the currently selected provider.
#[derive(Resource, Clone, Debug)]
pub(crate) struct CurrentHierarchyProjection {
    read_model: Arc<HierarchyReadModel>,
    page_index: HierarchyPageIndex,
}

/// Reusable indexes for one immutable hierarchy read model.
///
/// Provider caches may prepare a candidate read model and its paging index,
/// but only [`CurrentHierarchyProjection`] is installed as the active
/// application resource consumed by hierarchy commands and search.
#[derive(Clone, Debug)]
pub(crate) struct HierarchyPageIndex {
    by_id: HashMap<HierarchyNodeId, usize>,
    children_by_parent: HashMap<Option<HierarchyNodeId>, Vec<usize>>,
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
        Self {
            read_model: Arc::new(read_model),
            page_index,
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
        let mut children_by_parent: HashMap<Option<HierarchyNodeId>, Vec<usize>> = HashMap::new();
        for (index, node) in read_model.nodes.iter().enumerate() {
            children_by_parent
                .entry(node.parent_id.clone())
                .or_default()
                .push(index);
        }
        for children in children_by_parent.values_mut() {
            children.sort_unstable_by_key(|index| read_model.nodes[*index].id.clone());
        }

        Self {
            by_id,
            children_by_parent,
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

        let page_size = page_size.clamp(1, MAX_SCENE_PAGE_SIZE);
        let child_indices = self
            .children_by_parent
            .get(&parent_id.cloned())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let total = child_indices.len() as u32;
        let start = (page as usize).saturating_mul(page_size as usize);
        let nodes = child_indices
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
