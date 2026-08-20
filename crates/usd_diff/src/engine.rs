//! Snapshot reconciliation and diff result types.

use std::collections::{HashMap, HashSet};

use usd_model::{
    ChangeFlags, EntityKey, EntitySnapshot, PresenceState, SemanticSnapshot, SnapshotId,
};

use crate::classification::classify_existing;
use crate::config::DiffConfig;
use crate::metadata::{MetadataChange, metadata_changes};
use crate::recreation::{RecreationCandidate, find_recreations};

/// The complete semantic diff between two snapshots.
#[derive(Clone, Debug, PartialEq)]
pub struct StageDiff {
    pub baseline: SnapshotId,
    pub current: SnapshotId,
    pub entities: HashMap<EntityKey, EntityDiff>,
    pub summary: DiffSummary,
    pub recreations: Vec<RecreationCandidate>,
}

impl StageDiff {
    pub fn entity(&self, key: &EntityKey) -> Option<&EntityDiff> {
        self.entities.get(key)
    }
}

/// The diff for one identity-matched entity or one one-sided entity.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityDiff {
    pub key: EntityKey,
    pub presence: PresenceState,
    pub flags: ChangeFlags,
    pub old: Option<EntitySnapshot>,
    pub new: Option<EntitySnapshot>,
    pub metadata_changes: Vec<MetadataChange>,
}

impl EntityDiff {
    pub fn is_changed(&self) -> bool {
        !self.flags.is_empty()
    }
}

/// Aggregate counts for the entity-level diff.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiffSummary {
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub changed: usize,
    pub transform: usize,
    pub geometry: usize,
    pub metadata: usize,
    pub path: usize,
}

/// Compare two semantic snapshots using stable entity identity.
pub fn compare(baseline: &SemanticSnapshot, current: &SemanticSnapshot) -> StageDiff {
    compare_with_config(baseline, current, DiffConfig::default())
}

/// Compare two semantic snapshots with explicit detail collection settings.
pub fn compare_with_config(
    baseline: &SemanticSnapshot,
    current: &SemanticSnapshot,
    config: DiffConfig,
) -> StageDiff {
    let keys = baseline
        .entities
        .keys()
        .chain(current.entities.keys())
        .cloned()
        .collect::<HashSet<_>>();
    let mut entities = HashMap::with_capacity(keys.len());
    let mut summary = DiffSummary::default();

    for key in keys {
        let old = baseline.entities.get(&key);
        let new = current.entities.get(&key);
        let entity = match (old, new) {
            (Some(old), None) => {
                summary.removed += 1;
                EntityDiff {
                    key: key.clone(),
                    presence: PresenceState::Removed,
                    flags: ChangeFlags::empty(),
                    old: Some(old.clone()),
                    new: None,
                    metadata_changes: Vec::new(),
                }
            }
            (None, Some(new)) => {
                summary.added += 1;
                EntityDiff {
                    key: key.clone(),
                    presence: PresenceState::Added,
                    flags: ChangeFlags::empty(),
                    old: None,
                    new: Some(new.clone()),
                    metadata_changes: Vec::new(),
                }
            }
            (Some(old), Some(new)) => {
                // The full hash is the cheap unchanged fast path. The
                // extractor includes all semantic dimensions in this hash,
                // including the prim path and quantized signatures.
                let flags = if old.full_hash == new.full_hash {
                    ChangeFlags::empty()
                } else {
                    classify_existing(old, new)
                };

                update_summary(&mut summary, flags);
                let metadata_changes =
                    if config.collect_metadata_changes && flags.contains(ChangeFlags::METADATA) {
                        metadata_changes(old, new)
                    } else {
                        Vec::new()
                    };

                EntityDiff {
                    key: key.clone(),
                    presence: PresenceState::Existing,
                    flags,
                    old: Some(old.clone()),
                    new: Some(new.clone()),
                    metadata_changes,
                }
            }
            (None, None) => unreachable!("a key came from at least one snapshot"),
        };

        if entity.presence == PresenceState::Existing && !entity.is_changed() {
            summary.unchanged += 1;
        }
        entities.insert(key, entity);
    }

    let recreations = find_recreations(
        entities
            .values()
            .filter(|entity| entity.presence == PresenceState::Removed)
            .filter_map(|entity| entity.old.as_ref()),
        entities
            .values()
            .filter(|entity| entity.presence == PresenceState::Added)
            .filter_map(|entity| entity.new.as_ref()),
    );

    StageDiff {
        baseline: baseline.snapshot_id.clone(),
        current: current.snapshot_id.clone(),
        entities,
        summary,
        recreations,
    }
}

fn update_summary(summary: &mut DiffSummary, flags: ChangeFlags) {
    if flags.is_empty() {
        return;
    }

    summary.changed += 1;
    summary.transform += usize::from(flags.contains(ChangeFlags::TRANSFORM));
    summary.geometry += usize::from(flags.contains(ChangeFlags::GEOMETRY));
    summary.metadata += usize::from(flags.contains(ChangeFlags::METADATA));
    summary.path += usize::from(flags.contains(ChangeFlags::PATH));
}
