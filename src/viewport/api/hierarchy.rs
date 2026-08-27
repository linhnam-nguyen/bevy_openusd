//! Current provider-neutral hierarchy projection.
//!
//! The scene index owns source-specific rows, while this adapter owns the
//! single immutable projection consumed by hierarchy search and future
//! virtual providers. The projection is UI-neutral and never exposes Bevy
//! entities.

use std::collections::HashMap;
use std::sync::Arc;

use viewport_protocol::{
    HierarchyChildrenPage, HierarchyNodeId, HierarchyNodeReadModel, HierarchyReadModel,
    HierarchySource, MAX_SCENE_PAGE_SIZE, PrimNodeReadModel, SceneAnchor,
};

/// Shared immutable hierarchy projection for the currently selected provider.
#[derive(Clone, Debug)]
pub(crate) struct CurrentHierarchyProjection {
    read_model: Arc<HierarchyReadModel>,
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

        Self {
            read_model: Arc::new(HierarchyReadModel {
                source: HierarchySource::Prim,
                revision,
                nodes,
            }),
        }
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
        if let Some(parent_id) = parent_id
            && !self
                .read_model
                .nodes
                .iter()
                .any(|node| &node.id == parent_id)
        {
            return Err(format!(
                "unknown hierarchy parent id `{}`",
                parent_id.as_str()
            ));
        }

        let page_size = page_size.clamp(1, MAX_SCENE_PAGE_SIZE);
        let total = self
            .read_model
            .nodes
            .iter()
            .filter(|node| node.parent_id.as_ref() == parent_id)
            .count() as u32;
        let start = (page as usize).saturating_mul(page_size as usize);
        let nodes = self
            .read_model
            .nodes
            .iter()
            .filter(|node| node.parent_id.as_ref() == parent_id)
            .skip(start)
            .take(page_size as usize)
            .cloned()
            .collect();

        Ok(HierarchyChildrenPage {
            source: self.read_model.source,
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
