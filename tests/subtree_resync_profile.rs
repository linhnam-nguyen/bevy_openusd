//! Milestone 25-O0 baseline profiling for USD resync reconciliation.
//!
//! Measures and records baseline whole-frame execution timings across synthetic
//! wide, deep-overlap, and real materials stages before subtree-resync
//! optimizations are applied.

use std::time::Instant;

use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use openusd::usd::Stage;
use usd_bevy::{LiveStage, LiveStagePlugin, PrimEntities, UsdPlugin, UsdSnippet};

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
