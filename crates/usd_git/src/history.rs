use crate::RevisionId;

/// Metadata for one local Git branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchInfo {
    /// The short branch name, such as `main` or `feature/editor`.
    pub name: String,
    /// The commit currently referenced by this branch.
    pub tip: RevisionId,
    /// Whether this is the branch currently checked out by `HEAD`.
    pub is_current: bool,
}

/// A Git author or committer identity and timestamp.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitSignature {
    pub name: String,
    pub email: String,
    pub time_seconds: i64,
    pub time_offset_seconds: i32,
}

/// Metadata for one immutable Git commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitInfo {
    pub id: RevisionId,
    pub tree_id: RevisionId,
    pub parents: Vec<RevisionId>,
    pub author: CommitSignature,
    pub committer: CommitSignature,
    pub message: String,
}
