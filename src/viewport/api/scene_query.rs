//! Asynchronous queries over the authoritative scene-anchor index.
//!
//! Scene paging is cheap and stays on the ECS thread. Text search is moved to
//! one worker so bursts of keystrokes cannot make the render schedule wait on
//! a full-scene scan. The worker coalesces queued jobs and keeps only the most
//! recent query before starting a scan.

use std::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender},
};

use bevy::prelude::Resource;
use viewport_protocol::{
    DEFAULT_SCENE_PAGE_SIZE, MAX_SCENE_SEARCH_RESULTS, PrimNodeReadModel, SceneAnchor,
    ScenePageReference, SceneSearchMatch,
};

#[derive(Debug)]
struct SearchJob {
    request_id: String,
    query: String,
    offset: u32,
    limit: u32,
    nodes: Vec<PrimNodeReadModel>,
}

#[derive(Debug)]
pub(crate) struct SearchResult {
    pub(crate) request_id: String,
    pub(crate) query: String,
    pub(crate) offset: u32,
    pub(crate) total: u32,
    pub(crate) matches: Vec<SceneSearchMatch>,
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
        nodes: Vec<PrimNodeReadModel>,
    ) -> bool {
        self.jobs
            .send(SearchJob {
                request_id,
                query,
                offset,
                limit,
                nodes,
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

        let (total, matches) = search_nodes(&job.nodes, &job.query, job.offset, job.limit);
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

fn search_nodes(
    nodes: &[PrimNodeReadModel],
    query: &str,
    offset: u32,
    limit: u32,
) -> (u32, Vec<SceneSearchMatch>) {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return (0, Vec::new());
    }

    let limit = if limit == 0 {
        MAX_SCENE_SEARCH_RESULTS
    } else {
        limit.min(MAX_SCENE_SEARCH_RESULTS)
    } as usize;
    let by_anchor = nodes
        .iter()
        .map(|node| (node.anchor.clone(), node))
        .collect::<std::collections::HashMap<_, _>>();

    let mut ranked: Vec<(&PrimNodeReadModel, u8)> = nodes
        .iter()
        .filter_map(|node| {
            let Some(display_name) = node.display_name.as_deref() else {
                return None;
            };
            let display_name = display_name.to_lowercase();
            let score = if display_name == query {
                Some(0)
            } else if display_name.starts_with(&query) {
                Some(1)
            } else if display_name.contains(&query) {
                Some(2)
            } else {
                None
            }?;
            Some((node, score))
        })
        .collect();
    ranked.sort_by(|(left, left_score), (right, right_score)| {
        left_score
            .cmp(right_score)
            .then_with(|| left.anchor.prim_path.cmp(&right.anchor.prim_path))
    });

    let total = ranked.len() as u32;
    let matches = ranked
        .into_iter()
        .skip(offset as usize)
        .take(limit)
        .map(|(node, _)| SceneSearchMatch {
            anchor: node.anchor.clone(),
            parent: node.parent.clone(),
            label: node.label.clone(),
            visible: node.visible,
            has_children: node.has_children,
            reveal_pages: reveal_pages(node, nodes, &by_anchor),
        })
        .collect();

    (total, matches)
}

fn reveal_pages(
    target: &PrimNodeReadModel,
    nodes: &[PrimNodeReadModel],
    by_anchor: &std::collections::HashMap<SceneAnchor, &PrimNodeReadModel>,
) -> Vec<ScenePageReference> {
    let mut path = Vec::new();
    let mut current = Some(target);
    while let Some(node) = current {
        path.push(node);
        current = node
            .parent
            .as_ref()
            .and_then(|parent| by_anchor.get(parent).copied());
    }

    path.into_iter()
        .rev()
        .map(|node| ScenePageReference {
            parent: node.parent.clone(),
            page: sibling_page(node, nodes),
        })
        .collect()
}

fn sibling_page(node: &PrimNodeReadModel, nodes: &[PrimNodeReadModel]) -> u32 {
    let index = nodes
        .iter()
        .filter(|candidate| candidate.parent.as_ref() == node.parent.as_ref())
        .position(|candidate| candidate.anchor == node.anchor)
        .unwrap_or_default();
    (index as u32) / DEFAULT_SCENE_PAGE_SIZE
}

#[cfg(test)]
mod tests {
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
    fn search_returns_ancestor_pages_for_reveal() {
        let nodes = vec![
            node("/World", None, "World"),
            node("/World/Environment", Some("/World"), "Environment"),
            node(
                "/World/Environment/Door",
                Some("/World/Environment"),
                "Door",
            ),
        ];

        let (total, matches) = search_nodes(&nodes, "door", 0, 10);

        assert_eq!(total, 1);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].anchor.prim_path, "/World/Environment/Door");
        assert_eq!(
            matches[0]
                .reveal_pages
                .iter()
                .map(|page| page.parent.as_ref().map(|parent| parent.prim_path.as_str()))
                .collect::<Vec<_>>(),
            vec![None, Some("/World"), Some("/World/Environment")]
        );
    }

    #[test]
    fn search_ranks_exact_and_paginates_without_changing_total() {
        let nodes = vec![
            node("/World/DoorPanel", None, "DoorPanel"),
            node("/World/Door", None, "Door"),
            node("/World/DoorFrame", None, "Frame"),
        ];

        let (total, first) = search_nodes(&nodes, "door", 0, 2);
        assert_eq!(total, 2);
        assert_eq!(
            first
                .iter()
                .map(|result| result.anchor.prim_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/World/Door", "/World/DoorPanel"]
        );

        let (_, second) = search_nodes(&nodes, "door", 2, 2);
        assert!(second.is_empty());

        let (path_only_total, path_only_matches) = search_nodes(&nodes, "doorframe", 0, 10);
        assert_eq!(path_only_total, 0);
        assert!(path_only_matches.is_empty());
    }
}
