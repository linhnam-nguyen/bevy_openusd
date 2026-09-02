//! Asynchronous queries over the current in-memory hierarchy projection.
//!
//! Scene paging is cheap and stays on the ECS thread. Hierarchy search is moved
//! to one worker so bursts of keystrokes cannot make the render schedule wait
//! on a full-projection scan. A one-slot mailbox keeps only the most recent
//! pending query and result.

use std::sync::Arc;

use bevy::prelude::Resource;
use usd_model::SemanticSnapshot;
use viewport_protocol::{
    BimSearchQuery, BimSearchResult as ProtocolBimSearchResult, HierarchyReadModel,
    HierarchySearchMatch as ProtocolHierarchySearchMatch, HierarchySource,
};

#[cfg(test)]
use viewport_protocol::{HierarchyNodeId, SceneAnchor};

#[path = "scene_query_generic.rs"]
mod generic;
#[path = "scene_query_hierarchy.rs"]
mod hierarchy;
#[path = "scene_query_paging.rs"]
mod paging;
#[path = "scene_query_projection.rs"]
mod projection;

pub(crate) use hierarchy::{HierarchySearchMatch, search_hierarchy};
pub(crate) use hierarchy::{sibling_page, substring_name_matches};
pub(crate) use paging::LatestMailbox;

#[cfg(test)]
pub(crate) use generic::search_hierarchy_generic;

#[derive(Debug)]
struct HierarchySearchJob {
    request_id: String,
    activation_generation: u64,
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
    activation_generation: u64,
    query: BimSearchQuery,
    snapshot: Arc<SemanticSnapshot>,
    index: Arc<crate::viewport::bim::BimReadIndex>,
}

#[derive(Debug)]
enum SearchJob {
    Hierarchy(HierarchySearchJob),
    Bim(BimSearchJob),
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
        activation_generation: u64,
        query: String,
        offset: u32,
        total: u32,
        source: HierarchySource,
        matches: SearchMatches,
        has_more: bool,
    },
    Bim {
        request_id: String,
        activation_generation: u64,
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
            .spawn(move || projection::search_worker(worker_jobs, worker_results))
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
        activation_generation: u64,
    ) -> bool {
        self.jobs
            .replace(SearchJob::Hierarchy(HierarchySearchJob {
                request_id,
                activation_generation,
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
        index: Arc<crate::viewport::bim::BimReadIndex>,
        activation_generation: u64,
    ) -> bool {
        self.jobs
            .replace(SearchJob::Bim(BimSearchJob {
                request_id,
                activation_generation,
                query,
                snapshot,
                index,
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

#[cfg(test)]
#[path = "scene_query_tests.rs"]
mod tests;
