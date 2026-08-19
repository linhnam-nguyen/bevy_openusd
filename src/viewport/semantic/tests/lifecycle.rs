use anyhow::Result;
use bevy::prelude::World;
use openusd::usd::Stage;
use usd_bevy::LiveStage;
use usd_model::{EntityKey, HashDigest, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::fixtures::{response, snapshot};
use super::super::{
    SemanticDiffState, SemanticFilter, SemanticQuery, SemanticResponse, SemanticSyncState,
    SemanticWorkingStore, synchronize_live_stage,
};

#[test]
fn resync_full_replace_removes_entities_from_the_working_store() -> Result<()> {
    let store = SemanticWorkingStore::default();
    let initial = snapshot()?;
    let initial_count = initial.entities.len() as u32;
    assert!(store.submit_snapshot("load-resync", initial.clone()));
    let _ = response(&store);

    let mut entities = initial.entities.clone();
    entities.remove(&EntityKey::from("/World/Robot"));
    let rebuilt = SemanticExtractor::new(SemanticConfig::default()).snapshot_from_entities(
        SnapshotSource::Working {
            session: "semantic-worker-test".to_owned(),
            live_revision: 3,
        },
        entities,
    );
    assert!(store.submit_snapshot("resync-1", rebuilt));
    assert!(matches!(
        response(&store),
        SemanticResponse::SnapshotLoaded {
            request_id,
            entity_count
        } if request_id == "resync-1" && entity_count == initial_count - 1
    ));

    assert!(store.submit_query(
        "query-removed-type",
        SemanticQuery {
            filters: vec![SemanticFilter::TypeEquals("Cube".to_owned())],
            ..Default::default()
        },
    ));
    let SemanticResponse::QueryResult { result, .. } = response(&store) else {
        panic!("expected rebuilt query result")
    };
    assert_eq!(result.total, 0);
    Ok(())
}

#[test]
fn manual_baseline_recomputes_for_working_changes_and_resets_on_reload() -> Result<()> {
    let initial = snapshot()?;
    let mut state = SemanticDiffState::default();
    state.update_working(1, initial.clone());

    assert!(state.capture_baseline());
    let initial_summary = state.summary().expect("baseline and working are present");
    assert_eq!(initial_summary.added, 0);
    assert_eq!(initial_summary.removed, 0);
    assert_eq!(initial_summary.changed, 0);
    assert_eq!(initial_summary.unchanged, initial.entities.len());

    let mut changed = initial;
    let key = changed
        .entities
        .keys()
        .next()
        .cloned()
        .expect("fixture contains semantic entities");
    let entity = changed
        .entities
        .get_mut(&key)
        .expect("entity key came from the snapshot");
    entity.prim_path.push_str("/Moved");
    entity.full_hash = HashDigest::new([0xa5; HashDigest::BYTE_LEN]);
    state.update_working(1, changed);

    let summary = state.summary().expect("baseline and working are present");
    assert_eq!(summary.changed, 1);
    assert_eq!(summary.path, 1);
    assert_eq!(summary.transform, 0);
    assert_eq!(summary.metadata, 0);
    assert_eq!(summary.geometry, 0);

    state.update_working(2, snapshot()?);
    assert!(!state.has_baseline());
    assert_eq!(state.summary(), None);
    Ok(())
}

#[test]
fn a_new_live_stage_session_triggers_a_full_semantic_load() -> Result<()> {
    let mut world = World::new();
    world.insert_resource(SemanticWorkingStore::default());
    world.insert_resource(usd_bevy::PendingStageChanges::default());
    world.insert_resource(SemanticSyncState::default());
    world.insert_non_send(LiveStage::new(Stage::open(
        "tests/stages/custom_attrs_extensive.usda",
    )?));

    synchronize_live_stage(&mut world);
    assert!(matches!(
        response(world.resource::<SemanticWorkingStore>()),
        SemanticResponse::SnapshotLoaded { .. }
    ));

    world.remove_non_send::<LiveStage>();
    world.insert_non_send(LiveStage::new(Stage::open(
        "tests/stages/custom_attrs_extensive.usda",
    )?));
    synchronize_live_stage(&mut world);
    assert!(matches!(
        response(world.resource::<SemanticWorkingStore>()),
        SemanticResponse::SnapshotLoaded { .. }
    ));
    Ok(())
}
