//! Asynchronous queries over the current in-memory hierarchy projection.
//!
//! Scene paging is cheap and stays on the ECS thread. Hierarchy search is moved
//! to one worker so bursts of keystrokes cannot make the render schedule wait
//! on a full-projection scan. The worker coalesces queued jobs and keeps only
//! the most recent query before starting a scan.

use std::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender},
};

use bevy::prelude::Resource;
use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, MAX_SCENE_SEARCH_RESULTS, SceneAnchor, ScenePageReference,
    SceneSearchMatch,
};

use super::hierarchy::{HierarchyNode, HierarchyNodeId, HierarchyReadModel};

#[derive(Debug)]
struct SearchJob {
    request_id: String,
    query: String,
    offset: u32,
    limit: u32,
    hierarchy: HierarchyReadModel,
}

#[derive(Debug)]
pub(crate) struct HierarchySearchMatch {
    pub(crate) node_id: HierarchyNodeId,
    pub(crate) name: String,
    pub(crate) breadcrumb: String,
    pub(crate) prim_path: Option<String>,
    pub(crate) anchor: Option<SceneAnchor>,
    pub(crate) parent: Option<SceneAnchor>,
    pub(crate) visible: bool,
    pub(crate) has_children: bool,
    pub(crate) reveal_pages: Vec<ScenePageReference>,
}

impl HierarchySearchMatch {
    pub(crate) fn into_scene_search_match(self) -> Option<SceneSearchMatch> {
        let anchor = self.anchor?;
        let prim_path = self.prim_path?;
        debug_assert_eq!(anchor.prim_path, prim_path);
        Some(SceneSearchMatch {
            anchor,
            parent: self.parent,
            label: self.name,
            breadcrumb: self.breadcrumb,
            visible: self.visible,
            has_children: self.has_children,
            reveal_pages: self.reveal_pages,
        })
    }
}

#[derive(Debug)]
pub(crate) struct SearchResult {
    pub(crate) request_id: String,
    pub(crate) query: String,
    pub(crate) offset: u32,
    pub(crate) total: u32,
    pub(crate) matches: Vec<HierarchySearchMatch>,
    pub(crate) has_more: bool,
}

#[derive(Resource, Debug)]
pub(crate) struct SceneQueryService {
    jobs: Sender<SearchJob>,
    results: Mutex<Receiver<SearchResult>>,
}

impl Default for SceneQueryService {
    fn default() -> Self {
        let (jobs, pending_jobs) = mpsc::channel();
        let (results, pending_results) = mpsc::channel();

        std::thread::Builder::new()
            .name("usdview-scene-search".to_owned())
            .spawn(move || search_worker(pending_jobs, results))
            .expect("scene search worker should start");

        Self {
            jobs,
            results: Mutex::new(pending_results),
        }
    }
}

impl SceneQueryService {
    pub(crate) fn submit_search(
        &self,
        request_id: String,
        query: String,
        offset: u32,
        limit: u32,
        hierarchy: HierarchyReadModel,
    ) -> bool {
        self.jobs
            .send(SearchJob {
                request_id,
                query,
                offset,
                limit,
                hierarchy,
            })
            .is_ok()
    }

    pub(crate) fn drain_results(&self) -> Vec<SearchResult> {
        let Ok(results) = self.results.lock() else {
            return Vec::new();
        };
        results.try_iter().collect()
    }
}

fn search_worker(pending_jobs: Receiver<SearchJob>, results: Sender<SearchResult>) {
    while let Ok(mut job) = pending_jobs.recv() {
        // A fast typist can enqueue several searches before this worker gets
        // scheduled. The UI already uses request IDs, so intermediate jobs
        // can be discarded safely and only the newest query is evaluated.
        while let Ok(newer) = pending_jobs.try_recv() {
            job = newer;
        }

        let (total, matches) = search_hierarchy(&job.hierarchy, &job.query, job.offset, job.limit);
        let has_more = job.offset.saturating_add(matches.len() as u32) < total;
        if results
            .send(SearchResult {
                request_id: job.request_id,
                query: job.query,
                offset: job.offset,
                total,
                matches,
                has_more,
            })
            .is_err()
        {
            break;
        }
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
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return (0, Vec::new());
    }

    let limit = if limit == 0 {
        MAX_SCENE_SEARCH_RESULTS
    } else {
        limit.min(MAX_SCENE_SEARCH_RESULTS)
    } as usize;
    let by_id = hierarchy
        .nodes
        .iter()
        .map(|node| (&node.id, node))
        .collect::<std::collections::HashMap<_, _>>();

    let mut matches: Vec<&HierarchyNode> = hierarchy
        .nodes
        .iter()
        .filter(|node| node.name.to_lowercase() == query)
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
            prim_path: node.prim_path.clone(),
            anchor: node.anchor.clone(),
            parent: node.parent_anchor.clone(),
            visible: node.visible,
            has_children: node.has_children,
            reveal_pages: reveal_pages(node, hierarchy, &by_id),
        })
        .collect();

    (total, matches)
}

fn reveal_pages(
    target: &HierarchyNode,
    hierarchy: &HierarchyReadModel,
    by_id: &std::collections::HashMap<&HierarchyNodeId, &HierarchyNode>,
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

fn sibling_page(node: &HierarchyNode, hierarchy: &HierarchyReadModel) -> u32 {
    let index = hierarchy
        .nodes
        .iter()
        .filter(|candidate| candidate.parent_id == node.parent_id)
        .position(|candidate| candidate.id == node.id)
        .unwrap_or_default();
    (index as u32) / DEFAULT_SCENE_PAGE_SIZE
}

#[cfg(test)]
#[path = "scene_query_tests.rs"]
mod tests;
