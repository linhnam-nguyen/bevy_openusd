use std::path::PathBuf;

use crate::RevisionId;

/// Result of materializing a complete Git tree into an isolated directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedRevision {
    pub revision: RevisionId,
    pub root: PathBuf,
    pub file_count: usize,
}
