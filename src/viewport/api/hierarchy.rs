//! Current provider-neutral hierarchy projection.
//!
//! The scene index owns source-specific rows, while this adapter owns the
//! single immutable projection consumed by hierarchy search and future
//! virtual providers. The projection is UI-neutral and never exposes Bevy
//! entities.

use std::collections::HashMap;
use std::sync::Arc;

use viewport_protocol::{
    HierarchyNodeId, HierarchyNodeReadModel, HierarchyReadModel, HierarchySource,
    PrimNodeReadModel, SceneAnchor,
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
}

/// Stable generic identity for a real prim row.
pub(crate) fn prim_node_id(anchor: &SceneAnchor) -> HierarchyNodeId {
    let instance = anchor.instance_context.as_deref().unwrap_or("single");
    HierarchyNodeId::new(format!("prim:{}:{instance}", anchor.prim_path))
}
