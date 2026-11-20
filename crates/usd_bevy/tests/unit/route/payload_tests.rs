use bevy::prelude::*;
use openusd::sdf;
use openusd::usd::{InitialLoadSet, Stage};

use super::{PayloadRoute, UsdPayloadError, UsdPayloadUnloaded, is_unloaded};
use crate::live::LiveStage;
use crate::route::{PrimRoute, RouteCtx};

fn open_payload_stage(load_set: InitialLoadSet) -> (tempfile::TempDir, Stage) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let sub_path = temp_dir.path().join("sub.usda");
    let main_path = temp_dir.path().join("main.usda");

    std::fs::write(
        &sub_path,
        r#"#usda 1.0
(
    defaultPrim = "Sub"
)
def Xform "Sub"
{
    def Mesh "Child"
    {
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
    }
}
"#,
    )
    .expect("write sub layer");

    std::fs::write(
        &main_path,
        r#"#usda 1.0
def Xform "World"
{
    def Xform "PayloadPrim" (
        payload = @./sub.usda@
    )
    {
    }
}
"#,
    )
    .expect("write main layer");

    let stage = Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .load(load_set)
        .open(main_path.to_str().expect("utf8 main path"))
        .expect("open payload stage");
    (temp_dir, stage)
}

#[test]
fn payload_route_distinguishes_loaded_unloaded_and_error_states() {
    let (_dir_loaded, loaded_stage) = open_payload_stage(InitialLoadSet::LoadAll);
    let (_dir_unloaded, unloaded_stage) = open_payload_stage(InitialLoadSet::LoadNone);
    let prim_path = sdf::path("/World/PayloadPrim").expect("prim path");

    // 1. Positive loaded prim -> Ok(false)
    let loaded_ctx = RouteCtx::new(&loaded_stage, &prim_path);
    assert_eq!(
        is_unloaded(&loaded_ctx).expect("loaded state query"),
        false
    );

    // 2. Positive unloaded prim -> Ok(true)
    let unloaded_ctx = RouteCtx::new(&unloaded_stage, &prim_path);
    assert_eq!(
        is_unloaded(&unloaded_ctx).expect("unloaded state query"),
        true
    );

    let mut world = World::new();
    let entity = world.spawn_empty().id();

    // Unloaded projection inserts UsdPayloadUnloaded and has no UsdPayloadError.
    PayloadRoute.project(&unloaded_ctx, &mut world, entity);
    assert!(world.entity(entity).contains::<UsdPayloadUnloaded>());
    assert!(!world.entity(entity).contains::<UsdPayloadError>());

    // 3. Negative invalid-path lookup -> Err, never clears UsdPayloadUnloaded or claims loaded.
    let invalid_path = sdf::Path::default();
    let err_ctx = RouteCtx::new(&unloaded_stage, &invalid_path);
    assert!(
        is_unloaded(&err_ctx).is_err(),
        "invalid prim path query must propagate Err rather than defaulting to loaded (false)"
    );
    PayloadRoute.project(&err_ctx, &mut world, entity);
    assert!(
        world.entity(entity).contains::<UsdPayloadUnloaded>(),
        "error must preserve prior unloaded state instead of reclassifying as loaded"
    );
    assert!(
        world.entity(entity).contains::<UsdPayloadError>(),
        "error must be recorded explicitly via UsdPayloadError"
    );

    // 4. Subsequent loaded projection clears both UsdPayloadUnloaded and UsdPayloadError.
    PayloadRoute.project(&loaded_ctx, &mut world, entity);
    assert!(!world.entity(entity).contains::<UsdPayloadUnloaded>());
    assert!(!world.entity(entity).contains::<UsdPayloadError>());

    // 5. Negative OpenUSD prim.is_loaded() composition failure -> Err + UsdPayloadError.
    let (_dir_corrupt, corrupt_stage) = open_payload_stage(InitialLoadSet::LoadNone);
    let root_id = corrupt_stage.root_layer().identifier().to_owned();
    let root_layer = corrupt_stage
        .layer(&root_id)
        .expect("root layer lookup")
        .expect("root layer present");
    root_layer
        .edit(|edit| {
            let mut prim = edit
                .prim_mut(prim_path.clone())?
                .expect("payload prim spec exists");
            prim.set("active", openusd::vt::Value::Int(42));
            Ok(())
        })
        .expect("corrupt active metadata");
    let corrupt_ctx = RouteCtx::new(&corrupt_stage, &prim_path);
    assert!(
        is_unloaded(&corrupt_ctx).is_err(),
        "OpenUSD prim.is_loaded() error must propagate rather than returning false"
    );
    let mut corrupt_world = World::new();
    let corrupt_entity = corrupt_world.spawn(UsdPayloadUnloaded).id();
    PayloadRoute.project(&corrupt_ctx, &mut corrupt_world, corrupt_entity);
    assert!(corrupt_world.entity(corrupt_entity).contains::<UsdPayloadUnloaded>());
    assert!(corrupt_world.entity(corrupt_entity).contains::<UsdPayloadError>());
}

#[test]
fn live_stage_load_and_unload_payload_propagate_errors() {
    let (_dir, stage) = open_payload_stage(InitialLoadSet::LoadAll);
    let live = LiveStage::new(stage);

    assert!(live.load_payload("not a valid path").is_err());
    assert!(live.unload_payload("not a valid path").is_err());
    assert!(!live.has_changes());

    assert!(live.unload_payload("/World/PayloadPrim").is_ok());
    assert!(live.has_changes());
    let _ = live.drain_change_batch();

    assert!(live.load_payload("/World/PayloadPrim").is_ok());
    assert!(live.has_changes());
}
