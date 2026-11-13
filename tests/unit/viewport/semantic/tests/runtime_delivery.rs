use anyhow::Result;
use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_model::EntityKey;

use super::super::{
    RenderServerInterface, RuntimeDeliveryRuntime, SemanticResponse, SemanticSyncState,
    SemanticWorkingStore, drain_runtime_delivery_results, flush_pending_runtime_delivery,
    synchronize_live_stage,
};
use super::fixtures::response;
use super::runtime_delivery_support::wait_for_manifest;

#[test]
fn test_regression_subtree_delta_runtime_delivery_manifest_and_blob_reuse() -> Result<()> {
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
    app.insert_resource(RuntimeDeliveryRuntime::default());

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

    // Frame 1: Full replace load
    app.update();

    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp, SemanticResponse::SnapshotLoaded { .. }));

    let shared_interface = server_interface.shared();
    let manifest_1 = wait_for_manifest(&mut app, &shared_interface, None);
    assert_eq!(manifest_1.meshes.len(), 2);

    // Find initial blob IDs
    let sync_state = app.world().resource::<SemanticSyncState>();
    let snapshot_1 = sync_state.snapshot.as_ref().expect("snapshot present");
    let b_key = EntityKey::from("/World/B/MeshB");
    let old_a_key = EntityKey::from("/World/A/OldMesh");

    let b_blob_id = snapshot_1
        .entities
        .get(&b_key)
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.as_ref())
        .unwrap()
        .0
        .clone();

    let old_a_blob_id = snapshot_1
        .entities
        .get(&old_a_key)
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.as_ref())
        .unwrap()
        .0
        .clone();

    let b_blob_bytes_1 = shared_interface
        .runtime_blob(&b_blob_id)
        .expect("b_blob exists in initial delivery");

    // Mutate stage: Remove /World/A/OldMesh, add /World/A/NewMesh
    {
        let live_ref = app.world().get_non_send::<LiveStage>().unwrap();
        live_ref
            .stage
            .remove_prim("/World/A/OldMesh")
            .expect("remove OldMesh");
        let new_mesh = live_ref
            .stage
            .define_prim("/World/A/NewMesh")
            .expect("define NewMesh");
        let new_mesh = new_mesh.set_type_name("Mesh").expect("set type");
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
        // Trigger resync on /World/A
        live_ref.load_payload("/World/A");
    }

    // Frame 2: Subtree delta update
    app.update();

    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp, SemanticResponse::DeltaApplied { .. }));

    // Check new manifest published
    let manifest_2 = wait_for_manifest(&mut app, &shared_interface, Some(&manifest_1.revision));
    assert_ne!(manifest_1.revision, manifest_2.revision);
    assert_eq!(manifest_2.meshes.len(), 2);

    let mesh_blob_ids: Vec<String> = manifest_2
        .meshes
        .iter()
        .map(|m| m.blob_id.clone())
        .collect();

    // 1. Unchanged /World/B blob is reused without change
    assert!(mesh_blob_ids.contains(&b_blob_id));
    let b_blob_bytes_2 = shared_interface
        .runtime_blob(&b_blob_id)
        .expect("b_blob exists in delta delivery");
    assert_eq!(b_blob_bytes_1, b_blob_bytes_2);

    // 2. Old /World/A/OldMesh blob is no longer in manifest meshes
    assert!(!mesh_blob_ids.contains(&old_a_blob_id));

    // 3. New /World/A/NewMesh blob is present
    let sync_state_2 = app.world().resource::<SemanticSyncState>();
    let snapshot_2 = sync_state_2.snapshot.as_ref().expect("snapshot present");
    let new_a_key = EntityKey::from("/World/A/NewMesh");
    let new_a_blob_id = snapshot_2
        .entities
        .get(&new_a_key)
        .and_then(|e| e.geometry.as_ref())
        .and_then(|g| g.render_blob.as_ref())
        .unwrap()
        .0
        .clone();
    assert!(mesh_blob_ids.contains(&new_a_blob_id));
    assert!(shared_interface.runtime_blob(&new_a_blob_id).is_some());

    // 4. Verify delivered hierarchy payload contains NewMesh, MeshB, and NOT OldMesh
    let hierarchy_bytes = shared_interface
        .runtime_blob(&manifest_2.hierarchy.blob_id)
        .expect("hierarchy blob exists");
    let hierarchy: crate::project::runtime_delivery::RuntimeHierarchyBlob =
        serde_json::from_slice(&hierarchy_bytes).expect("deserialize hierarchy blob");

    let hierarchy_paths: Vec<&str> = hierarchy
        .entities
        .iter()
        .map(|e| e.prim_path.as_str())
        .collect();
    assert!(hierarchy_paths.contains(&"/World/A/NewMesh"));
    assert!(hierarchy_paths.contains(&"/World/B/MeshB"));
    assert!(!hierarchy_paths.contains(&"/World/A/OldMesh"));

    Ok(())
}

#[test]
fn test_regression_local_native_mode_boundary_skips_delivery() -> Result<()> {
    let usda = String::from(
        r#"#usda 1.0
def Xform "World"
{
    def Xform "A" {}
    def Xform "B" {}
}
"#,
    );
    let stage = usd_bevy::UsdSnippet::new(&usda)
        .open_stage()
        .expect("stage opens");
    let live = LiveStage::new(stage);

    let mut app = App::new();
    app.add_plugins(usd_bevy::LiveStagePlugin);
    app.insert_resource(SemanticWorkingStore::default());
    app.insert_resource(SemanticSyncState::default());
    // Deliberately NO RenderServerInterface resource (local native mode)
    app.world_mut().insert_non_send(live);
    app.add_systems(PostUpdate, synchronize_live_stage);

    // Frame 1: Full replace load
    app.update();
    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp, SemanticResponse::SnapshotLoaded { .. }));

    // Resync /World/A
    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .load_payload("/World/A");

    // Frame 2: Subtree delta update
    app.update();
    let resp = response(app.world().resource::<SemanticWorkingStore>());
    assert!(matches!(resp, SemanticResponse::DeltaApplied { .. }));

    // Confirm RenderServerInterface was never created or queried
    assert!(
        app.world()
            .get_resource::<RenderServerInterface>()
            .is_none()
    );
    Ok(())
}
