//! Single-selection semantic property diff projection.
//!
//! This module accepts only an explicit Git-backed baseline. It does not
//! inspect the working tree or infer a historical revision from a display
//! label/path.

use std::collections::{BTreeMap, HashSet};

use usd_model::{EntitySnapshot, SemanticProperty, SemanticSnapshot, SnapshotSource};
use viewport_protocol::{
    BimPropertyDiffReadModel, BimPropertyDiffRow, BimPropertyDiffStatus, SceneAnchor,
};

/// Builds a property diff for exactly one selected target.
///
/// Entity identity comes from `EntityKey` whenever the working entity is
/// present. A prim-path lookup is used only to locate a deleted baseline
/// entity when the working side no longer has an entity to provide its key.
/// The metadata hash avoids the detailed `usd_diff` property pass when no
/// metadata changed.
pub(crate) fn property_diff(
    baseline: &SemanticSnapshot,
    working: &SemanticSnapshot,
    selection: &[SceneAnchor],
) -> Option<BimPropertyDiffReadModel> {
    if selection.len() != 1 {
        return None;
    }
    let target = &selection[0];
    target.validate().ok()?;
    let SnapshotSource::GitCommit { oid: base_git_oid } = &baseline.source else {
        return None;
    };

    let working_entity = working
        .entities
        .values()
        .find(|entity| entity.prim_path == target.prim_path);
    let baseline_entity = working_entity
        .and_then(|entity| baseline.entities.get(&entity.key))
        .or_else(|| {
            working_entity
                .is_none()
                .then(|| {
                    baseline
                        .entities
                        .values()
                        .find(|entity| entity.prim_path == target.prim_path)
                })
                .flatten()
        });

    let (status, properties) = match (baseline_entity, working_entity) {
        (Some(old), Some(new)) => {
            let changed_property_names = if old.metadata_hash == new.metadata_hash {
                HashSet::new()
            } else {
                usd_diff::metadata_changes(old, new)
                    .into_iter()
                    .filter_map(|change| change.name.strip_prefix("property.").map(str::to_owned))
                    .collect::<HashSet<_>>()
            };
            let properties = property_rows(
                old,
                new,
                &changed_property_names,
                BimPropertyDiffStatus::Modified,
            );
            let status = if properties
                .iter()
                .any(|property| property.status != BimPropertyDiffStatus::Unchanged)
            {
                BimPropertyDiffStatus::Modified
            } else {
                BimPropertyDiffStatus::Unchanged
            };
            (status, properties)
        }
        (None, Some(new)) => (
            BimPropertyDiffStatus::Added,
            one_sided_property_rows(new, BimPropertyDiffStatus::Added),
        ),
        (Some(old), None) => (
            BimPropertyDiffStatus::Deleted,
            one_sided_property_rows(old, BimPropertyDiffStatus::Deleted),
        ),
        (None, None) => return None,
    };

    Some(BimPropertyDiffReadModel {
        target: target.clone(),
        base_git_oid: base_git_oid.clone(),
        working_snapshot_id: working.snapshot_id.0.clone(),
        status,
        properties,
    })
}

fn property_rows(
    old: &EntitySnapshot,
    new: &EntitySnapshot,
    changed_property_names: &HashSet<String>,
    modified_status: BimPropertyDiffStatus,
) -> Vec<BimPropertyDiffRow> {
    let mut properties: BTreeMap<String, (Option<&SemanticProperty>, Option<&SemanticProperty>)> =
        BTreeMap::new();
    for property in &old.properties {
        properties.entry(property.name.clone()).or_default().0 = Some(property);
    }
    for property in &new.properties {
        properties.entry(property.name.clone()).or_default().1 = Some(property);
    }
    properties
        .into_iter()
        .map(|(key, (old, new))| match (old, new) {
            (Some(old), Some(new)) => BimPropertyDiffRow {
                key,
                status: if changed_property_names.contains(&old.name) {
                    modified_status
                } else {
                    BimPropertyDiffStatus::Unchanged
                },
                old_value: Some(old.value.clone()),
                new_value: Some(new.value.clone()),
                old_measurement: old.measurement.clone(),
                new_measurement: new.measurement.clone(),
            },
            (Some(old), None) => BimPropertyDiffRow {
                key,
                status: BimPropertyDiffStatus::Deleted,
                old_value: Some(old.value.clone()),
                new_value: None,
                old_measurement: old.measurement.clone(),
                new_measurement: None,
            },
            (None, Some(new)) => BimPropertyDiffRow {
                key,
                status: BimPropertyDiffStatus::Added,
                old_value: None,
                new_value: Some(new.value.clone()),
                old_measurement: None,
                new_measurement: new.measurement.clone(),
            },
            (None, None) => unreachable!("property union contains a populated side"),
        })
        .collect()
}

fn one_sided_property_rows(
    entity: &EntitySnapshot,
    status: BimPropertyDiffStatus,
) -> Vec<BimPropertyDiffRow> {
    entity
        .properties
        .iter()
        .map(|property| match status {
            BimPropertyDiffStatus::Added => BimPropertyDiffRow {
                key: property.name.clone(),
                status,
                old_value: None,
                new_value: Some(property.value.clone()),
                old_measurement: None,
                new_measurement: property.measurement.clone(),
            },
            BimPropertyDiffStatus::Deleted => BimPropertyDiffRow {
                key: property.name.clone(),
                status,
                old_value: Some(property.value.clone()),
                new_value: None,
                old_measurement: property.measurement.clone(),
                new_measurement: None,
            },
            BimPropertyDiffStatus::Unchanged | BimPropertyDiffStatus::Modified => {
                unreachable!("one-sided rows use added/deleted status")
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/bim/diff_inline_tests.rs"]
mod tests;
