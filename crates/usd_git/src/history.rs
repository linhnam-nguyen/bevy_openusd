use crate::RevisionId;

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
