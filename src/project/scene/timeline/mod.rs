//! Canonical playback-timeline aggregation, ancestor propagation, and legacy repair.
//!
//! OpenUSD references do not inherit a child stage's root `startTimeCode` or
//! `endTimeCode`. This module maps referenced child playback domains into each
//! parent Scene's time-code domain using OpenUSD's composed rate and
//! `LayerOffset` rules, propagates changes atomically through all affected
//! ancestors up to the Project root, and provides explicit non-destructive
//! repair for legacy wrappers and Scenes.

mod domain;
mod propagation;
mod publication;
mod repair;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use usd_project::{ModelId, ProjectManifestV1, SceneId};

use crate::project::storage::ProjectStorageLayout;

pub(crate) use domain::{
    existing_member_reference_offset, has_intentional_authored_range, map_child_domain_to_parent,
    mark_new_adopted_source_timeline, preserve_linked_scene_timeline, read_member_reference_offset,
    recompute_scene_stage_timeline, root_layer_authored_time_sample_range,
};
pub(crate) use propagation::{
    PreparedAncestorTimelines, affected_ancestors_child_first, prepare_ancestor_timelines,
    prepare_scene_member_timelines, propagate_ancestor_timelines_atomic,
    retime_scene_member_atomic, save_scene_with_timelines_atomic,
};
pub(crate) use publication::{
    capture_ancestor_preimages, restore_ancestor_preimages, restore_timeline_preimage,
    sync_file_durable,
};
pub(super) use publication::publish_staged_layers_atomic;
pub(crate) use repair::{inspect_project_timeline_repairs, repair_project_timelines_atomic};

pub(super) const TIMELINE_RANGE_MODE_METADATA: &str = "usdhub:timelineRangeMode";
pub(super) const TIMELINE_RANGE_MODE_GENERATED: &str = "generated";
pub(super) const GENERATED_START_METADATA: &str = "usdhub:generatedStartTimeCode";
pub(super) const GENERATED_END_METADATA: &str = "usdhub:generatedEndTimeCode";
pub(super) const MODEL_ROOT_PRIM: &str = "ModelRoot";
pub(super) const SOURCE_PRIM: &str = "Source";
pub(super) const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
pub(super) const TRANSACTIONS_DIRECTORY: &str = ".transactions";

/// Staged file overrides used during an in-flight atomic publication so parent
/// aggregation inspects candidate child layers before they are renamed into place
/// and verifies authored reference paths against candidate canonical destinations.
#[derive(Clone, Debug, Default)]
pub(crate) struct TimelineStageOverrides {
    pub scenes: HashMap<SceneId, PathBuf>,
    pub models: HashMap<ModelId, PathBuf>,
    pub scene_targets: HashMap<SceneId, PathBuf>,
    pub model_targets: HashMap<ModelId, PathBuf>,
}

impl TimelineStageOverrides {
    /// Populate expected canonical target paths from a candidate manifest so
    /// reference verification resolves readable paths before the manifest is
    /// written to disk.
    pub(crate) fn populate_manifest_targets(
        &mut self,
        project_root: &Path,
        manifest: &ProjectManifestV1,
    ) {
        let layout = ProjectStorageLayout::new(project_root);
        for scene in &manifest.scenes {
            self.scene_targets
                .entry(scene.id)
                .or_insert_with(|| layout.readable_scene_path(manifest, scene));
        }
        for model in &manifest.models {
            self.model_targets
                .entry(model.id)
                .or_insert_with(|| layout.canonical_model_wrapper_path(model));
        }
    }
}

/// Result of recomputing a parent Scene's timeline metadata.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TimelineUpdateOutcome {
    /// Intentional user-authored playback bounds were preserved untouched.
    PreservedUserRange,
    /// Generated playback range was updated (or cleared when `None`).
    Updated(Option<crate::document::stages::metadata::TimeCodeRange>),
    /// Generated metadata already matched the computed domain.
    Unchanged(Option<crate::document::stages::metadata::TimeCodeRange>),
}

/// Target identified by legacy timeline inspection or repair.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TimelineRepairTarget {
    Model(ModelId),
    Scene(SceneId),
}

/// One proposed non-destructive metadata repair entry.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineRepairEntry {
    pub target: TimelineRepairTarget,
    pub layer_path: PathBuf,
    pub current_range: Option<(f64, f64)>,
    pub proposed_range: Option<(f64, f64)>,
    pub current_time_codes_per_second: f64,
    pub proposed_time_codes_per_second: f64,
    pub proposed_frames_per_second: Option<f64>,
    pub repair_range: bool,
    pub repair_time_codes_per_second: bool,
    pub repair_frames_per_second: bool,
    pub reason: String,
}

/// Read-only inspection plan describing legacy layers missing timeline metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectTimelineRepairPlan {
    pub entries: Vec<TimelineRepairEntry>,
}

impl ProjectTimelineRepairPlan {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Summary returned after an explicit transactional timeline repair.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectTimelineRepairReport {
    pub repaired_models: Vec<ModelId>,
    pub repaired_scenes: Vec<SceneId>,
}
