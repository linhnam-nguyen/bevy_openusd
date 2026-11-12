use std::{path::Path, time::Duration};

use super::*;

pub(crate) fn cleanup_for_test(queue: &ProjectRuntimeAuthorityQueue, project_root: &Path) {
    cleanup_stale_artifacts(&runtime_root(queue, project_root), Duration::ZERO);
}
