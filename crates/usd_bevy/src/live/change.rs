use bevy::prelude::*;
use std::collections::HashSet;

use super::path::{is_descendant_or_self, minimize_resync_roots, normalize_prim_path};

/// One committed stage change, copied out of the borrowed [`openusd::usd::CommittedChange`]
/// so it can outlive the sink callback and be drained on a later frame.
///
/// * `resynced` — composition restructured (define / remove / reparent /
///   variant / reference / layer-mute …); the subtree must be reprojected.
/// * `changed_info` — a field/value/target changed, namespace intact; the
///   corresponding component(s) can be patched in place.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StageChange {
    pub resynced: Vec<String>,
    pub changed_info: Vec<String>,
}

/// Monotonic revision of the in-memory live stage.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LiveRevision(pub u64);

/// One authoritative, once-drained batch of stage changes.
///
/// The batch is retained in [`PendingStageChanges`] for the rest of the
/// frame, so projection, semantic indexing, and diagnostics can all consume
/// the same revision without independently draining [`super::stage::LiveStage`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageChangeBatch {
    pub revision: LiveRevision,
    pub changes: Vec<StageChange>,
}

/// The stage-change batch drained for the current frame.
///
/// This is intentionally a transient fan-out resource, not another model
/// representation. It is replaced by [`drain_stage_changes_system`] before
/// each projection pass and remains readable by later consumers in the same
/// schedule.
#[derive(Resource, Default)]
pub struct PendingStageChanges {
    pub(super) batch: Option<StageChangeBatch>,
}

impl PendingStageChanges {
    pub fn batch(&self) -> Option<&StageChangeBatch> {
        self.batch.as_ref()
    }
}

impl StageChange {
    /// All paths mentioned by this change (resynced ∪ changed-info).
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.resynced.iter().chain(self.changed_info.iter())
    }
}

impl StageChangeBatch {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Returns `true` if any change in the batch contains a resync notice.
    pub fn has_resync(&self) -> bool {
        self.changes.iter().any(|c| !c.resynced.is_empty())
    }

    /// Returns the minimal, boundary-aware resync roots that cover all resynced paths
    /// in this batch.
    pub fn resync_roots(&self) -> Vec<String> {
        let all_resynced = self.changes.iter().flat_map(|c| &c.resynced);
        minimize_resync_roots(all_resynced)
    }

    /// Checks if a given path (prim or property) falls under any resync root in this batch.
    pub fn is_path_under_resync(&self, path: &str) -> bool {
        let roots = self.resync_roots();
        roots.iter().any(|root| is_descendant_or_self(root, path))
    }

    /// Returns all `changed_info` paths from this batch that are outside all resync roots.
    ///
    /// Changes under a resync root are owned by subtree reconciliation and should not be
    /// redundantly sparse-patched.
    pub fn unshaded_changed_info(&self) -> Vec<String> {
        let roots = self.resync_roots();
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        for change in &self.changes {
            for info_path in &change.changed_info {
                let prim_path = normalize_prim_path(info_path);
                let covered = roots
                    .iter()
                    .any(|root| is_descendant_or_self(root, &prim_path));
                if !covered && seen.insert(info_path.clone()) {
                    result.push(info_path.clone());
                }
            }
        }
        result
    }
}
