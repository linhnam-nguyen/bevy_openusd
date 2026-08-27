use std::collections::HashMap;

use crate::SceneId;

/// A cycle found while proposing a Scene-to-Scene placement.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SceneCompositionError {
    #[error("Scene composition cycle rejected for parent {parent} and child {child}")]
    Cycle { parent: SceneId, child: SceneId },
}

/// Indexed Scene-to-Scene composition relationships.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SceneCompositionGraph {
    children: HashMap<SceneId, Vec<SceneId>>,
}

impl SceneCompositionGraph {
    /// Builds one adjacency index from authored Scene relationships.
    pub fn from_edges<I>(edges: I) -> Self
    where
        I: IntoIterator<Item = (SceneId, SceneId)>,
    {
        let mut graph = Self::default();
        for (parent, child) in edges {
            graph.children.entry(parent).or_default().push(child);
        }
        graph
    }

    /// Returns whether adding a parent-to-child placement would create a cycle.
    pub fn would_create_cycle(&self, parent: SceneId, child: SceneId) -> bool {
        parent == child || reaches_target(child, parent, &self.children, &mut HashMap::new())
    }

    /// Adds a placement only when the indexed graph remains acyclic.
    pub fn add_placement(
        &mut self,
        parent: SceneId,
        child: SceneId,
    ) -> Result<(), SceneCompositionError> {
        if self.would_create_cycle(parent, child) {
            return Err(SceneCompositionError::Cycle { parent, child });
        }
        self.children.entry(parent).or_default().push(child);
        Ok(())
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
    Unvisited,
    Visiting,
    Done,
}

fn reaches_target(
    current: SceneId,
    target: SceneId,
    children: &HashMap<SceneId, Vec<SceneId>>,
    colors: &mut HashMap<SceneId, VisitState>,
) -> bool {
    if current == target {
        return true;
    }

    match colors
        .get(&current)
        .copied()
        .unwrap_or(VisitState::Unvisited)
    {
        VisitState::Visiting | VisitState::Done => return false,
        VisitState::Unvisited => {}
    }
    colors.insert(current, VisitState::Visiting);

    if let Some(descendants) = children.get(&current)
        && descendants
            .iter()
            .copied()
            .any(|child| reaches_target(child, target, children, colors))
    {
        return true;
    }

    colors.insert(current, VisitState::Done);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_self_cycle() {
        let scene = SceneId::new_v4();
        let graph = SceneCompositionGraph::default();

        assert!(graph.would_create_cycle(scene, scene));
    }

    #[test]
    fn rejects_direct_two_node_cycle() {
        let first = SceneId::new_v4();
        let second = SceneId::new_v4();
        let graph = SceneCompositionGraph::from_edges([(first, second)]);

        assert!(graph.would_create_cycle(second, first));
    }

    #[test]
    fn rejects_deep_ancestor_cycle() {
        let scenes = (0..1024).map(|_| SceneId::new_v4()).collect::<Vec<_>>();
        let graph =
            SceneCompositionGraph::from_edges(scenes.windows(2).map(|pair| (pair[0], pair[1])));

        assert!(graph.would_create_cycle(*scenes.last().unwrap(), scenes[0]));
    }

    #[test]
    fn accepts_diamond_dag() {
        let root = SceneId::new_v4();
        let left = SceneId::new_v4();
        let right = SceneId::new_v4();
        let leaf = SceneId::new_v4();
        let graph = SceneCompositionGraph::from_edges([
            (root, left),
            (root, right),
            (left, leaf),
            (right, leaf),
        ]);

        assert!(!graph.would_create_cycle(root, leaf));
    }

    #[test]
    fn accepts_repeated_placement_without_back_edge() {
        let parent = SceneId::new_v4();
        let target = SceneId::new_v4();
        let mut graph = SceneCompositionGraph::default();

        graph.add_placement(parent, target).unwrap();
        graph.add_placement(parent, target).unwrap();
    }
}
