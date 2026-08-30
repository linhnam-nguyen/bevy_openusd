use anyhow::{Result, bail};
use std::path::Path;
use usd_bevy::LiveRevision;
use usd_git::RevisionId;
use usd_model::SemanticSnapshot;

/// Commit/base state owned by the application project layer.
#[derive(Debug, Default)]
pub(crate) struct CommitState {
    pub(super) base_revision: Option<RevisionId>,
    pub(super) dirty: bool,
    pub(super) committing: bool,
}

impl CommitState {
    pub(crate) fn new(base_revision: Option<RevisionId>) -> Self {
        Self {
            base_revision,
            ..Self::default()
        }
    }

    pub(crate) fn base_revision(&self) -> Option<&RevisionId> {
        self.base_revision.as_ref()
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn is_committing(&self) -> bool {
        self.committing
    }

    pub(crate) fn mark_dirty(&mut self) -> Result<()> {
        if self.committing {
            bail!("a commit is already in progress; write transaction rejected")
        }
        self.dirty = true;
        Ok(())
    }

    pub(crate) fn acquire_lease(
        &mut self,
        expected_live_revision: LiveRevision,
    ) -> Result<CommitLease> {
        if self.committing {
            bail!("a commit is already in progress")
        }
        if !self.dirty {
            bail!("cannot commit a clean runtime state")
        }
        self.committing = true;
        Ok(CommitLease {
            expected_live_revision,
            finalized: false,
        })
    }

    fn finalize_git_commit(&mut self, lease: &mut CommitLease, revision: RevisionId) {
        self.base_revision = Some(revision);
        self.dirty = false;
        lease.finalized = true;
    }

    pub(super) fn release_lease(&mut self) {
        self.committing = false;
    }
}

/// Exclusive authority token for one frozen LiveStage commit attempt.
///
/// The lease is acquired before staging, the LiveStage revision is checked
/// again immediately before Git creates its OID, and Git success is finalized
/// before any derived Turso persistence is attempted.
#[derive(Debug)]
pub(crate) struct CommitLease {
    expected_live_revision: LiveRevision,
    finalized: bool,
}

impl CommitLease {
    pub(crate) fn expected_live_revision(&self) -> LiveRevision {
        self.expected_live_revision
    }

    pub(crate) fn finalize_git_commit(&mut self, state: &mut CommitState, revision: RevisionId) {
        state.finalize_git_commit(self, revision);
    }
}

/// Result of a successful Git commit.
///
/// `semantic_persisted` can be false even though the Git commit succeeded:
/// the semantic snapshot is a rebuildable cache and will be regenerated later.
#[derive(Debug)]
pub(crate) struct CommitOutcome {
    pub(crate) revision: RevisionId,
    pub(crate) snapshot: Option<SemanticSnapshot>,
    pub(crate) semantic_persisted: bool,
    pub(crate) semantic_error: Option<String>,
}

pub(super) fn validate_relative_stage_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("commit stage path must be a non-empty relative path")
    }
    if path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        bail!("commit stage path must not contain parent traversal")
    }
    Ok(())
}
