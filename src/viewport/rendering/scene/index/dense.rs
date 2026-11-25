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
}
