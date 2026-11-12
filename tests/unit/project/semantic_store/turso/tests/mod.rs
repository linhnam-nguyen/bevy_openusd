mod query;
mod schema;
mod snapshot;

use std::collections::HashMap;

use usd_model::{
    CanonicalValue, EntityKey, EntitySnapshot, HashDigest, IdentitySource, MeasurementMetadata,
    SemanticInfo, SemanticProperty, SemanticSnapshot, SnapshotId, SnapshotSource,
    TransformSignature,
};

pub(super) fn snapshot(oid: &str, snapshot_id: &str, comments: &str, seed: u8) -> SemanticSnapshot {
    let key = EntityKey::from("/World/Wall");
    let entity = EntitySnapshot {
        key: key.clone(),
        prim_path: "/World/Wall".to_owned(),
        identity_source: IdentitySource::PrimPath,
        semantic: SemanticInfo {
            category: Some("Architecture".to_owned()),
            family: Some("Wall".to_owned()),
            type_name: Some("IfcWall".to_owned()),
            type_id: Some("wall-type".to_owned()),
            display_name: Some("Wall".to_owned()),
            bim: Default::default(),
            bim_classification: Default::default(),
        },
        transform: TransformSignature {
            translation_mm: [seed as i64, 0, 0],
            rotation_quantized: [0, 0, 0, 1],
            scale_quantized: [1, 1, 1],
            hash: HashDigest::new([seed; 32]),
        },
        geometry: None,
        properties: vec![
            SemanticProperty {
                name: "Comments".to_owned(),
                value: CanonicalValue::Text(comments.to_owned()),
                measurement: None,
                display_name: None,
            },
            SemanticProperty {
                name: "Height".to_owned(),
                value: CanonicalValue::Real(3.048),
                measurement: Some(MeasurementMetadata::new(
                    "length",
                    "m",
                    Some("[ft_i]".to_owned()),
                )),
                display_name: None,
            },
        ],
        metadata_hash: HashDigest::new([seed.wrapping_add(1); 32]),
        full_hash: HashDigest::new([seed.wrapping_add(2); 32]),
    };
    let mut entities = HashMap::new();
    entities.insert(key, entity);
    SemanticSnapshot {
        snapshot_id: SnapshotId(snapshot_id.to_owned()),
        source: SnapshotSource::GitCommit {
            oid: oid.to_owned(),
        },
        config_hash: HashDigest::new([9; 32]),
        entities,
    }
}

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
}
