use bevy::prelude::Entity;
use std::collections::HashMap;
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::{DenseSceneIndex, DenseSceneNode};

impl DenseSceneIndex {
    pub(super) fn from_nodes(
        nodes: &[PrimNodeReadModel],
        entities: &HashMap<SceneAnchor, Entity>,
    ) -> Self {
        let by_anchor = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.anchor.clone(), index))
            .collect::<HashMap<_, _>>();
        let mut dense_nodes = nodes
            .iter()
            .map(|node| DenseSceneNode {
                entity: entities.get(&node.anchor).copied(),
                anchor: node.anchor.clone(),
                parent: node
                    .parent
                    .as_ref()
                    .and_then(|parent| by_anchor.get(parent).copied()),
                first_child: 0,
                child_count: 0,
                sibling_index: 0,
                label: node.label.clone(),
                display_name: node.display_name.clone(),
                visible: node.visible,
                has_children: node.has_children,
            })
            .collect::<Vec<_>>();

        let by_entity = dense_nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.entity.map(|entity| (entity, index)))
            .collect::<HashMap<_, _>>();
        let mut by_path: HashMap<String, Vec<usize>> = HashMap::new();
        let mut children_by_parent = vec![Vec::new(); dense_nodes.len() + 1];
        for (index, node) in dense_nodes.iter().enumerate() {
            by_path
                .entry(node.anchor.prim_path.clone())
                .or_default()
                .push(index);
            let slot = node.parent.map_or(0, |parent| parent + 1);
            children_by_parent[slot].push(index);
        }
        for children in &mut children_by_parent {
            children.sort_unstable_by(|left, right| {
                dense_nodes[*left].anchor.cmp(&dense_nodes[*right].anchor)
            });
        }

        let mut child_order = Vec::with_capacity(dense_nodes.len());
        let mut child_ranges = Vec::with_capacity(children_by_parent.len());
        for (parent_slot, children) in children_by_parent.into_iter().enumerate() {
            let start = child_order.len();
            for (sibling_index, child) in children.into_iter().enumerate() {
                dense_nodes[child].sibling_index = sibling_index;
                child_order.push(child);
            }
            let end = child_order.len();
            if parent_slot > 0 {
                let parent = parent_slot - 1;
                dense_nodes[parent].first_child = start;
                dense_nodes[parent].child_count = end - start;
                dense_nodes[parent].has_children =
                    dense_nodes[parent].has_children || end > start;
            }
            child_ranges.push(start..end);
        }

        Self {
            nodes: dense_nodes,
            by_anchor,
            by_entity,
            by_path,
            child_ranges,
            child_order,
        }
    }

    pub(super) fn node(&self, index: usize) -> Option<&DenseSceneNode> {
        self.nodes.get(index)
    }

    pub(super) fn children(&self, parent: Option<usize>) -> &[usize] {
        let range = match parent {
            Some(parent) => self
                .node(parent)
                .map(|node| node.first_child..node.first_child.saturating_add(node.child_count)),
            None => self.child_ranges.first().cloned(),
        };
        range
            .map(|range| &self.child_order[range])
            .unwrap_or_default()
    }

    pub(super) fn protocol_node(&self, index: usize) -> Option<PrimNodeReadModel> {
        let node = self.node(index)?;
        Some(PrimNodeReadModel {
            anchor: node.anchor.clone(),
            parent: node
                .parent
                .and_then(|parent| self.node(parent).map(|node| node.anchor.clone())),
            label: node.label.clone(),
            display_name: node.display_name.clone(),
            visible: node.visible,
            has_children: node.has_children,
        })
    }

    pub(super) fn bind_entity(&mut self, anchor: &SceneAnchor, entity: Entity) -> bool {
        let Some(&index) = self.by_anchor.get(anchor) else {
            return false;
        };
        if let Some(previous) = self.nodes[index].entity.replace(entity)
            && previous != entity
        {
            self.by_entity.remove(&previous);
        }
        self.by_entity.insert(entity, index);
        true
    }

    pub(super) fn unbind_entity(&mut self, entity: Entity) {
        if let Some(index) = self.by_entity.remove(&entity)
            && let Some(node) = self.nodes.get_mut(index)
            && node.entity == Some(entity)
        {
            node.entity = None;
        }
    }

    pub(super) fn set_visibility(&mut self, anchor: &SceneAnchor, visible: bool) -> bool {
        let Some(&index) = self.by_anchor.get(anchor) else {
            return false;
        };
        if let Some(node) = self.nodes.get_mut(index) {
            node.visible = visible;
            return true;
        }
        false
    }

    pub(super) fn subtree_anchors(&self, root: &SceneAnchor) -> Vec<SceneAnchor> {
        let Some(&root_index) = self.by_anchor.get(root) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut stack = vec![root_index];
        while let Some(index) = stack.pop() {
            if let Some(node) = self.node(index) {
                out.push(node.anchor.clone());
                stack.extend_from_slice(self.children(Some(index)));
            }
        }
        out
    }

    pub(super) fn anchors_for_prim_path(&self, prim_path: &str) -> Vec<SceneAnchor> {
        self.by_path
            .get(prim_path)
            .into_iter()
            .flat_map(|indices| indices.iter())
            .filter_map(|&index| self.node(index).map(|node| node.anchor.clone()))
            .collect()
    }
}
