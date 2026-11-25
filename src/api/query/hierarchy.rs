use std::collections::HashMap;

use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, HierarchyNodeId, HierarchyNodeReadModel, HierarchyReadModel,
    HierarchyVisibilityState, MAX_SCENE_SEARCH_RESULTS, SceneAnchor, ScenePageReference,
    SceneSearchMatch,
};

#[derive(Debug)]
pub(crate) struct HierarchySearchMatch {
    pub(crate) node_id: HierarchyNodeId,
    pub(crate) name: String,
    pub(crate) breadcrumb: String,
    pub(crate) anchor: Option<SceneAnchor>,
    pub(crate) parent: Option<SceneAnchor>,
    pub(crate) visibility: HierarchyVisibilityState,
    pub(crate) has_children: bool,
    pub(crate) reveal_pages: Vec<ScenePageReference>,
}

impl HierarchySearchMatch {
    pub(crate) fn into_scene_search_match(self) -> Option<SceneSearchMatch> {
        let anchor = self.anchor?;
        Some(SceneSearchMatch {
            anchor,
            parent: self.parent,
            label: self.name,
            breadcrumb: self.breadcrumb,
            visible: self.visibility.is_visible(),
            has_children: self.has_children,
            reveal_pages: self.reveal_pages,
        })
    }
}

/// Searches only the names in the supplied hierarchy projection.
///
/// The projection adapter owns the relationship between a node name and its
/// source data. This function never derives a name from `prim_path`, searches
/// an ancestor breadcrumb, or reads authored USD display metadata.
pub(crate) fn search_hierarchy(
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
            parent: node.parent_anchor.clone(),
            visibility: node.visibility,
            has_children: node.has_children,
            reveal_pages: reveal_pages(node, hierarchy, &by_id),
        })
        .collect();

    (total, matches)
}

pub(crate) fn substring_name_matches(name: &str, query: &[char]) -> bool {
    let name_chars = name.chars().collect::<Vec<_>>();
    if query.is_empty() || query.len() > name_chars.len() {
        return false;
    }

    name_chars
        .windows(query.len())
        .enumerate()
        .any(|(start, window)| {
            let end = start + query.len();
            !matches_numeric_fragment_boundary(&name_chars, start, end)
                && window.iter().zip(query).all(|(name_char, query_char)| {
                    name_char.to_lowercase().eq(query_char.to_lowercase())
                })
        })
}

fn matches_numeric_fragment_boundary(name: &[char], start: usize, end: usize) -> bool {
    (start > 0 && name[start - 1].is_numeric() && name[start].is_numeric())
        || (end < name.len() && name[end - 1].is_numeric() && name[end].is_numeric())
}

fn reveal_pages(
    target: &HierarchyNodeReadModel,
    hierarchy: &HierarchyReadModel,
    by_id: &HashMap<&HierarchyNodeId, &HierarchyNodeReadModel>,
) -> Vec<ScenePageReference> {
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
        .map(|node| ScenePageReference {
            parent: node.parent_anchor.clone(),
            page: sibling_page(node, hierarchy),
        })
        .collect()
}

pub(crate) fn sibling_page(node: &HierarchyNodeReadModel, hierarchy: &HierarchyReadModel) -> u32 {
    let index = hierarchy
        .nodes
        .iter()
        .filter(|candidate| candidate.parent_id == node.parent_id)
        .position(|candidate| candidate.id == node.id)
        .unwrap_or_default();
    (index as u32) / DEFAULT_SCENE_PAGE_SIZE
}
