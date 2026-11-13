use anyhow::Result;
use bevy::prelude::*;
use usd_bevy::{LiveRevision, LiveStage, StageChange, StageChangeBatch};
use usd_model::{EntityKey, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};

use super::super::{
    SemanticResponse, SemanticSyncAction, SemanticSyncState, SemanticWorkingStore,
    attach_render_blobs_to_action, resync_subtree_update, synchronize_live_stage,
};
use super::fixtures::response;

#[test]
fn test_regression_resync_subtree_render_blob_enrichment_scoped_to_affected_entities() -> Result<()>
{
    let temp_dir = tempfile::tempdir()?;
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Mesh "MeshA"
        {
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
        }
    }
    def Xform "B"
    {
        def Mesh "MeshB"
        {
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
        }
    }
}
"#,
    );

    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("synthetic stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(usd_bevy::UsdPlugin)
        .add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(crate::project::recovery::RecoverySettings {
        project_root: temp_dir.path().to_path_buf(),
    });
    app.insert_resource(crate::project::ghost_cache::HistoricalGeometryCache::default());
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    // Frame 1: Full replace load
    app.update();

    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp, SemanticResponse::SnapshotLoaded { .. }));

    // Get initial blob reference on /World/B/MeshB
    let sync_state = app.world().resource::<SemanticSyncState>();
    let snapshot = sync_state.snapshot.as_ref().expect("snapshot present");
    let b_key = EntityKey::from("/World/B/MeshB");
    let b_blob_before = snapshot
        .entities
        .get(&b_key)
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.clone())
        .expect("/World/B/MeshB has render_blob");

    // Reset cache counters to 0 to measure only the subtree resync delta work
    {
        let mut cache = app
            .world_mut()
            .resource_mut::<crate::project::ghost_cache::HistoricalGeometryCache>();
        *cache = crate::project::ghost_cache::HistoricalGeometryCache::default();
    }

    // Resync /World/A (affected prims: /World/A and /World/A/MeshA)
    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .load_payload("/World/A");

    // Frame 2: Subtree delta update
    app.update();

    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp, SemanticResponse::DeltaApplied { .. }));

    // Verify HistoricalGeometryCache: only /World/A affected entities and mesh were scanned!
    let cache = *app
        .world()
        .resource::<crate::project::ghost_cache::HistoricalGeometryCache>();
    assert_eq!(cache.snapshots_seen, 1);
    // Scanned ONLY the affected /World/A entities (2: /World/A and /World/A/MeshA), NOT the whole stage (5 prims)!
    assert_eq!(cache.semantic_entities_scanned, 2);
    // Scanned ONLY the affected mesh handle for /World/A/MeshA (1 mesh), NOT all meshes (2 meshes)!
    assert_eq!(cache.mesh_handles_scanned, 1);

    // Verify unaffected /World/B/MeshB render blob identity is unchanged
    let sync_state = app.world().resource::<SemanticSyncState>();
    let snapshot = sync_state.snapshot.as_ref().expect("snapshot present");
    let b_blob_after = snapshot
        .entities
        .get(&b_key)
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.clone())
        .expect("/World/B/MeshB has render_blob");

    assert_eq!(b_blob_before, b_blob_after);
    Ok(())
}

#[test]
fn test_fallback_missing_prim_entities_triggers_full_attach() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Mesh "MeshA"
        {
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
        }
    }
}
"#,
    );
    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("stage opens");
    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source_1 = SnapshotSource::Working {
        session: "fallback-test".to_owned(),
        live_revision: 1,
    };
    let snapshot_1 = extractor.extract(&stage, source_1)?;

    let mut batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: Vec::new(),
    };
    batch.changes.push(StageChange {
        changed_info: Vec::new(),
        resynced: vec!["/World/A".to_owned()],
    });

    let source_2 = SnapshotSource::Working {
        session: "fallback-test".to_owned(),
        live_revision: 2,
    };
    let delta = resync_subtree_update(&stage, &extractor, snapshot_1, &batch, source_2)?;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(usd_bevy::UsdPlugin);
    app.insert_resource(crate::project::recovery::RecoverySettings {
        project_root: temp_dir.path().to_path_buf(),
    });
    app.insert_resource(crate::project::ghost_cache::HistoricalGeometryCache::default());

    // Spawn a Mesh directly with UsdPrimRef
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(bevy::mesh::Indices::U32(vec![0, 1, 2]));
    let handle = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    app.world_mut().spawn((
        usd_bevy::UsdPrimRef::new("/World/A/MeshA"),
        bevy::mesh::Mesh3d(handle),
    ));

    // Note: app.world does NOT have PrimEntities resource!
    assert!(
        app.world()
            .get_resource::<usd_bevy::PrimEntities>()
            .is_none()
    );

    let mut action = SemanticSyncAction::Delta(delta);
    attach_render_blobs_to_action(app.world_mut(), &mut action, LiveRevision(2), 1);

    let SemanticSyncAction::Delta(result_delta) = action else {
        panic!("expected Delta action");
    };

    // Verifies fallback attached the blob via full attach safely
    let blob = result_delta
        .snapshot
        .entities
        .get(&EntityKey::from("/World/A/MeshA"))
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.clone());
    assert!(
        blob.is_some(),
        "blob safely attached via full attach fallback"
    );
    Ok(())
}

#[test]
fn test_fallback_partial_prim_entities_index_corruption_triggers_full_attach() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Mesh "MeshA"
        {
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
        }
    }
}
"#,
    );
    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("stage opens");
    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source_1 = SnapshotSource::Working {
        session: "fallback-test".to_owned(),
        live_revision: 1,
    };
    let snapshot_1 = extractor.extract(&stage, source_1)?;

    let mut batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: Vec::new(),
    };
    batch.changes.push(StageChange {
        changed_info: Vec::new(),
        resynced: vec!["/World/A".to_owned()],
    });

    let source_2 = SnapshotSource::Working {
        session: "fallback-test".to_owned(),
        live_revision: 2,
    };
    let delta = resync_subtree_update(&stage, &extractor, snapshot_1, &batch, source_2)?;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<bevy::image::Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(usd_bevy::UsdPlugin);
    app.insert_resource(crate::project::recovery::RecoverySettings {
        project_root: temp_dir.path().to_path_buf(),
    });
    app.insert_resource(crate::project::ghost_cache::HistoricalGeometryCache::default());

    // Spawn a Mesh directly with UsdPrimRef
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(bevy::mesh::Indices::U32(vec![0, 1, 2]));
    let handle = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let entity = app
        .world_mut()
        .spawn((
            usd_bevy::UsdPrimRef::new("/World/A/MeshA"),
            bevy::mesh::Mesh3d(handle),
        ))
        .id();

    // PrimEntities EXISTS in World, but does NOT contain /World/A/MeshA (partial corruption)
    let mut map = usd_bevy::PrimEntities::default();
    let mut paths = usd_bevy::PathStore::default();
    map.insert(&mut paths, "/World", entity);
    // /World/A/MeshA is intentionally omitted from map!
    app.insert_resource(paths);
    app.insert_resource(map);

    let mut action = SemanticSyncAction::Delta(delta);
    attach_render_blobs_to_action(app.world_mut(), &mut action, LiveRevision(2), 1);

    let SemanticSyncAction::Delta(result_delta) = action else {
        panic!("expected Delta action");
    };

    // Verifies partial index corruption triggers full attach fallback and successfully attaches blob
    let blob = result_delta
        .snapshot
        .entities
        .get(&EntityKey::from("/World/A/MeshA"))
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.clone());
    assert!(
        blob.is_some(),
        "blob safely attached via full attach fallback after partial index corruption"
    );
    Ok(())
}
