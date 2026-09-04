use std::collections::HashMap;

use usd_diff::{DiffConfig, RecreationReason, compare, compare_with_config};
use usd_model::{
    Bounds3, CanonicalValue, ChangeFlags, EntityKey, EntitySnapshot, GeometrySignature, HashDigest,
    IdentitySource, MeasurementMetadata, QuantizedPoint3, SemanticInfo, SemanticProperty,
    SemanticSnapshot, SnapshotId, SnapshotSource, TransformSignature,
};

fn digest(seed: u8) -> HashDigest {
    HashDigest::new([seed; HashDigest::BYTE_LEN])
}

fn entity(key: &str, path: &str) -> EntitySnapshot {
    EntitySnapshot {
        key: EntityKey::from(key),
        prim_path: path.to_owned(),
        identity_source: IdentitySource::Synthetic,
        semantic: SemanticInfo::default(),
        transform: TransformSignature {
            translation_mm: [0, 0, 0],
            rotation_quantized: [0, 0, 0, 1_000_000],
            scale_quantized: [1_000_000; 3],
            hash: digest(10),
        },
        geometry: Some(GeometrySignature {
            vertex_count: 8,
            index_count: 36,
            local_bounds: Bounds3 {
                min: [-1.0, -1.0, -1.0],
                max: [1.0, 1.0, 1.0],
            },
            local_centroid: QuantizedPoint3([0, 0, 0]),
            topology_hash: digest(20),
            shape_hash: digest(21),
            render_blob: None,
        }),
        properties: vec![SemanticProperty {
            name: "Comments".to_owned(),
            value: CanonicalValue::Text("A".to_owned()),
            measurement: None,
            display_name: None,
        }],
        metadata_hash: digest(30),
        full_hash: digest(40),
    }
}

fn snapshot(id: &str, entities: impl IntoIterator<Item = EntitySnapshot>) -> SemanticSnapshot {
    let entities = entities
        .into_iter()
        .map(|entity| (entity.key.clone(), entity))
        .collect::<HashMap<_, _>>();
    SemanticSnapshot {
        snapshot_id: SnapshotId(id.to_owned()),
        source: SnapshotSource::Working {
            session: "test".to_owned(),
            live_revision: 1,
        },
        config_hash: digest(1),
        entities,
    }
}

fn changed(mut entity: EntitySnapshot, full_hash: u8) -> EntitySnapshot {
    entity.full_hash = digest(full_hash);
    entity
}

#[test]
fn diff_matrix_classifies_core_actions() {
    let baseline_entity = entity("wall", "/World/Wall");

    let unchanged = compare(
        &snapshot("base", [baseline_entity.clone()]),
        &snapshot("same", [baseline_entity.clone()]),
    );
    let unchanged_diff = unchanged.entity(&EntityKey::from("wall")).unwrap();
    assert_eq!(unchanged_diff.presence, usd_model::PresenceState::Existing);
    assert!(unchanged_diff.flags.is_empty());
    assert_eq!(unchanged.summary.unchanged, 1);

    let mut moved = baseline_entity.clone();
    moved.transform.translation_mm[0] = 125;
    moved.transform.hash = digest(11);
    let moved = changed(moved, 41);
    let moved_diff = compare(
        &snapshot("base", [baseline_entity.clone()]),
        &snapshot("moved", [moved]),
    );
    assert_eq!(
        moved_diff.entity(&EntityKey::from("wall")).unwrap().flags,
        ChangeFlags::TRANSFORM
    );

    let mut extended = baseline_entity.clone();
    extended.geometry.as_mut().unwrap().vertex_count += 4;
    extended.geometry.as_mut().unwrap().shape_hash = digest(22);
    let extended = changed(extended, 42);
    let extended_diff = compare(
        &snapshot("base", [baseline_entity.clone()]),
        &snapshot("extended", [extended]),
    );
    assert_eq!(
        extended_diff
            .entity(&EntityKey::from("wall"))
            .unwrap()
            .flags,
        ChangeFlags::GEOMETRY
    );

    let mut metadata = baseline_entity.clone();
    metadata.properties[0].value = CanonicalValue::Text("B".to_owned());
    metadata.metadata_hash = digest(31);
    let metadata = changed(metadata, 43);
    let metadata_diff = compare(
        &snapshot("base", [baseline_entity.clone()]),
        &snapshot("metadata", [metadata]),
    );
    let metadata_entity = metadata_diff.entity(&EntityKey::from("wall")).unwrap();
    assert_eq!(metadata_entity.flags, ChangeFlags::METADATA);
    assert_eq!(metadata_entity.metadata_changes.len(), 1);
    assert_eq!(
        metadata_entity.metadata_changes[0].name,
        "property.Comments"
    );

    let mut type_swap = baseline_entity.clone();
    type_swap.semantic.type_name = Some("Door".to_owned());
    type_swap.geometry.as_mut().unwrap().shape_hash = digest(23);
    type_swap.metadata_hash = digest(32);
    let type_swap = changed(type_swap, 44);
    let type_swap_diff = compare(
        &snapshot("base", [baseline_entity.clone()]),
        &snapshot("type-swap", [type_swap]),
    );
    assert_eq!(
        type_swap_diff
            .entity(&EntityKey::from("wall"))
            .unwrap()
            .flags,
        ChangeFlags::GEOMETRY | ChangeFlags::METADATA
    );

    let mut hierarchy_move = baseline_entity.clone();
    hierarchy_move.prim_path = "/World/Interior/Wall".to_owned();
    let hierarchy_move = changed(hierarchy_move, 45);
    let hierarchy_diff = compare(
        &snapshot("base", [baseline_entity.clone()]),
        &snapshot("hierarchy", [hierarchy_move]),
    );
    assert_eq!(
        hierarchy_diff
            .entity(&EntityKey::from("wall"))
            .unwrap()
            .flags,
        ChangeFlags::PATH
    );

    let mut compound = baseline_entity.clone();
    compound.transform.translation_mm[1] = 75;
    compound.transform.hash = digest(12);
    compound.properties[0].value = CanonicalValue::Text("C".to_owned());
    compound.metadata_hash = digest(33);
    let compound = changed(compound, 46);
    let compound_diff = compare(
        &snapshot("base", [baseline_entity]),
        &snapshot("compound", [compound]),
    );
    assert_eq!(
        compound_diff
            .entity(&EntityKey::from("wall"))
            .unwrap()
            .flags,
        ChangeFlags::TRANSFORM | ChangeFlags::METADATA
    );
}

#[test]
fn additions_and_removals_remain_separate_identity_changes() {
    let removed = entity("removed", "/World/Removed");
    let added = entity("added", "/World/Added");
    let diff = compare(&snapshot("base", [removed]), &snapshot("current", [added]));

    assert_eq!(diff.summary.removed, 1);
    assert_eq!(diff.summary.added, 1);
    assert_eq!(diff.recreations.len(), 1);
    assert_eq!(diff.recreations[0].removed, EntityKey::from("removed"));
    assert_eq!(diff.recreations[0].added, EntityKey::from("added"));
    assert_eq!(
        diff.entity(&EntityKey::from("removed")).unwrap().presence,
        usd_model::PresenceState::Removed
    );
    assert_eq!(
        diff.entity(&EntityKey::from("added")).unwrap().presence,
        usd_model::PresenceState::Added
    );
}

#[test]
fn recreation_candidates_include_confidence_reasons_and_preserve_details() {
    let mut removed = entity("old-door", "/World/OldDoor");
    removed.semantic = SemanticInfo {
        category: Some("Doors".to_owned()),
        family: Some("Single".to_owned()),
        type_name: Some("Door".to_owned()),
        type_id: Some("door-type".to_owned()),
        display_name: Some("Old door".to_owned()),
        bim: Default::default(),
        bim_classification: Default::default(),
    };

    let mut added = removed.clone();
    added.key = EntityKey::from("new-door");
    added.prim_path = "/World/NewDoor".to_owned();
    added.semantic.display_name = Some("New door".to_owned());

    let diff = compare(
        &snapshot("base", [removed.clone()]),
        &snapshot("current", [added.clone()]),
    );
    let candidate = &diff.recreations[0];

    assert_eq!(candidate.score, 100);
    assert_eq!(
        candidate.reasons,
        vec![
            RecreationReason::SameCategory,
            RecreationReason::SameFamily,
            RecreationReason::SameType,
            RecreationReason::SimilarTransform,
            RecreationReason::SimilarGeometry,
        ]
    );
    assert_eq!(
        diff.entity(&EntityKey::from("old-door")).unwrap().old,
        Some(removed)
    );
    assert_eq!(
        diff.entity(&EntityKey::from("new-door")).unwrap().new,
        Some(added)
    );
}

#[test]
fn unrelated_one_sided_entities_do_not_create_a_candidate() {
    let mut removed = entity("old", "/World/Old");
    removed.semantic.category = Some("Walls".to_owned());
    removed.semantic.type_name = Some("Wall".to_owned());

    let mut added = entity("new", "/World/New");
    added.semantic.category = Some("Furniture".to_owned());
    added.semantic.type_name = Some("Chair".to_owned());
    added.transform.translation_mm = [10_000, 0, 0];
    added.geometry.as_mut().unwrap().shape_hash = digest(99);

    let diff = compare(&snapshot("base", [removed]), &snapshot("current", [added]));

    assert!(diff.recreations.is_empty());
}

#[test]
fn tiny_float_noise_does_not_change_geometry_classification() {
    let baseline = entity("wall", "/World/Wall");
    let mut noisy = baseline.clone();
    noisy.geometry.as_mut().unwrap().local_bounds.max[0] += f64::EPSILON;
    // The full hash changes because the extractor currently serializes raw
    // bounds, so this also exercises geometry's canonical deep comparison.
    noisy.full_hash = digest(48);
    let diff = compare(&snapshot("base", [baseline]), &snapshot("noise", [noisy]));

    assert!(
        diff.entity(&EntityKey::from("wall"))
            .unwrap()
            .flags
            .is_empty()
    );
    assert_eq!(diff.summary.unchanged, 1);
}

#[test]
fn metadata_details_can_be_disabled_without_changing_flags() {
    let baseline = entity("wall", "/World/Wall");
    let mut current = baseline.clone();
    current.properties[0].value = CanonicalValue::Text("B".to_owned());
    current.metadata_hash = digest(31);
    current.full_hash = digest(47);
    let diff = compare_with_config(
        &snapshot("base", [baseline]),
        &snapshot("current", [current]),
        DiffConfig {
            collect_metadata_changes: false,
        },
    );
    let entity = diff.entity(&EntityKey::from("wall")).unwrap();

    assert_eq!(entity.flags, ChangeFlags::METADATA);
    assert!(entity.metadata_changes.is_empty());
}

#[test]
fn metadata_details_include_measurement_changes() {
    let baseline = entity("wall", "/World/Wall");
    let mut current = baseline.clone();
    current.properties[0].measurement = Some(MeasurementMetadata::new("length", "m", None));
    current.metadata_hash = digest(49);
    current.full_hash = digest(50);

    let diff = compare(
        &snapshot("base", [baseline]),
        &snapshot("current", [current]),
    );
    let change = &diff
        .entity(&EntityKey::from("wall"))
        .expect("wall diff")
        .metadata_changes[0];

    assert_eq!(change.name, "property.Comments");
    assert_eq!(change.old, Some(CanonicalValue::Text("A".to_owned())));
    assert!(change.old_measurement.is_none());
    assert_eq!(
        change.new_measurement,
        Some(MeasurementMetadata::new("length", "m", None))
    );
}
