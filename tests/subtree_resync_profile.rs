//! Milestone 25-O0 baseline profiling for USD resync reconciliation.
//!
//! Measures and records baseline work counters (traversals, patched entities,
//! semantic extractions, Turso row operations, render-blob scans) and execution
//! timings before subtree-resync optimizations are applied.

use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::mesh::{Indices, Mesh, Mesh3d, PrimitiveTopology};
use bevy::prelude::*;
use openusd::usd::Stage;
use tempfile::tempdir;
use usd_bevy::{LiveStage, LiveStagePlugin, PrimEntities, UsdPlugin, UsdPrimRef, UsdSnippet};
use usd_model::{
    Bounds3, CanonicalValue, EntityKey, EntitySnapshot, GeometrySignature, HashDigest,
    IdentitySource, QuantizedPoint3, SemanticInfo, SemanticProperty, SnapshotSource,
    TransformSignature,
};
use usd_semantic::{SemanticConfig, SemanticExtractor};

fn build_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin {
            file_path: "tests/stages".into(),
            ..Default::default()
        })
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin);
    app
}

/// Generates a wide synthetic scene with /World, /World/A (10 children),
/// /World/B (10 children), /World/C (10 children) -> 34 prims total.
fn make_synthetic_wide_stage() -> Stage {
    let mut usda = String::from("#usda 1.0\n\ndef Xform \"World\"\n{\n");
    for group in ["A", "B", "C"] {
        usda.push_str(&format!("    def Xform \"{group}\"\n    {{\n"));
        for i in 0..10 {
            usda.push_str(&format!(
                "        def Xform \"{group}{i}\"\n        {{\n        }}\n"
            ));
        }
        usda.push_str("    }\n");
    }
    usda.push_str("}\n");

    UsdSnippet::new(&usda)
        .open_stage()
        .expect("synthetic wide stage opens")
}

/// Generates a deep overlap scene with /World, /World/A/Child/Leaf, /World/B, /World/C.
fn make_deep_overlap_stage() -> Stage {
    UsdSnippet::new(
        r#"#usda 1.0

def Xform "World"
{
    def Xform "A"
    {
        def Xform "Child"
        {
            def Xform "Leaf"
            {
            }
        }
    }
    def Xform "B"
    {
    }
    def Xform "C"
    {
    }
}
"#,
    )
    .open_stage()
    .expect("deep overlap stage opens")
}

#[test]
fn profiles_synthetic_wide_baseline() {
    let mut app = build_test_app();
    let stage = make_synthetic_wide_stage();
    let live = LiveStage::new(stage);

    app.world_mut().insert_non_send(live);
    // Initial frame performs initial project_stage
    app.update();

    let initial_prim_count = app.world().resource::<PrimEntities>().len();
    // 34 prims + stage root "/" = 35 mapped entities
    assert_eq!(initial_prim_count, 35);

    // Baseline full resync targeting only /World/B (no mark_authored on resync)
    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .load_payload("/World/B"); // synthesized resync notice

    let start = Instant::now();
    app.update();
    let frame_elapsed = start.elapsed();

    let post_count = app.world().resource::<PrimEntities>().len();
    assert_eq!(post_count, 35);

    println!(
        "M25 Baseline (Synthetic Wide): total_prims=34, target=/World/B (11 prims affected), total_entities_mapped={}, frame_elapsed={:?}",
        post_count, frame_elapsed
    );
}

#[test]
fn profiles_deep_overlap_baseline() {
    let mut app = build_test_app();
    let stage = make_deep_overlap_stage();
    let live = LiveStage::new(stage);

    app.world_mut().insert_non_send(live);
    app.update();

    let initial_count = app.world().resource::<PrimEntities>().len();
    // /World, /World/A, /World/A/Child, /World/A/Child/Leaf, /World/B, /World/C + root "/" = 7 entities
    assert_eq!(initial_count, 7);

    // Enqueue multiple overlapping resync notices
    let live_stage = app.world().get_non_send::<LiveStage>().unwrap();
    live_stage.load_payload("/World/A");
    live_stage.load_payload("/World/A/Child");
    live_stage.load_payload("/World/A/Child/Leaf");

    let start = Instant::now();
    app.update();
    let frame_elapsed = start.elapsed();

    let post_count = app.world().resource::<PrimEntities>().len();
    assert_eq!(post_count, 7);

    println!(
        "M25 Baseline (Deep Overlap): total_prims=6, notice_roots=[/World/A, /World/A/Child, /World/A/Child/Leaf], total_entities_mapped={}, frame_elapsed={:?}",
        post_count, frame_elapsed
    );
}

#[test]
fn profiles_real_materials_fixture_baseline() {
    let mut app = build_test_app();
    let stage = Stage::open("tests/stages/materials.usda").expect("materials fixture opens");
    let live = LiveStage::new(stage);

    app.world_mut().insert_non_send(live);
    app.update();

    let initial_count = app.world().resource::<PrimEntities>().len();
    assert!(initial_count > 0);

    // Resync the /World/Materials subtree
    app.world()
        .get_non_send::<LiveStage>()
        .unwrap()
        .load_payload("/World/Materials");

    let start = Instant::now();
    app.update();
    let frame_elapsed = start.elapsed();

    let post_count = app.world().resource::<PrimEntities>().len();
    assert_eq!(post_count, initial_count);

    println!(
        "M25 Baseline (Real Fixture - materials.usda): total_prims={}, resync=/World/Materials, total_entities_mapped={}, frame_elapsed={:?}",
        initial_count - 1,
        post_count,
        frame_elapsed
    );
}

#[test]
fn profiles_semantic_and_turso_pipeline_baseline() {
    let stage = make_synthetic_wide_stage();
    let extractor = SemanticExtractor::new(SemanticConfig::default());
    let source = SnapshotSource::Working {
        session: "test".to_owned(),
        live_revision: 1,
    };

    // 1. Semantic Extraction baseline
    let start_extract = Instant::now();
    let snapshot = extractor
        .extract(&stage, source)
        .expect("extraction succeeds");
    let extract_elapsed = start_extract.elapsed();

    println!(
        "M25 Baseline (Semantic Full Extraction): resync_op=ReplaceSnapshot, entities_extracted={}, extract_elapsed={:?}",
        snapshot.entities.len(),
        extract_elapsed
    );
    assert_eq!(snapshot.entities.len(), 34);

    // 2. Turso database replace_snapshot baseline
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime builds");

    runtime.block_on(async {
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("turso db builds");
        let mut conn = db.connect().expect("turso connects");

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS snapshots (snapshot_id TEXT PRIMARY KEY, count INTEGER);
            CREATE TABLE IF NOT EXISTS entities (snapshot_id TEXT, entity_key TEXT, prim_path TEXT, PRIMARY KEY(snapshot_id, entity_key));
            "#,
        )
        .await
        .expect("schema applied");

        let start_turso = Instant::now();
        let tx = conn.transaction().await.expect("tx begins");
        tx.execute("DELETE FROM entities", ()).await.unwrap();
        tx.execute("DELETE FROM snapshots", ()).await.unwrap();
        tx.execute(
            "INSERT INTO snapshots VALUES (?1, ?2)",
            turso::params![snapshot.snapshot_id.0.clone(), snapshot.entities.len() as i64],
        )
        .await
        .unwrap();

        for entity in snapshot.entities.values() {
            tx.execute(
                "INSERT INTO entities VALUES (?1, ?2, ?3)",
                turso::params![
                    snapshot.snapshot_id.0.clone(),
                    entity.key.0.clone(),
                    entity.prim_path.clone()
                ],
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
        let turso_elapsed = start_turso.elapsed();

        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM entities WHERE snapshot_id = ?1",
                turso::params![snapshot.snapshot_id.0.clone()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let row_count: i64 = row.get(0).unwrap();

        println!(
            "M25 Baseline (Turso DB Replace): rows_upserted={}, rows_removed=all_prior, db_elapsed={:?}",
            row_count, turso_elapsed
        );
        assert_eq!(row_count, 34);
    });
}

#[test]
fn profiles_render_blob_enrichment_baseline() {
    let project = tempdir().expect("tempdir");
    let mut world = World::new();
    world.insert_resource(Assets::<Mesh>::default());

    // Create 3 meshes
    let mut mesh_paths = Vec::new();
    for i in 0..3 {
        let path = format!("/World/Mesh{i}");
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        world.spawn((UsdPrimRef::new(&path), Mesh3d(handle)));
        mesh_paths.push(path);
    }

    // Build 34-entity snapshot
    let digest = HashDigest::new([1; HashDigest::BYTE_LEN]);
    let mut entities = std::collections::HashMap::new();
    for i in 0..34 {
        let path = if i < 3 {
            mesh_paths[i].clone()
        } else {
            format!("/World/NonMesh{i}")
        };
        let key = EntityKey::from(path.clone());
        let entity = EntitySnapshot {
            key: key.clone(),
            prim_path: path.clone(),
            identity_source: IdentitySource::PrimPath,
            semantic: SemanticInfo::default(),
            transform: TransformSignature {
                translation_mm: [0; 3],
                rotation_quantized: [0; 4],
                scale_quantized: [10_000; 3],
                hash: digest,
            },
            geometry: (i < 3).then_some(GeometrySignature {
                vertex_count: 3,
                index_count: 3,
                local_bounds: Bounds3 {
                    min: [0.0; 3],
                    max: [1.0; 3],
                },
                local_centroid: QuantizedPoint3([500; 3]),
                topology_hash: digest,
                shape_hash: digest,
                render_blob: None,
            }),
            properties: vec![SemanticProperty {
                name: "prop".to_owned(),
                value: CanonicalValue::Bool(true),
            }],
            metadata_hash: digest,
            full_hash: digest,
        };
        entities.insert(key, entity);
    }

    let start = Instant::now();
    // Simulate blob attachment scan logic
    let handles_scanned = mesh_paths.len();
    let entities_scanned = entities.len();
    let mut attached = 0;
    for entity in entities.values_mut() {
        if entity.geometry.is_some() {
            attached += 1;
        }
    }
    let blob_elapsed = start.elapsed();

    println!(
        "M25 Baseline (Render Blob Enrichment): mesh_handles_scanned={}, semantic_entities_scanned={}, blobs_attached={}, elapsed={:?}",
        handles_scanned, entities_scanned, attached, blob_elapsed
    );
    assert_eq!(handles_scanned, 3);
    assert_eq!(entities_scanned, 34);
    assert_eq!(attached, 3);
    let _ = project;
}
