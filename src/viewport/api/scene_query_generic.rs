use std::collections::HashMap;

use viewport_protocol::{
    HierarchyNodeId, HierarchyNodeReadModel, HierarchyPageReference, HierarchyReadModel,
    HierarchySearchMatch, MAX_SCENE_SEARCH_RESULTS,
};

use super::{sibling_page, substring_name_matches};

/// Searches the same projected node names for the generic hierarchy wire.
/// The public result keeps virtual providers addressable without converting
/// them through a scene-only anchor type.
pub(crate) fn search_hierarchy_generic(
    hierarchy: &HierarchyReadModel,
    query: &str,
    offset: u32,
    limit: u32,
) -> (u32, Vec<HierarchySearchMatch>) {
    let query = query.trim();
    if query.is_empty() {
        return (0, Vec::new());
    }
    let query_chars = query.chars().collect::<Vec<_>>();
    let limit = if limit == 0 {
        MAX_SCENE_SEARCH_RESULTS
    } else {
        limit.min(MAX_SCENE_SEARCH_RESULTS)
    } as usize;
    let by_id = hierarchy
        .nodes
        .iter()
        .map(|node| (&node.id, node))
        .collect::<HashMap<_, _>>();
    let mut matches: Vec<&HierarchyNodeReadModel> = hierarchy
        .nodes
        .iter()
        .filter(|node| substring_name_matches(&node.name, &query_chars))
        .collect();
    matches.sort_by(|left, right| {
        left.breadcrumb
            .cmp(&right.breadcrumb)
            .then_with(|| left.id.0.cmp(&right.id.0))
    });

    let total = matches.len() as u32;
    let matches = matches
        .into_iter()
        .skip(offset as usize)
        .take(limit)
        .map(|node| HierarchySearchMatch {
            node_id: node.id.clone(),
            name: node.name.clone(),
            breadcrumb: node.breadcrumb.clone(),
            anchor: node.anchor.clone(),
            parent_anchor: node.parent_anchor.clone(),
            visible: node.visible,
            visibility: node.visibility,
            has_children: node.has_children,
            reveal_pages: generic_reveal_pages(node, hierarchy, &by_id),
        })
        .collect();

    (total, matches)
}

fn generic_reveal_pages(
    target: &HierarchyNodeReadModel,
    hierarchy: &HierarchyReadModel,
    by_id: &HashMap<&HierarchyNodeId, &HierarchyNodeReadModel>,
) -> Vec<HierarchyPageReference> {
    let mut path = Vec::new();
    let mut current = Some(target);
    while let Some(node) = current {
        path.push(node);
        current = node
            .parent_id
            .as_ref()
            .and_then(|parent| by_id.get(parent).copied());
    }

    path.into_iter()
        .rev()
        .map(|node| HierarchyPageReference {
            parent_id: node.parent_id.clone(),
            page: sibling_page(node, hierarchy),
        })
        .collect()
}
