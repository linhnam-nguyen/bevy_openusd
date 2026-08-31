use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use viewport_protocol::{
    HierarchyNodeId, HierarchyNodeKind, HierarchyNodeReadModel, HierarchyNodeVisibility,
    HierarchyReadModel, HierarchySource, HierarchyVisibilityState, ViewportEvent,
};

use super::hierarchy::{CurrentHierarchyProjection, HierarchyPageIndex, HierarchyVisibilityTarget};
use super::scene_index::SceneAnchorIndex;

impl CurrentHierarchyProjection {
    pub(crate) fn visibility_targets(
        &self,
        node_id: &HierarchyNodeId,
    ) -> Option<&[HierarchyVisibilityTarget]> {
        self.visibility_index.targets_for(node_id)
    }

    /// Refreshes only the presentation visibility overlay. Semantic grouping
    /// and the private action index remain unchanged across visibility clicks.
    pub(crate) fn refresh_visibility<F>(&mut self, visibility_for: F)
    where
        F: Fn(&HierarchyVisibilityTarget) -> HierarchyVisibilityState,
    {
        let mut read_model = (*self.read_model).clone();
        let mut states = HashMap::with_capacity(read_model.nodes.len());
        for node in &mut read_model.nodes {
            if node.kind == HierarchyNodeKind::Object
                && let Some(targets) = self.visibility_index.targets_for(&node.id)
            {
                let state =
                    aggregate_visibility(targets.iter().map(|target| visibility_for(target)));
                node.set_visibility(state);
                states.insert(node.id.clone(), state);
            }
        }
        recompute_group_visibility(&mut read_model.nodes, &mut states);
        self.install_read_model(read_model);
    }

    /// Applies a provider-neutral visibility request to the projected rows and
    /// returns the bounded authoritative event payload for the client.
    pub(crate) fn apply_visibility(
        &mut self,
        target: &HierarchyNodeId,
        requested: bool,
    ) -> Option<ViewportEvent> {
        if !self.read_model.nodes.iter().any(|node| &node.id == target) {
            return None;
        }

        let affected = match self.source() {
            HierarchySource::Prim => descendant_ids(&self.read_model.nodes, target),
            HierarchySource::BimClassification => {
                let members = self.visibility_targets(target)?;
                let paths = members
                    .iter()
                    .filter_map(|member| match member {
                        HierarchyVisibilityTarget::PrimPath(path) => Some(path.as_str()),
                        HierarchyVisibilityTarget::SceneAnchor(_) => None,
                    })
                    .collect::<HashSet<_>>();
                self.visibility_index
                    .nodes_for_prim_paths(paths.into_iter())
                    .cloned()
                    .collect()
            }
        };
        let requested_state = HierarchyVisibilityState::from_visible(requested);
        let mut read_model = (*self.read_model).clone();
        let mut states = HashMap::with_capacity(read_model.nodes.len());
        for node in &mut read_model.nodes {
            if affected.contains(&node.id) && node.kind == HierarchyNodeKind::Object {
                node.set_visibility(requested_state);
            }
            states.insert(node.id.clone(), node.visibility);
        }
        recompute_group_visibility(&mut read_model.nodes, &mut states);
        let visibility = read_model
            .nodes
            .iter()
            .find(|node| &node.id == target)
            .map(|node| node.visibility)?;
        let mut ancestors = Vec::new();
        let mut parent = read_model
            .nodes
            .iter()
            .find(|node| &node.id == target)
            .and_then(|node| node.parent_id.clone());
        while let Some(parent_id) = parent {
            let node = read_model.nodes.iter().find(|node| node.id == parent_id)?;
            ancestors.push(HierarchyNodeVisibility {
                node_id: node.id.clone(),
                visibility: node.visibility,
            });
            parent = node.parent_id.clone();
        }
        self.install_read_model(read_model);
        Some(ViewportEvent::HierarchyVisibilityChanged {
            source: self.source(),
            target: target.clone(),
            visibility,
            ancestors,
        })
    }

    fn install_read_model(&mut self, read_model: HierarchyReadModel) {
        let read_model = Arc::new(read_model);
        self.page_index = HierarchyPageIndex::from_read_model(&read_model);
        self.read_model = read_model;
    }
}

pub(crate) fn refresh_projection_visibility(
    projection: &mut CurrentHierarchyProjection,
    scene_index: &SceneAnchorIndex,
) {
    projection.refresh_visibility(|target| match target {
        HierarchyVisibilityTarget::SceneAnchor(anchor) => scene_index.visibility_for_anchor(anchor),
        HierarchyVisibilityTarget::PrimPath(path) => scene_index.visibility_for_prim_path(path),
    });
}

fn aggregate_visibility(
    mut states: impl Iterator<Item = HierarchyVisibilityState>,
) -> HierarchyVisibilityState {
    let Some(first) = states.next() else {
        return HierarchyVisibilityState::Visible;
    };
    if states.all(|state| state == first) {
        first
    } else {
        HierarchyVisibilityState::Mixed
    }
}

fn descendant_ids(
    nodes: &[HierarchyNodeReadModel],
    target: &HierarchyNodeId,
) -> HashSet<HierarchyNodeId> {
    let mut affected = HashSet::from([target.clone()]);
    let mut pending = vec![target.clone()];
    while let Some(parent) = pending.pop() {
        for node in nodes {
            if node.parent_id.as_ref() == Some(&parent) && affected.insert(node.id.clone()) {
                pending.push(node.id.clone());
            }
        }
    }
    affected
}

fn recompute_group_visibility(
    nodes: &mut [HierarchyNodeReadModel],
    states: &mut HashMap<HierarchyNodeId, HierarchyVisibilityState>,
) {
    let mut ordered = (0..nodes.len()).collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|index| {
        std::cmp::Reverse(nodes[*index].breadcrumb.matches(" /").count())
    });
    for index in ordered {
        if !nodes[index].has_children {
            continue;
        }
        let child_states = nodes
            .iter()
            .filter(|node| node.parent_id.as_ref() == Some(&nodes[index].id))
            .filter_map(|node| states.get(&node.id).copied());
        let state = aggregate_visibility(child_states);
        nodes[index].set_visibility(state);
        states.insert(nodes[index].id.clone(), state);
    }
}
