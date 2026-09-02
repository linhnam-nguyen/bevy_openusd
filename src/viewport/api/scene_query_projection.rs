use std::sync::Arc;

use super::generic::search_hierarchy_generic;
use super::paging::LatestMailbox;
use super::search_hierarchy;
use super::{BimSearchJob, HierarchySearchJob, SearchJob, SearchMatches, SearchResult};

pub(super) fn search_worker(
    pending_jobs: Arc<LatestMailbox<SearchJob>>,
    results: Arc<LatestMailbox<SearchResult>>,
) {
    while let Some(job) = pending_jobs.pop() {
        let result = match job {
            SearchJob::Hierarchy(job) => hierarchy_result(job),
            SearchJob::Bim(job) => bim_result(job),
        };
        if results.replace(result).is_err() {
            break;
        }
    }
}

fn hierarchy_result(job: HierarchySearchJob) -> SearchResult {
    let (total, matches) = if job.generic {
        let (total, matches) =
            search_hierarchy_generic(&job.hierarchy, &job.query, job.offset, job.limit);
        (total, SearchMatches::Generic(matches))
    } else {
        let (total, matches) = search_hierarchy(&job.hierarchy, &job.query, job.offset, job.limit);
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

fn bim_result(job: BimSearchJob) -> SearchResult {
    SearchResult::Bim {
        request_id: job.request_id,
        result: crate::viewport::bim::BimReadService::with_index(&job.snapshot, job.index)
            .search(&job.query)
            .map_err(|error| error.to_string()),
    }
}
