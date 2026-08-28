//! Git history and revision materialization for USDHub.
//!
//! The crate intentionally exposes a small Git-neutral API.  `gix` remains an
//! implementation detail so the rest of USDHub does not need to depend on its
//! repository, object, or tree types.

mod commit;
mod error;
mod history;
mod materialize;
mod repository;
mod revision;

pub use commit::CommitRequest;
pub use error::{Error, Result};
pub use history::{BranchInfo, CommitInfo, CommitSignature};
pub use materialize::MaterializedRevision;
pub use repository::{
    BranchName, BranchSwitchOutcome, GitRepository, Repository, WorkingTreeStatus,
};
pub use revision::{Revision, RevisionId, RevisionSpec};
