use anyhow::Result;
use openusd::usd::Stage;
use usd_model::{EntityKey, IdentitySource, SnapshotSource};
use usd_semantic::{IdentityConfig, SemanticConfig, SemanticExtractor};

const ORIGINAL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/identity_original.usda"
);
const MOVED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/identity_moved.usda"
);

fn config() -> SemanticConfig {
    SemanticConfig {
        identity: IdentityConfig {
            revit_unique_id_candidates: vec!["source:revitUniqueId".to_owned()],
            ifc_guid_candidates: vec!["source:ifcGuid".to_owned()],
            application_guid_candidates: vec!["source:applicationGuid".to_owned()],
            asset_identifier_candidates: vec!["source:assetIdentifier".to_owned()],
            allow_prim_path_fallback: true,
            allow_synthetic_fallback: false,
        },
        ..Default::default()
    }
}

fn extract(path: &str) -> Result<usd_model::SemanticSnapshot> {
    let stage = Stage::open(path)?;
    SemanticExtractor::new(config()).extract(
        &stage,
        SnapshotSource::Working {
            session: "identity-fixture".to_owned(),
            live_revision: 1,
        },
    )
}

#[test]
fn source_identity_survives_a_prim_path_move() -> Result<()> {
    let original = extract(ORIGINAL)?;
    let moved = extract(MOVED)?;
    let key = EntityKey::from("revit:revit-wall-001-opaque");

    let original_entity = original.entities.get(&key).expect("original wall key");
    let moved_entity = moved.entities.get(&key).expect("moved wall key");
    assert_eq!(
        original_entity.identity_source,
        IdentitySource::RevitUniqueId
    );
    assert_eq!(moved_entity.identity_source, IdentitySource::RevitUniqueId);
    assert_eq!(original_entity.prim_path, "/World/Building/Wall_A");
    assert_eq!(
        moved_entity.prim_path,
        "/World/RenamedBuilding/Wall_A_Renamed"
    );
    assert_eq!(original.entities.len(), moved.entities.len());
    Ok(())
}

#[test]
fn identity_priority_prefers_revit_over_other_configured_ids() -> Result<()> {
    let snapshot = extract(ORIGINAL)?;
    assert!(
        snapshot
            .entities
            .contains_key(&EntityKey::from("revit:revit-wall-001-opaque"))
    );
    assert!(
        !snapshot
            .entities
            .contains_key(&EntityKey::from("ifc:ifc-wall-001"))
    );
    Ok(())
}

#[test]
fn path_fallback_remains_the_default_when_no_candidates_are_configured() -> Result<()> {
    let stage = Stage::open(ORIGINAL)?;
    let snapshot = SemanticExtractor::new(SemanticConfig::default()).extract(
        &stage,
        SnapshotSource::Working {
            session: "identity-fixture".to_owned(),
            live_revision: 1,
        },
    )?;
    let entity = snapshot
        .entities
        .get(&EntityKey::from("/World/Building/Wall_A"))
        .expect("path identity");
    assert_eq!(entity.identity_source, IdentitySource::PrimPath);
    Ok(())
}
