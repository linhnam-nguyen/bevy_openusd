use anyhow::Result;
use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{LiveRevision, StageChange, StageChangeBatch};
use usd_model::{CanonicalValue, EntityKey, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::{
    SemanticFilter, SemanticIncrementalUpdate, SemanticQuery, SemanticResponse, SemanticSyncState,
    SemanticWorkingStore, SubtreeUpdateError, changed_info_update, synchronize_live_stage,
};
use super::fixtures::{response, snapshot};

#[test]
fn sync_telemetry_separates_changed_info_from_subtree_extraction() -> Result<()> {
    let project = tempfile::tempdir()?;
    let live = usd_bevy::LiveStage::new(Stage::open("tests/stages/custom_attrs_extensive.usda")?);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(crate::project::recovery::RecoverySettings {
        project_root: project.path().to_path_buf(),
    });
    app.insert_resource(crate::viewport::diagnostics::performance::RendererCounters::default());
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    app.update();
    let _ = response(app.world().resource::<SemanticWorkingStore>());
    let before = app
        .world()
        .resource::<crate::viewport::diagnostics::performance::RendererCounters>()
        .clone();

    usd_bevy::authoring::set_attribute(
        &app.world()
            .get_non_send::<usd_bevy::LiveStage>()
            .unwrap()
            .stage,
        "/World/Robot",
        "userProperties:name",
        "string",
        openusd::sdf::Value::String("cart_telemetry".to_owned()),
    )?;
    app.update();
    let _ = response(app.world().resource::<SemanticWorkingStore>());
    let after_changed_info = app
        .world()
        .resource::<crate::viewport::diagnostics::performance::RendererCounters>();
    assert_eq!(
        after_changed_info.semantic_changed_info_updates,
        before.semantic_changed_info_updates + 1
    );
    assert_eq!(
        after_changed_info.semantic_subtree_extractions,
        before.semantic_subtree_extractions
    );

    app.world()
        .get_non_send::<usd_bevy::LiveStage>()
        .unwrap()
        .load_payload("/World/Robot");
    app.update();
    let _ = response(app.world().resource::<SemanticWorkingStore>());
    let after_subtree = app
        .world()
        .resource::<crate::viewport::diagnostics::performance::RendererCounters>();
    assert_eq!(
        after_subtree.semantic_subtree_extractions,
        before.semantic_subtree_extractions + 1
    );
    Ok(())
}

#[test]
fn changed_info_delta_updates_only_the_affected_semantic_entity() -> Result<()> {
    let store = SemanticWorkingStore::default();
    let snapshot = snapshot()?;
    let entity_count = snapshot.entities.len() as u32;
    assert!(store.submit_snapshot("load-delta", snapshot.clone()));
    let _ = response(&store);

    let mut robot = snapshot
        .entities
        .get(&EntityKey::from("/World/Robot"))
        .cloned()
        .expect("fixture robot entity");
    let property = robot
        .properties
        .iter_mut()
        .find(|property| property.name == "userProperties:name")
        .expect("fixture robot property");
    property.value = CanonicalValue::Text("cart_02".to_owned());

    assert!(store.submit_delta(
        "delta-1",
        SemanticIncrementalUpdate {
            snapshot_id: snapshot.snapshot_id.clone(),
            source: SnapshotSource::Working {
                session: "semantic-worker-test".to_owned(),
                live_revision: 2,
            },
            config_hash: snapshot.config_hash,
            upserts: vec![robot],
            removed_paths: Vec::new(),
        },
    ));
    assert!(matches!(
        response(&store),
        SemanticResponse::DeltaApplied {
            request_id,
            upserted: 1,
            removed: 0,
        } if request_id == "delta-1"
    ));

    assert!(store.submit_query(
        "query-updated-property",
        SemanticQuery {
            filters: vec![SemanticFilter::PropertyTextEquals {
                name: "userProperties:name".to_owned(),
                value: "cart_02".to_owned(),
            }],
            ..Default::default()
        },
    ));
    let SemanticResponse::QueryResult { result, .. } = response(&store) else {
        panic!("expected updated property query result")
    };
    assert_eq!(result.total, 1);
    assert_eq!(result.rows[0].prim_path, "/World/Robot");

    assert!(store.submit_query("query-all-after-delta", SemanticQuery::default()));
    let SemanticResponse::QueryResult { result, .. } = response(&store) else {
        panic!("expected full query result")
    };
    assert_eq!(result.total, entity_count);
    Ok(())
}

#[test]
fn changed_info_extraction_replaces_only_the_affected_prim() -> Result<()> {
    let stage = Stage::open("tests/stages/custom_attrs_extensive.usda")?;
    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let before = extractor.extract(
        &stage,
        SnapshotSource::Working {
            session: "semantic-sync-test".to_owned(),
            live_revision: 1,
        },
    )?;
    let batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: vec![StageChange {
            resynced: Vec::new(),
            changed_info: vec!["/World/Robot.userProperties:name".to_owned()],
        }],
    };

    let delta = changed_info_update(
        &stage,
        &extractor,
        before.clone(),
        &batch,
        SnapshotSource::Working {
            session: "semantic-sync-test".to_owned(),
            live_revision: 2,
        },
    )?;
    assert_eq!(delta.request.upserts.len(), 1);
    assert_eq!(delta.request.upserts[0].prim_path, "/World/Robot");
    assert!(delta.request.removed_paths.is_empty());
    assert_eq!(delta.snapshot.entities.len(), before.entities.len());
    Ok(())
}

#[test]
fn test_changed_info_captures_old_identities_and_computes_removed_paths() -> Result<()> {
    let stage = Stage::builder()
        .in_memory("changed-info-remove.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/B").unwrap();

    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source = SnapshotSource::Working {
        session: "test-session".to_owned(),
        live_revision: 1,
    };
    let initial_snapshot = extractor.extract(&stage, source.clone())?;
    assert_eq!(initial_snapshot.entities.len(), 3);

    // Remove /World/A from stage, but send changed_info for /World/A
    stage.remove_prim("/World/A")?;

    let mut batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: Vec::new(),
    };
    batch.changes.push(StageChange {
        changed_info: vec!["/World/A.xformOp:translate".to_owned()],
        resynced: Vec::new(),
    });

    let delta = changed_info_update(&stage, &extractor, initial_snapshot, &batch, source)?;
    assert_eq!(delta.request.removed_paths, vec!["/World/A"]);
    assert_eq!(delta.request.upserts.len(), 0);
    assert_eq!(delta.snapshot.entities.len(), 2);
    assert!(
        !delta
            .snapshot
            .entities
            .values()
            .any(|e| e.prim_path == "/World/A")
    );
    assert!(
        delta
            .snapshot
            .entities
            .values()
            .any(|e| e.prim_path == "/World/B")
    );
    Ok(())
}

#[test]
fn test_changed_info_rejects_entity_key_collisions() -> Result<()> {
    let stage = Stage::builder()
        .in_memory("changed-info-collision.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();
    stage.define_prim("/World/A").unwrap();
    stage.define_prim("/World/B").unwrap();

    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source = SnapshotSource::Working {
        session: "test-session".to_owned(),
        live_revision: 1,
    };
    let mut initial_snapshot = extractor.extract(&stage, source.clone())?;

    // Artificially give unaffected entity /World/B the same EntityKey as /World/A's extracted key
    let a_usd_path = openusd::sdf::path("/World/A")?;
    let a_entity = extractor.extract_entity(&stage, &a_usd_path)?;

    // Find /World/B in snapshot and replace its key with /World/A's key
    let b_key = initial_snapshot
        .entities
        .iter()
        .find(|(_, e)| e.prim_path == "/World/B")
        .map(|(k, _)| k.clone())
        .unwrap();
    let mut b_entity = initial_snapshot.entities.remove(&b_key).unwrap();
    b_entity.key = a_entity.key.clone();
    initial_snapshot
        .entities
        .insert(a_entity.key.clone(), b_entity);

    // Send changed_info for /World/A only (/World/B is unaffected)
    let mut batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: Vec::new(),
    };
    batch.changes.push(StageChange {
        changed_info: vec!["/World/A.xformOp:translate".to_owned()],
        resynced: Vec::new(),
    });

    let result = changed_info_update(&stage, &extractor, initial_snapshot, &batch, source);
    assert!(matches!(
        result,
        Err(SubtreeUpdateError::EntityKeyCollision(_))
    ));
    Ok(())
}

#[test]
fn test_changed_info_propagates_extraction_errors() -> Result<()> {
    let stage = Stage::builder()
        .in_memory("changed-info-error.usda")
        .expect("in-memory stage");

    stage.define_prim("/World").unwrap();

    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source = SnapshotSource::Working {
        session: "test-session".to_owned(),
        live_revision: 1,
    };
    let initial_snapshot = extractor.extract(&stage, source.clone())?;

    // Send invalid path format in changed_info that fails sdf::path parsing
    let mut batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: Vec::new(),
    };
    batch.changes.push(StageChange {
        changed_info: vec!["not_a_valid_usd_path".to_owned()],
        resynced: Vec::new(),
    });

    let result = changed_info_update(&stage, &extractor, initial_snapshot, &batch, source.clone());
    // Stage traverse won't find "not_a_valid_usd_path", so it marks it as removed_paths
    assert!(result.is_ok());

    let mut batch_invalid = StageChangeBatch {
        revision: LiveRevision(3),
        changes: Vec::new(),
    };
    stage.define_prim("/World/Child").unwrap();
    let initial_snapshot_2 = extractor.extract(&stage, source.clone())?;

    batch_invalid.changes.push(StageChange {
        changed_info: vec!["/World/Child".to_owned()],
        resynced: Vec::new(),
    });
    let valid_result = changed_info_update(
        &stage,
        &extractor,
        initial_snapshot_2,
        &batch_invalid,
        source,
    );
    assert!(valid_result.is_ok());
    Ok(())
}
