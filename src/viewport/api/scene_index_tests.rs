use super::*;

fn node(path: &str, parent: Option<&str>, label: &str) -> PrimNodeReadModel {
    PrimNodeReadModel {
        anchor: SceneAnchor::active_session(path),
        parent: parent.map(SceneAnchor::active_session),
        label: label.to_owned(),
        display_name: Some(label.to_owned()),
        visible: true,
        has_children: false,
    }
}

#[test]
fn semantic_path_resolution_preserves_runtime_reveal_pages() {
    let index = SceneAnchorIndex {
        nodes: vec![
            node("/World", None, "World"),
            node("/World/Environment", Some("/World"), "Environment"),
            node(
                "/World/Environment/Door",
                Some("/World/Environment"),
                "Door",
            ),
        ],
        ..Default::default()
    };

    let result = index
        .search_match_for_path("/World/Environment/Door")
        .expect("semantic row resolves to a runtime node");
    assert_eq!(result.label, "Door");
    assert_eq!(result.reveal_pages.len(), 3);
    assert_eq!(result.reveal_pages[0].parent, None);
    assert_eq!(
        result.reveal_pages[1]
            .parent
            .as_ref()
            .map(|anchor| anchor.prim_path.as_str()),
        Some("/World")
    );
    assert_eq!(
        result.reveal_pages[2]
            .parent
            .as_ref()
            .map(|anchor| anchor.prim_path.as_str()),
        Some("/World/Environment")
    );
}

#[test]
fn prim_name_projection_uses_only_the_final_path_segment() {
    assert_eq!(prim_name("/root/name1/name2/name3"), "name3");
    assert_eq!(prim_name("/Architecture/Level01/Wall_0042"), "Wall_0042");
}
