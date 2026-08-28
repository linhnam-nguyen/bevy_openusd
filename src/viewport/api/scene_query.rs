//! Asynchronous queries over the current in-memory hierarchy projection.
//!
//! Scene paging is cheap and stays on the ECS thread. Hierarchy search is moved
//! to one worker so bursts of keystrokes cannot make the render schedule wait
//! on a full-projection scan. A one-slot mailbox keeps only the most recent
//! pending query and result.

use std::sync::{Arc, Condvar, Mutex};

use bevy::prelude::Resource;
use usd_model::SemanticSnapshot;
use viewport_protocol::{
    BimSearchQuery, BimSearchResult as ProtocolBimSearchResult, DEFAULT_SCENE_PAGE_SIZE,
    HierarchyNodeId, HierarchyNodeReadModel, HierarchyReadModel,
    HierarchySearchMatch as ProtocolHierarchySearchMatch, HierarchySource,
    MAX_SCENE_SEARCH_RESULTS, SceneAnchor, ScenePageReference, SceneSearchMatch,
};

#[path = "scene_query_generic.rs"]
mod generic;
use generic::search_hierarchy_generic;

#[derive(Debug)]
pub(super) struct LatestMailboxState<T> {
    pending: Option<T>,
    closed: bool,
}

impl<T> Default for LatestMailboxState<T> {
    fn default() -> Self {
        Self {
            pending: None,
            closed: false,
        }
    }
}

#[derive(Debug)]
pub(super) struct LatestMailbox<T> {
    state: Mutex<LatestMailboxState<T>>,
    wake: Condvar,
}

impl<T> LatestMailbox<T> {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(LatestMailboxState::default()),
            wake: Condvar::new(),
        }
    }

    pub(super) fn replace(&self, value: T) -> Result<(), T> {
        let Ok(mut state) = self.state.lock() else {
            return Err(value);
        };
        if state.closed {
            return Err(value);
        }
        state.pending = Some(value);
        self.wake.notify_one();
        Ok(())
    }

    pub(super) fn pop(&self) -> Option<T> {
        let mut state = self.state.lock().ok()?;
        loop {
            if let Some(value) = state.pending.take() {
                return Some(value);
            }
            if state.closed {
                return None;
            }
            state = self.wake.wait(state).ok()?;
        }
    }

    pub(super) fn take(&self) -> Option<T> {
        self.state.lock().ok()?.pending.take()
    }

    pub(super) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.pending = None;
            state.closed = true;
            self.wake.notify_all();
        }
    }
}

#[derive(Debug)]
struct HierarchySearchJob {
    request_id: String,
    query: String,
    offset: u32,
    limit: u32,
    hierarchy: Arc<HierarchyReadModel>,
    source: HierarchySource,
    generic: bool,
}

#[derive(Debug)]
struct BimSearchJob {
    request_id: String,
    query: BimSearchQuery,
    snapshot: Arc<SemanticSnapshot>,
}

#[derive(Debug)]
enum SearchJob {
    Hierarchy(HierarchySearchJob),
    Bim(BimSearchJob),
}

#[derive(Debug)]
pub(crate) struct HierarchySearchMatch {
    pub(crate) node_id: HierarchyNodeId,
    pub(crate) name: String,
    pub(crate) breadcrumb: String,
    pub(crate) anchor: Option<SceneAnchor>,
    pub(crate) parent: Option<SceneAnchor>,
    pub(crate) visible: bool,
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
            visible: self.visible,
            has_children: self.has_children,
            reveal_pages: self.reveal_pages,
        })
    }
}

#[derive(Debug)]
pub(crate) enum SearchMatches {
    Scene(Vec<HierarchySearchMatch>),
    Generic(Vec<ProtocolHierarchySearchMatch>),
}

#[derive(Debug)]
pub(crate) enum SearchResult {
    Hierarchy {
        request_id: String,
        query: String,
        offset: u32,
        total: u32,
        source: HierarchySource,
        matches: SearchMatches,
        has_more: bool,
    },
    Bim {
        request_id: String,
        result: Result<ProtocolBimSearchResult, String>,
    },
}

#[derive(Resource, Debug)]
pub(crate) struct SceneQueryService {
    jobs: Arc<LatestMailbox<SearchJob>>,
    results: Arc<LatestMailbox<SearchResult>>,
}

impl Default for SceneQueryService {
    fn default() -> Self {
        let jobs = Arc::new(LatestMailbox::new());
        let results = Arc::new(LatestMailbox::new());
        let worker_jobs = Arc::clone(&jobs);
        let worker_results = Arc::clone(&results);

        std::thread::Builder::new()
            .name("usdview-scene-search".to_owned())
            .spawn(move || search_worker(worker_jobs, worker_results))
            .expect("scene search worker should start");

        Self { jobs, results }
    }
}

impl SceneQueryService {
    pub(crate) fn submit_search(
        &self,
        request_id: String,
        query: String,
        offset: u32,
        limit: u32,
        hierarchy: Arc<HierarchyReadModel>,
        source: HierarchySource,
        generic: bool,
    ) -> bool {
        self.jobs
            .replace(SearchJob::Hierarchy(HierarchySearchJob {
                request_id,
                query,
                offset,
                limit,
                hierarchy,
                source,
                generic,
            }))
            .is_ok()
    }

    pub(crate) fn submit_bim_search(
        &self,
        request_id: String,
        query: BimSearchQuery,
        snapshot: Arc<SemanticSnapshot>,
    ) -> bool {
        self.jobs
            .replace(SearchJob::Bim(BimSearchJob {
                request_id,
                query,
                snapshot,
            }))
            .is_ok()
    }

    pub(crate) fn drain_results(&self) -> Vec<SearchResult> {
        self.results.take().into_iter().collect()
    }
}

impl Drop for SceneQueryService {
    fn drop(&mut self) {
        self.jobs.close();
        self.results.close();
    }
}

fn search_worker(
    pending_jobs: Arc<LatestMailbox<SearchJob>>,
    results: Arc<LatestMailbox<SearchResult>>,
) {
    while let Some(job) = pending_jobs.pop() {
        let result = match job {
            SearchJob::Hierarchy(job) => {
                let (total, matches) = if job.generic {
                    let (total, matches) =
                        search_hierarchy_generic(&job.hierarchy, &job.query, job.offset, job.limit);
                    (total, SearchMatches::Generic(matches))
                } else {
                    let (total, matches) =
                        search_hierarchy(&job.hierarchy, &job.query, job.offset, job.limit);
                    (total, SearchMatches::Scene(matches))
                };
                let match_count = match &matches {
                    SearchMatches::Scene(matches) => matches.len(),
                    SearchMatches::Generic(matches) => matches.len(),
                };
                SearchResult::Hierarchy {
                    request_id: job.request_id,
                    query: job.query,
                    offset: job.offset,
                    total,
                    source: job.source,
                    matches,
                    has_more: job.offset.saturating_add(match_count as u32) < total,
                }
            }
            SearchJob::Bim(job) => SearchResult::Bim {
                request_id: job.request_id,
                result: crate::viewport::bim::BimReadService::new(&job.snapshot)
                    .search(&job.query)
                    .map_err(|error| error.to_string()),
            },
        };
        if results.replace(result).is_err() {
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
        .collect::<std::collections::HashMap<_, _>>();

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
            visible: node.visible,
            has_children: node.has_children,
            reveal_pages: reveal_pages(node, hierarchy, &by_id),
        })
        .collect();

    (total, matches)
}

fn substring_name_matches(name: &str, query: &[char]) -> bool {
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
    by_id: &std::collections::HashMap<&HierarchyNodeId, &HierarchyNodeReadModel>,
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

fn sibling_page(node: &HierarchyNodeReadModel, hierarchy: &HierarchyReadModel) -> u32 {
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
