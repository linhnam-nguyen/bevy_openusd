use usd_model::{
    BimIdentity, CanonicalValue, EntityKey, EntitySnapshot, HashDigest, IdentitySource,
    MeasurementMetadata, SemanticInfo, SemanticProperty, SemanticSnapshot, SnapshotId,
    SnapshotSource, TransformSignature,
};
use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe};

pub(super) fn digest(seed: u8) -> HashDigest {
    HashDigest::new([seed; HashDigest::BYTE_LEN])
}

pub(super) fn property(
    name: &str,
    value: CanonicalValue,
    measurement: Option<MeasurementMetadata>,
) -> SemanticProperty {
    SemanticProperty {
        name: name.to_owned(),
        value,
        measurement,
    }
}

pub(super) fn entity(
    key: &str,
    path: &str,
    category: Option<&str>,
    family: Option<&str>,
    type_name: Option<&str>,
    properties: Vec<SemanticProperty>,
) -> EntitySnapshot {
    EntitySnapshot {
        key: EntityKey::from(key),
        prim_path: path.to_owned(),
        identity_source: IdentitySource::PrimPath,
        semantic: SemanticInfo {
            category: category.map(str::to_owned),
            family: family.map(str::to_owned),
            type_name: type_name.map(str::to_owned),
            bim: BimIdentity {
                family_name: family.map(str::to_owned),
                ..Default::default()
            },
            ..SemanticInfo::default()
        },
        transform: TransformSignature {
            translation_mm: [0, 0, 0],
            rotation_quantized: [0, 0, 0, 1],
            scale_quantized: [1, 1, 1],
            hash: digest(3),
        },
        geometry: None,
        properties,
        metadata_hash: digest(4),
        full_hash: digest(5),
    }
}

pub(crate) fn snapshot() -> SemanticSnapshot {
    let measured = Some(MeasurementMetadata::new(
        "length",
        "m",
        Some("mm".to_owned()),
    ));
    let entities = [
        entity(
            "wall-a",
            "/World/WallA",
            Some("Walls"),
            Some("Basic"),
            Some("Wall"),
            vec![
                property("Mark", CanonicalValue::Text("AHU-01".to_owned()), None),
                property("Width", CanonicalValue::Real(0.2), measured.clone()),
                property("Level", CanonicalValue::Text("02".to_owned()), None),
            ],
        ),
        entity(
            "wall-b",
            "/World/WallB",
            Some("Walls"),
            Some("Basic"),
            Some("Wall"),
            vec![
                property("Mark", CanonicalValue::Text("AHU-02".to_owned()), None),
                property("Width", CanonicalValue::Real(0.2), measured),
                property("Level", CanonicalValue::Text("03".to_owned()), None),
            ],
        ),
        entity(
            "equipment-a",
            "/World/EquipmentA",
            Some("Equipment"),
            Some("Mechanical"),
            Some("AHU"),
            vec![property(
                "Mark",
                CanonicalValue::Text("AHU-03".to_owned()),
                None,
            )],
        ),
    ]
    .into_iter()
    .map(|entity| (entity.key.clone(), entity))
    .collect();
    SemanticSnapshot {
        snapshot_id: SnapshotId("bim-test-snapshot".to_owned()),
        source: SnapshotSource::Working {
            session: "bim-tests".to_owned(),
            live_revision: 1,
        },
        config_hash: digest(1),
        entities,
    }
}

pub(super) fn recipe() -> ClassificationRecipe {
    ClassificationRecipe::new(vec![
        ClassificationLevel::new("category", BimFieldKey::Category),
        ClassificationLevel::new("level", BimFieldKey::property("Level")),
        ClassificationLevel::new("type", BimFieldKey::Type),
    ])
}
