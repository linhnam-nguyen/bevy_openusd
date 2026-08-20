use bevy::prelude::Resource;
use usd_diff::{DiffSummary, StageDiff};
use usd_model::{SemanticSnapshot, SnapshotId};

/// Manual working-vs-baseline comparison state for diagnostics.
///
/// The baseline is intentionally an in-memory snapshot. Git-backed baselines
/// are introduced by the later `usd_git` milestone; this resource only makes
/// the current live semantic snapshot observable through `usd_diff`.
#[derive(Resource, Default)]
pub(crate) struct SemanticDiffState {
    baseline: Option<SemanticSnapshot>,
    working: Option<SemanticSnapshot>,
    session_id: Option<u64>,
    diff: Option<StageDiff>,
}

impl SemanticDiffState {
    pub(crate) fn update_working(&mut self, session_id: u64, snapshot: SemanticSnapshot) {
        if self.session_id != Some(session_id) {
            self.baseline = None;
            self.diff = None;
            self.session_id = Some(session_id);
        }
        self.working = Some(snapshot);
        self.recompute();
    }

    pub(crate) fn capture_baseline(&mut self) -> bool {
        let Some(working) = self.working.clone() else {
            return false;
        };
        self.baseline = Some(working);
        self.recompute();
        true
    }

    pub(crate) fn clear_baseline(&mut self) {
        self.baseline = None;
        self.diff = None;
    }

    pub(crate) fn has_working_snapshot(&self) -> bool {
        self.working.is_some()
    }

    pub(crate) fn has_baseline(&self) -> bool {
        self.baseline.is_some()
    }

    pub(crate) fn summary(&self) -> Option<DiffSummary> {
        self.diff.as_ref().map(|diff| diff.summary)
    }

    pub(crate) fn stage_diff(&self) -> Option<&StageDiff> {
        self.diff.as_ref()
    }

    pub(crate) fn baseline_snapshot_id(&self) -> Option<&SnapshotId> {
        self.baseline.as_ref().map(|snapshot| &snapshot.snapshot_id)
    }

    fn recompute(&mut self) {
        self.diff = self
            .baseline
            .as_ref()
            .zip(self.working.as_ref())
            .map(|(baseline, working)| usd_diff::compare(baseline, working));
    }
}
