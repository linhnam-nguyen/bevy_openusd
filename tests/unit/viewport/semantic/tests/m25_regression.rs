use anyhow::Result;
use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_model::EntityKey;

use super::super::{
    RenderServerInterface, SemanticResponse, SemanticSyncState, SemanticWorkingStore,
    drain_runtime_delivery_results, flush_pending_runtime_delivery, synchronize_live_stage,
};
use super::fixtures::response;
use super::runtime_delivery_support::wait_for_manifest;

#[test]
fn test_m25_o11_milestone_acceptance_unaffected_sibling_pipeline_invariance() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A"
    {
        def Mesh "OldMesh"
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
            point3f[] points = [(10, 0, 0), (11, 0, 0), (10, 1, 0)]
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

    let server_interface = RenderServerInterface::default();
    app.insert_resource(server_interface.clone());

    app.world_mut().insert_non_send(live);
    app.add_systems(
        PostUpdate,
        (
            drain_runtime_delivery_results,
            synchronize_live_stage,
            flush_pending_runtime_delivery,
        )
            .chain(),
    );

    // ==========================================
    // Frame 1: Initial full load
    // ==========================================
    app.update();
    let resp_1 = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp_1, SemanticResponse::SnapshotLoaded { .. }));

    // Capture BEFORE state for /World/B
    let b_key = EntityKey::from("/World/B/MeshB");
    let prim_entities = app.world().resource::<usd_bevy::PrimEntities>();
    let paths = app.world().resource::<usd_bevy::PathStore>();
    let b_bevy_entity_before = prim_entities
        .entity(paths, "/World/B/MeshB")
        .expect("/World/B/MeshB in Bevy PrimEntities");

    let sync_state = app.world().resource::<SemanticSyncState>();
    let snapshot_1 = sync_state.snapshot.as_ref().expect("snapshot present");
    let b_entity_sem_before = snapshot_1
        .entities
        .get(&b_key)
        .cloned()
        .expect("/World/B/MeshB in semantic entities");
    let b_blob_id_before = b_entity_sem_before
        .geometry
        .as_ref()
        .and_then(|g| g.render_blob.as_ref())
        .unwrap()
        .0
        .clone();

    // Capture BEFORE state for /World/A/OldMesh blob
    let old_a_key = EntityKey::from("/World/A/OldMesh");
    let old_a_blob_id_before = snapshot_1
        .entities
        .get(&old_a_key)
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.as_ref())
        .unwrap()
        .0
        .clone();

    let shared_interface = server_interface.shared();
    let manifest_1 = wait_for_manifest(&mut app, &shared_interface, None);
    let b_blob_bytes_before = shared_interface
        .runtime_blob(&b_blob_id_before)
        .expect("b_blob in runtime delivery 1");

    // ==========================================
    // Stage Mutation strictly under /World/A
    // ==========================================
    {
        let live_ref = app.world().get_non_send::<LiveStage>().unwrap();
        live_ref.stage.remove_prim("/World/A/OldMesh").unwrap();
        let new_mesh = live_ref.stage.define_prim("/World/A/NewMesh").unwrap();
        let new_mesh = new_mesh.set_type_name("Mesh").unwrap();
        new_mesh
            .create_attribute("points", "point3f[]")
            .unwrap()
            .set(openusd::sdf::Value::Vec3fVec(vec![
                openusd::gf::Vec3f::from([0.0, 0.0, 0.0]),
                openusd::gf::Vec3f::from([2.0, 0.0, 0.0]),
                openusd::gf::Vec3f::from([0.0, 2.0, 0.0]),
            ]))
            .unwrap();
        new_mesh
            .create_attribute("faceVertexCounts", "int[]")
            .unwrap()
            .set(openusd::sdf::Value::IntVec(vec![3]))
            .unwrap();
        new_mesh
            .create_attribute("faceVertexIndices", "int[]")
            .unwrap()
            .set(openusd::sdf::Value::IntVec(vec![0, 1, 2]))
            .unwrap();
        let _ = live_ref.drain_change_batch();
        live_ref.load_payload("/World/A");
    }

    // ==========================================
    // Frame 2: Subtree delta update
    // ==========================================
    app.update();
    let resp_2 = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp_2, SemanticResponse::DeltaApplied { .. }));

    // ==========================================
    // Pipeline Invariant Verification
    // ==========================================
    // 1. /World/A changed as expected:
    let sync_state_2 = app.world().resource::<SemanticSyncState>();
    let snapshot_2 = sync_state_2.snapshot.as_ref().expect("snapshot 2 present");
    let new_a_key = EntityKey::from("/World/A/NewMesh");
    assert!(
        snapshot_2.entities.contains_key(&new_a_key),
        "/World/A/NewMesh exists in semantic snapshot"
    );
    assert!(
        !snapshot_2.entities.contains_key(&old_a_key),
        "/World/A/OldMesh was removed from semantic snapshot"
    );

    let new_a_blob_id_after = snapshot_2
        .entities
        .get(&new_a_key)
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.as_ref())
        .unwrap()
        .0
        .clone();

    // 2. /World/B: Bevy Entity ID unchanged:
    let prim_entities_2 = app.world().resource::<usd_bevy::PrimEntities>();
    let paths_2 = app.world().resource::<usd_bevy::PathStore>();
    let b_bevy_entity_after = prim_entities_2
        .entity(paths_2, "/World/B/MeshB")
        .expect("/World/B/MeshB still in Bevy PrimEntities");
    assert_eq!(
        b_bevy_entity_before, b_bevy_entity_after,
        "Bevy entity ID for /World/B must remain unchanged"
    );

    // 3. /World/B: Semantic entity unchanged:
    let b_entity_sem_after = snapshot_2
        .entities
        .get(&b_key)
        .cloned()
        .expect("/World/B/MeshB in semantic entities");
    assert_eq!(
        b_entity_sem_before, b_entity_sem_after,
        "Semantic entity content for /World/B must remain unchanged"
    );

    // 4. /World/B: Render blob identity & payload unchanged (no unintended blob churn):
    let b_blob_id_after = b_entity_sem_after
        .geometry
        .as_ref()
        .and_then(|g| g.render_blob.as_ref())
        .unwrap()
        .0
        .clone();
    assert_eq!(
        b_blob_id_before, b_blob_id_after,
        "Render blob ID for /World/B must remain identical"
    );

    let manifest_2 = wait_for_manifest(&mut app, &shared_interface, Some(&manifest_1.revision));
    let b_blob_bytes_after = shared_interface
        .runtime_blob(&b_blob_id_after)
        .expect("b_blob in runtime delivery 2");
    assert_eq!(
        b_blob_bytes_before, b_blob_bytes_after,
        "Render blob binary payload for /World/B must remain identical"
    );

    // 5. Manifest contains new A and reused B, and strictly NOT old A:
    let manifest_blob_ids: Vec<String> = manifest_2
        .meshes
        .iter()
        .map(|m| m.blob_id.clone())
        .collect();
    assert!(
        manifest_blob_ids.contains(&b_blob_id_after),
        "Manifest must contain reused B blob"
    );
    assert!(
        manifest_blob_ids.contains(&new_a_blob_id_after),
        "Manifest must contain new A blob"
    );
    assert!(
        !manifest_blob_ids.contains(&old_a_blob_id_before),
        "Manifest must NOT contain old A blob"
    );
    assert_eq!(manifest_2.meshes.len(), 2);

    Ok(())
}
