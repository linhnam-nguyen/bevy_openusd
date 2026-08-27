//! UI-neutral hierarchy projection types shared by the scene adapter and search.
//!
//! A hierarchy node owns the name and breadcrumb currently presented to the
//! user. A prim path is optional identity metadata, not the search algorithm's
//! source of names. This keeps future classification/filter projections on the
//! same search boundary as today's USD prim-tree projection.

use std::collections::HashMap;

use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HierarchyNodeId(pub(crate) String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HierarchyNode {
    pub(crate) id: HierarchyNodeId,
    pub(crate) parent_id: Option<HierarchyNodeId>,
    pub(crate) name: String,
    pub(crate) breadcrumb: String,
    pub(crate) prim_path: Option<String>,
    pub(crate) anchor: Option<SceneAnchor>,
    pub(crate) parent_anchor: Option<SceneAnchor>,
    pub(crate) visible: bool,
    pub(crate) has_children: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HierarchyReadModel {
    pub(crate) nodes: Vec<HierarchyNode>,
}

impl HierarchyReadModel {
    /// Adapts the current prim-tree read model into the search projection.
    ///
    /// `PrimNodeReadModel::label` is populated by the prim-tree builder as the
    /// node's own name. Search does not need to know how that name was derived.
    pub(crate) fn from_prim_nodes(nodes: &[PrimNodeReadModel]) -> Self {
        let ids: HashMap<SceneAnchor, HierarchyNodeId> = nodes
            .iter()
            .map(|node| {
                (
                    node.anchor.clone(),
                    HierarchyNodeId(format!(
                        "prim:{}:{:?}",
                        node.anchor.prim_path, node.anchor.instance_context
                    )),
                )
            })
            .collect();

        Self {
            nodes: nodes
                .iter()
                .map(|node| HierarchyNode {
                    id: ids[&node.anchor].clone(),
                    parent_id: node
                        .parent
                        .as_ref()
                        .and_then(|parent| ids.get(parent).cloned()),
                    name: node.label.clone(),
                    breadcrumb: node.anchor.prim_path.clone(),
                    prim_path: Some(node.anchor.prim_path.clone()),
                    anchor: Some(node.anchor.clone()),
                    parent_anchor: node.parent.clone(),
                    visible: node.visible,
                    has_children: node.has_children,
                })
                .collect(),
        }
    }
}
