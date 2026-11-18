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
            display_name: None,
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

#[test]
fn added_or_deleted_property_marks_existing_entity_modified() {
    let old = entity("revit-42", "/World/Wall", 1, 10);
    let mut new = entity("revit-42", "/World/Wall", 2, 10);
    new.properties.clear();
    new.properties.push(SemanticProperty {
        name: "BIM:Height".to_owned(),
        value: usd_model::CanonicalValue::Integer(4),
        measurement: None,
        display_name: None,
    });
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
        &[SceneAnchor::active_session("/World/Wall")],
    )
    .expect("existing entity has a property diff");
    assert_eq!(diff.status, BimPropertyDiffStatus::Modified);
    assert!(
        diff.properties
            .iter()
            .any(|property| property.status == BimPropertyDiffStatus::Deleted)
    );
    assert!(
        diff.properties
            .iter()
            .any(|property| property.status == BimPropertyDiffStatus::Added)
    );
}
