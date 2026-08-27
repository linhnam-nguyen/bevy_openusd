//! Generic hierarchy contracts shared by prim and virtual BIM projections.
//!
//! A hierarchy node is a presentation row, not a USD namespace mutation. Real
//! scene rows carry a [`SceneAnchor`]; virtual provider rows intentionally do
//! not. The same DTO therefore serves the existing prim tree and future BIM
//! classification without creating a second hierarchy protocol.

use serde::{Deserialize, Serialize};

use super::read_models::SceneAnchor;

/// Stable identity for one row in the active hierarchy provider.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct HierarchyNodeId(pub String);

impl HierarchyNodeId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn for_scene_anchor(anchor: &SceneAnchor) -> Self {
        let instance = anchor.instance_context.as_deref().unwrap_or("single");
        Self::new(format!("prim:{}:{instance}", anchor.prim_path))
    }
}

impl From<String> for HierarchyNodeId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Provider currently supplying the one hierarchy panel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HierarchySource {
    #[default]
    Prim,
    BimClassification,
}

/// Provider-neutral hierarchy row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HierarchyNodeReadModel {
    pub id: HierarchyNodeId,
    pub parent_id: Option<HierarchyNodeId>,
    pub name: String,
    pub breadcrumb: String,
    pub anchor: Option<SceneAnchor>,
    pub parent_anchor: Option<SceneAnchor>,
    pub visible: bool,
    pub has_children: bool,
}

impl HierarchyNodeReadModel {
    pub fn scene(
        id: HierarchyNodeId,
        parent_id: Option<HierarchyNodeId>,
        name: String,
        breadcrumb: String,
        anchor: SceneAnchor,
        parent_anchor: Option<SceneAnchor>,
        visible: bool,
        has_children: bool,
    ) -> Self {
        Self {
            id,
            parent_id,
            name,
            breadcrumb,
            anchor: Some(anchor),
            parent_anchor,
            visible,
            has_children,
        }
    }

    pub fn virtual_node(
        id: HierarchyNodeId,
        parent_id: Option<HierarchyNodeId>,
        name: String,
        breadcrumb: String,
        has_children: bool,
    ) -> Self {
        Self {
            id,
            parent_id,
            name,
            breadcrumb,
            anchor: None,
            parent_anchor: None,
            visible: true,
            has_children,
        }
    }
}

/// Immutable provider projection shared by hierarchy rendering and search.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HierarchyReadModel {
    pub source: HierarchySource,
    pub revision: u64,
    pub nodes: Vec<HierarchyNodeReadModel>,
}

/// One bounded page of direct provider children.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HierarchyChildrenPage {
    pub source: HierarchySource,
    pub parent_id: Option<HierarchyNodeId>,
    pub page: u32,
    pub page_size: u32,
    pub total: u32,
    pub nodes: Vec<HierarchyNodeReadModel>,
    pub has_more: bool,
}

/// One bounded page reference used to reveal a generic hierarchy result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HierarchyPageReference {
    pub parent_id: Option<HierarchyNodeId>,
    pub page: u32,
}

/// A search match over the same node name that the hierarchy displays.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HierarchySearchMatch {
    pub node_id: HierarchyNodeId,
    pub name: String,
    pub breadcrumb: String,
    pub anchor: Option<SceneAnchor>,
    pub parent_anchor: Option<SceneAnchor>,
    pub visible: bool,
    pub has_children: bool,
    pub reveal_pages: Vec<HierarchyPageReference>,
}
