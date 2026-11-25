use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, MAX_SCENE_PAGE_SIZE, SceneAnchor, SceneChildrenPage,
    ScenePageReference, SceneReadModel, SceneSearchMatch,
};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::{DenseSceneIndex, SceneAnchorIndex};

impl SceneAnchorIndex {
    pub(crate) fn prim_projection(&self) -> CurrentHierarchyProjection {
        if self.visual_unload_rows.is_empty() {
            return CurrentHierarchyProjection::from_prim_nodes(&self.nodes, self.revision);
        }
        CurrentHierarchyProjection::from_prim_nodes(&self.logical_nodes(), self.revision)
    }

    /// Snapshot logical rows before an automatic visual-only payload unload.
    /// Scene and selection resolution still use live entities when available.
    pub(crate) fn retain_visual_unload_rows(&mut self, roots: &[String]) {
        for node in &self.nodes {
            if roots.iter().any(|root| {
                node.anchor
                    .prim_path
                    .strip_prefix(root)
                    .is_some_and(|tail| tail.starts_with('/'))
            }) {
                self.visual_unload_rows
                    .insert(node.anchor.clone(), node.clone());
            }
        }
    }

    pub(crate) fn clear_visual_unload_rows(&mut self) {
        if !self.visual_unload_rows.is_empty() {
            self.visual_unload_rows.clear();
            self.derived_dirty = true;
        }
    }

    fn logical_nodes(&self) -> Vec<viewport_protocol::PrimNodeReadModel> {
        let mut by_anchor: std::collections::HashMap<_, _> = self
            .visual_unload_rows
            .values()
            .map(|row| (row.anchor.clone(), row.clone()))
            .collect();
        for node in &self.nodes {
            by_anchor.insert(node.anchor.clone(), node.clone());
        }
        let parents: Vec<_> = by_anchor
            .values()
            .filter_map(|row| row.parent.clone())
            .collect();
        for parent in parents {
            if let Some(row) = by_anchor.get_mut(&parent) {
                row.has_children = true;
            }
        }
        let mut rows: Vec<_> = by_anchor.into_values().collect();
        rows.sort_unstable_by(|a, b| a.anchor.cmp(&b.anchor));
        rows
    }

    pub(super) fn rebuild_logical_dense(&mut self) {
        if self.visual_unload_rows.is_empty() {
            self.dense = DenseSceneIndex::from_nodes(&self.nodes, &self.by_anchor);
        } else {
            self.dense = DenseSceneIndex::from_nodes(&self.logical_nodes(), &self.by_anchor);
        }
    }

    /// Returns the bounded initial tree payload. Descendants stay in the
    /// authoritative server index and are requested by the client when a
    /// parent is expanded.
    pub(crate) fn roots_read_model(&self) -> SceneReadModel {
        let page = self.children_page(None, 0, DEFAULT_SCENE_PAGE_SIZE);
        SceneReadModel {
            prims: page.nodes,
            total_prims: self.dense.nodes.len() as u32,
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
