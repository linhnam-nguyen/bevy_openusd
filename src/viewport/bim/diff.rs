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
                .any(|property| property.status == BimPropertyDiffStatus::Modified)
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
                status: changed_property_names
                    .contains(&old.name)
                    .then_some(modified_status)
                    .unwrap_or(BimPropertyDiffStatus::Unchanged),
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
mod tests {
    use std::collections::HashMap;

    use usd_model::{
        EntityKey, EntitySnapshot, HashDigest, IdentitySource, SemanticInfo, SemanticProperty,
        SnapshotId, SnapshotSource, TransformSignature,
    };

    use super::*;

    fn digest(value: u8) -> HashDigest {
        HashDigest::new([value; 32])
    }

    fn entity(key: &str, path: &str, metadata_hash: u8, value: i64) -> EntitySnapshot {
        EntitySnapshot {
            key: EntityKey::from(key),
            prim_path: path.to_owned(),
            identity_source: IdentitySource::RevitUniqueId,
            semantic: SemanticInfo::default(),
            transform: TransformSignature {
                translation_mm: [0; 3],
                rotation_quantized: [0; 4],
                scale_quantized: [0; 3],
                hash: digest(2),
            },
            geometry: None,
            properties: vec![SemanticProperty {
                name: "BIM:Width".to_owned(),
                value: usd_model::CanonicalValue::Integer(value),
                measurement: None,
            }],
            metadata_hash: digest(metadata_hash),
            full_hash: digest(metadata_hash),
        }
    }

    fn snapshot(source: SnapshotSource, entity: EntitySnapshot) -> SemanticSnapshot {
        SemanticSnapshot {
            snapshot_id: SnapshotId("snapshot".to_owned()),
            source,
            config_hash: digest(99),
            entities: HashMap::from([(entity.key.clone(), entity)]),
        }
    }

    #[test]
    fn uses_entity_key_and_marks_modified_property() {
        let old = entity("revit-42", "/World/Old", 1, 10);
        let new = entity("revit-42", "/World/New", 2, 20);
        let baseline = snapshot(
            SnapshotSource::GitCommit {
                oid: "abc123".into(),
            },
            old,
        );
        let working = snapshot(
            SnapshotSource::Working {
                session: "session".into(),
                live_revision: 3,
            },
            new,
        );

        let diff = property_diff(
            &baseline,
            &working,
            &[SceneAnchor::active_session("/World/New")],
        )
        .expect("one selected entity has a Git baseline");
        assert_eq!(diff.base_git_oid, "abc123");
        assert_eq!(diff.status, BimPropertyDiffStatus::Modified);
        assert_eq!(diff.properties[0].status, BimPropertyDiffStatus::Modified);
    }

    #[test]
    fn equal_metadata_hash_skips_detail_changes_and_multi_selection_is_disabled() {
        let old = entity("revit-42", "/World/Wall", 1, 10);
        let new = entity("revit-42", "/World/Wall", 1, 10);
        let baseline = snapshot(
            SnapshotSource::GitCommit {
                oid: "abc123".into(),
            },
            old,
        );
        let working = snapshot(
            SnapshotSource::Working {
                session: "session".into(),
                live_revision: 3,
            },
            new,
        );
        let selection = [SceneAnchor::active_session("/World/Wall")];
        let diff = property_diff(&baseline, &working, &selection).expect("unchanged diff");
        assert_eq!(diff.status, BimPropertyDiffStatus::Unchanged);
        assert_eq!(diff.properties[0].status, BimPropertyDiffStatus::Unchanged);
        assert!(
            property_diff(
                &baseline,
                &working,
                &[selection[0].clone(), selection[0].clone()]
            )
            .is_none()
        );
    }

    #[test]
    fn missing_working_entity_is_deleted_by_baseline_path() {
        let old = entity("revit-42", "/World/Wall", 1, 10);
        let baseline = snapshot(
            SnapshotSource::GitCommit {
                oid: "abc123".into(),
            },
            old,
        );
        let working = SemanticSnapshot {
            snapshot_id: SnapshotId("working".into()),
            source: SnapshotSource::Working {
                session: "session".into(),
                live_revision: 4,
            },
            config_hash: digest(99),
            entities: HashMap::new(),
        };
        let diff = property_diff(
            &baseline,
            &working,
            &[SceneAnchor::active_session("/World/Wall")],
        )
        .expect("deleted baseline entity is addressable by path");
        assert_eq!(diff.status, BimPropertyDiffStatus::Deleted);
        assert_eq!(diff.properties[0].status, BimPropertyDiffStatus::Deleted);
    }
}
