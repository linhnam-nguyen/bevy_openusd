use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, MAX_SCENE_PAGE_SIZE, SceneAnchor, SceneChildrenPage,
    ScenePageReference, SceneReadModel, SceneSearchMatch,
};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::SceneAnchorIndex;

impl SceneAnchorIndex {
    pub(crate) fn prim_projection(&self) -> CurrentHierarchyProjection {
        CurrentHierarchyProjection::from_prim_nodes(&self.nodes, self.revision)
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
}
