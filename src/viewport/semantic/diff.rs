use std::sync::Arc;

use bevy::prelude::Resource;
use usd_diff::{DiffSummary, StageDiff};
use usd_model::{SemanticSnapshot, SnapshotId, SnapshotSource};
use viewport_protocol::{BimPropertyDiffReadModel, SceneAnchor};

/// Working-vs-baseline comparison state for diagnostics and BIM diff reads.
///
/// The BIM property-diff API accepts only a baseline whose source is an
/// explicit Git commit. The manual capture methods remain for the existing
/// diagnostics panel and are intentionally not eligible for BIM diff styling.
#[derive(Resource, Default)]
pub(crate) struct SemanticDiffState {
    baseline: Option<Arc<SemanticSnapshot>>,
    working: Option<Arc<SemanticSnapshot>>,
    session_id: Option<u64>,
    diff: Option<StageDiff>,
}

impl SemanticDiffState {
    pub(crate) fn update_working(
        &mut self,
        session_id: u64,
        snapshot: impl Into<Arc<SemanticSnapshot>>,
    ) {
        let snapshot = snapshot.into();
        if self.session_id != Some(session_id) {
            self.baseline = None;
            self.diff = None;
            self.session_id = Some(session_id);
        }
        self.working = Some(snapshot);
        self.recompute();
    }

    pub(crate) fn capture_baseline(&mut self) -> bool {
        let Some(working) = self.working.as_ref().map(Arc::clone) else {
            return false;
        };
        self.baseline = Some(working);
        self.recompute();
        true
    }

    /// Installs a materialized Git semantic snapshot as the session baseline.
    /// Working snapshots and arbitrary path-derived snapshots are rejected.
    pub(crate) fn set_git_baseline(&mut self, snapshot: SemanticSnapshot) -> bool {
        if !matches!(snapshot.source, SnapshotSource::GitCommit { .. }) {
            return false;
        }
        self.baseline = Some(Arc::new(snapshot));
        self.recompute();
        true
    }

    pub(crate) fn bim_property_diff(
        &self,
        selection: &[SceneAnchor],
    ) -> Option<BimPropertyDiffReadModel> {
        let baseline = self.baseline.as_ref()?;
        let working = self.working.as_ref()?;
        crate::viewport::bim::diff::property_diff(baseline.as_ref(), working.as_ref(), selection)
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
