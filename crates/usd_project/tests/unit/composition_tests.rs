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
    let graph = SceneCompositionGraph::from_edges(scenes.windows(2).map(|pair| (pair[0], pair[1])));

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
