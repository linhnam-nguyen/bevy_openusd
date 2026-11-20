use bevy::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{PrimRoute, RouteCtx, UsdDisplayName, VisibilityRoute};

#[test]
fn affine_projection_preserves_shear_far_from_origin() {
    use crate::route::xform::propagation::propagate_usd_affine_transforms_system;
    use bevy::camera::visibility::{VisibilitySystems, update_frusta};
    use bevy::camera::{CameraProjectionPlugin, primitives::Frustum};
    use bevy::ecs::schedule::{InternedSystemSet, NodeId};
    use bevy::transform::{TransformPlugin, TransformSystems};

    for translation in [0, 100_000, -100_000, 1_000_000] {
        let source = format!(
            r#"#usda 1.0
def Xform "Shear" {{
    matrix4d xformOp:transform = ((1, 0, 0, 0), (0.5, 1, 0, 0), (0, 0, 1, 0), ({translation}, 0, 0, 1))
    uniform token[] xformOpOrder = ["xformOp:transform"]
}}
def Xform "PureTrs" {{
    double3 xformOp:translate = ({translation}, 0, 0)
    float xformOp:rotateZ = 37
    float3 xformOp:scale = (2, 3, 4)
    uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateZ", "xformOp:scale"]
}}
"#
        );
        let stage = crate::UsdSnippet::new(&source).open_stage().unwrap();
        let shear_path = openusd::sdf::path("/Shear").unwrap();
        let pure_trs_path = openusd::sdf::path("/PureTrs").unwrap();
        let shear = crate::read::xform::read_local_xform_at(&stage, &shear_path, None)
            .unwrap()
            .unwrap();
        assert!(
            shear.residual_affine.is_some() || shear.full_local_affine.is_some(),
            "shear was discarded at translation {translation}"
        );
        let pure_trs = crate::read::xform::read_local_xform_at(&stage, &pure_trs_path, None)
            .unwrap()
            .unwrap();
        assert!(
            pure_trs.residual_affine.is_none() && pure_trs.full_local_affine.is_none(),
            "pure TRS took the affine override path at translation {translation}"
        );

        let mut app = App::new();
        app.add_plugins((crate::UsdPlugin, TransformPlugin, CameraProjectionPlugin))
            .add_plugins(crate::LiveStagePlugin)
            .init_resource::<Assets<bevy::mesh::Mesh>>()
            .init_resource::<Assets<StandardMaterial>>();
        app.world_mut()
            .insert_non_send(crate::LiveStage::new(stage));
        let graph = app
            .get_schedule(PostUpdate)
            .expect("PostUpdate schedule")
            .graph();
        let system_node = |system_type| {
            let (key, _, _) = graph
                .systems
                .iter()
                .find(|(_, system, _)| system.system_type() == system_type)
                .expect("registered system");
            NodeId::System(key)
        };
        let set_node = |set: InternedSystemSet| {
            let key = graph
                .system_sets
                .get_key(set)
                .expect("registered system set");
            NodeId::Set(key)
        };
        let affine_node = system_node(propagate_usd_affine_transforms_system.system_type_id());
        let frusta_node = system_node(update_frusta.system_type_id());
        let update_frusta_set = set_node(VisibilitySystems::UpdateFrusta.intern());
        let transform_set = set_node(TransformSystems::Propagate.intern());
        let dependencies = graph.dependency().graph();
        assert!(dependencies.contains_edge(transform_set, affine_node));
        assert!(
            graph
                .hierarchy()
                .graph()
                .contains_edge(update_frusta_set, frusta_node)
        );
        assert!(
            dependencies.contains_edge(affine_node, update_frusta_set),
            "USD affine propagation must precede Bevy's camera frustum update"
        );
        assert!(
            dependencies.contains_edge(
                affine_node,
                set_node(VisibilitySystems::CalculateBounds.intern()),
            ),
            "USD affine propagation must precede dynamic skinned bounds"
        );
        app.update();

        let world = app.world_mut();
        let (shear_entity, shear_global) = world
            .query::<(Entity, &crate::UsdPrimRef, &GlobalTransform)>()
            .iter(world)
            .find_map(|(entity, prim, global)| (prim.path == "/Shear").then_some((entity, global)))
            .expect("sheared prim projected");
        let projected_basis = shear_global.affine().transform_vector3(Vec3::Y);
        assert!(
            projected_basis.abs_diff_eq(Vec3::new(0.5, 1.0, 0.0), 1e-5),
            "Y basis changed at translation {translation}: {projected_basis:?}"
        );
        assert_eq!(shear_global.translation(), Vec3::X * translation as f32);

        // A view offset along the sheared axis must see its corrected transform on its first frame.
        let projection = bevy::camera::Projection::Perspective(Default::default());
        let view = world
            .spawn((
                ChildOf(shear_entity),
                Transform::from_xyz(0.0, 4.0, 0.0),
                projection.clone(),
                Frustum::default(),
            ))
            .id();
        app.update();
        let world = app.world();
        let view_global = world.get::<GlobalTransform>(view).unwrap();
        let expected_translation = Vec3::new(translation as f32 + 2.0, 4.0, 0.0);
        assert!(
            view_global
                .translation()
                .abs_diff_eq(expected_translation, 1e-5),
            "view translation at {translation}: {:?}, expected {expected_translation:?}",
            view_global.translation()
        );
        let frustum = world.get::<Frustum>(view).unwrap();
        assert_eq!(frustum.0, projection.compute_frustum(view_global).0);
    }
}

#[test]
fn hierarchy_metadata_composed_reference_descendant_uses_composed_path() -> anyhow::Result<()> {
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
    let fixture_id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let fixture_root = std::env::temp_dir();
    let child_path: PathBuf = fixture_root.join(format!(
        "usd_bevy_c6_composed_child_{}_{}.usda",
        std::process::id(),
        fixture_id
    ));
    let parent_path: PathBuf = fixture_root.join(format!(
        "usd_bevy_c6_composed_parent_{}_{}.usda",
        std::process::id(),
        fixture_id
    ));
    let child_path_str = child_path.to_string_lossy().into_owned();
    let parent_path_str = parent_path.to_string_lossy().into_owned();

    let child = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .in_memory("c6-composed-child.usda")?;
    child
        .define_prim("/SceneRoot/Member_Kitchen")?
        .set_type_name("Xform")?
        .set_metadata(
            "ui:displayName",
            openusd::vt::Value::String("Kitchen_set".to_owned()),
        )?;
    child.root_layer().export(&child_path_str)?;

    let parent = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .in_memory("c6-composed-parent.usda")?;
    parent
        .define_prim("/SceneRoot")?
        .set_type_name("Xform")?
        .set_metadata(
            "ui:displayName",
            openusd::vt::Value::String("Pro3".to_owned()),
        )?;
    parent
        .define_prim("/SceneRoot/Member_Sc1")?
        .set_type_name("Xform")?
        .set_metadata(
            "references",
            openusd::vt::Value::ReferenceListOp(openusd::sdf::ReferenceListOp::prepended([
                openusd::sdf::Reference {
                    asset_path: child_path_str,
                    prim_path: openusd::sdf::path("/SceneRoot")?,
                    ..Default::default()
                },
            ])),
        )?;
    parent.root_layer().export(&parent_path_str)?;

    let stage = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(&parent_path_str)?;
    let composed_path = openusd::sdf::path("/SceneRoot/Member_Sc1/Member_Kitchen")?;
    assert!(stage.prim(composed_path.clone())?.is_defined()?);

    let mut world = World::new();
    super::prepare_hierarchy_metadata(&stage, &mut world);
    let index = world.resource::<super::hierarchy::HierarchyMetadataIndex>();
    assert_eq!(
        index.display_name(composed_path.as_str()),
        Some("Kitchen_set")
    );

    let ctx = RouteCtx::new(&stage, &composed_path);
    let entity = world.spawn_empty().id();
    VisibilityRoute.project(&ctx, &mut world, entity);
    assert_eq!(
        world.get::<UsdDisplayName>(entity),
        Some(&UsdDisplayName("Kitchen_set".to_owned()))
    );
    assert_eq!(ctx.prim_str(), "/SceneRoot/Member_Sc1/Member_Kitchen");

    let _ = std::fs::remove_file(child_path);
    let _ = std::fs::remove_file(parent_path);
    Ok(())
}

#[test]
fn projects_three_prim_hierarchy() {
    let stage = usd_support::open_fixture_stage("hierarchy.usda");
    assert_eq!(
        stage.stage_metadata("defaultPrim").unwrap(),
        Some(openusd::vt::Value::Token("World".into()))
    );
    assert!(!stage.layer_stack().is_empty());

    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(crate::LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let mut by_path = std::collections::HashMap::new();
    for (entity, prim) in world.query::<(Entity, &crate::UsdPrimRef)>().iter(world) {
        if prim.path != "/" {
            by_path.insert(prim.path.clone(), entity);
        }
    }
    assert!(by_path.contains_key("/World"));
    assert!(by_path.contains_key("/World/ChildA"));
    assert!(by_path.contains_key("/World/ChildB"));
    assert_eq!(by_path.len(), 3);

    let world_children = app
        .world()
        .get::<Children>(by_path["/World"])
        .map(|c| c.len())
        .unwrap_or(0);
    assert_eq!(world_children, 2);
    assert_eq!(by_path["/World/ChildA"] != by_path["/World/ChildB"], true);

    // U40 + U41: Verify non-TRS residual affine (shear) and !resetXformStack! propagation.
    let xform_usda = r#"#usda 1.0
(
    defaultPrim = "World"
    upAxis = "Y"
)

def Xform "World" {
    double3 xformOp:translate = (10, 20, 30)
    uniform token[] xformOpOrder = ["xformOp:translate"]

    def Xform "ResetChild" {
        double3 xformOp:translate = (1, 2, 3)
        uniform token[] xformOpOrder = ["!resetXformStack!", "xformOp:translate"]
    }

    def Xform "ShearedChild" {
        matrix4d xformOp:transform = ( (1, 0, 0, 0), (0.5, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )
        uniform token[] xformOpOrder = ["xformOp:transform"]
    }
}
"#;
    let xform_stage = crate::UsdSnippet::new(xform_usda)
        .open_stage()
        .unwrap();
    let mut xform_app = App::new();
    xform_app
        .add_plugins((crate::UsdPlugin, bevy::transform::TransformPlugin))
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    xform_app
        .world_mut()
        .insert_non_send(crate::LiveStage::new(xform_stage));
    xform_app.update();

    let xw = xform_app.world_mut();
    let mut globals = std::collections::HashMap::new();
    let mut has_residual = false;
    let mut has_reset = false;
    for (prim, gt, res, rst) in xw
        .query::<(
            &crate::UsdPrimRef,
            &GlobalTransform,
            Option<&crate::route::xform::UsdResidualAffine>,
            Option<&crate::route::xform::UsdResetXformStack>,
        )>()
        .iter(xw)
    {
        globals.insert(prim.path.clone(), *gt);
        if prim.path == "/World/ShearedChild" && res.is_some() {
            has_residual = true;
        }
        if prim.path == "/World/ResetChild" && rst.is_some() {
            has_reset = true;
        }
    }
    assert!(has_residual, "ShearedChild carries UsdResidualAffine (U40)");
    assert!(has_reset, "ResetChild carries UsdResetXformStack (U41)");
    assert_eq!(
        globals["/World/ResetChild"].translation(),
        Vec3::new(1.0, 2.0, 3.0),
        "ResetChild ignores parent translation of (10, 20, 30)"
    );
    let sheared_point = globals["/World/ShearedChild"].transform_point(Vec3::new(0.0, 2.0, 0.0));
    assert!(
        (sheared_point - Vec3::new(11.0, 22.0, 30.0)).length() < 1e-3,
        "ShearedChild preserves Y-to-X 0.5 shear in GlobalTransform: got {sheared_point:?}"
    );

    // F-C03-I1-04: Verify read_int checked i64-to-i32 boundary and overflow rejection.
    let int_stage = crate::UsdSnippet::new(
        r#"#usda 1.0
(
    defaultPrim = "World"
)
def Xform "World" {
    int scalar_int = 42
    int64 scalar_i64 = 42
    int64 min_i32 = -2147483648
    int64 max_i32 = 2147483647
    int64 underflow_i64 = -2147483649
    int64 overflow_i64 = 2147483648
}
"#,
    )
    .open_stage()
    .unwrap();
    let world_path = openusd::sdf::path("/World").unwrap();
    assert_eq!(
        crate::read::util::read_int(&int_stage, &world_path, "scalar_int").unwrap(),
        Some(42)
    );
    assert_eq!(
        crate::read::util::read_int(&int_stage, &world_path, "scalar_i64").unwrap(),
        Some(42)
    );
    assert_eq!(
        crate::read::util::read_int(&int_stage, &world_path, "min_i32").unwrap(),
        Some(i32::MIN)
    );
    assert_eq!(
        crate::read::util::read_int(&int_stage, &world_path, "max_i32").unwrap(),
        Some(i32::MAX)
    );
    assert_eq!(
        crate::read::util::read_int(&int_stage, &world_path, "unauthored_int").unwrap(),
        None
    );
    assert!(
        crate::read::util::read_int(&int_stage, &world_path, "underflow_i64").is_err(),
        "i32::MIN - 1 must return Err, never wrap"
    );
    assert!(
        crate::read::util::read_int(&int_stage, &world_path, "overflow_i64").is_err(),
        "i32::MAX + 1 must return Err, never wrap"
    );
    int_stage
        .prim(world_path.clone())
        .unwrap()
        .attribute("overflow_i64")
        .set(openusd::vt::Value::Int64(i64::MAX))
        .unwrap();
    assert!(
        crate::read::util::read_int(&int_stage, &world_path, "overflow_i64").is_err(),
        "i64::MAX must return Err, never wrap"
    );
    // Multi-step recovery: authoring a valid i64 after an out-of-range value recovers cleanly.
    int_stage
        .prim(world_path.clone())
        .unwrap()
        .attribute("overflow_i64")
        .set(openusd::vt::Value::Int64(99))
        .unwrap();
    assert_eq!(
        crate::read::util::read_int(&int_stage, &world_path, "overflow_i64").unwrap(),
        Some(99)
    );

    // F-C03-I1-02: Verify tiny/singular determinant shear (UsdFullLocalAffine),
    // non-finite (NaN/Inf) and projective matrix rejection, multi-step transition
    // cleanup, and nested !resetXformStack! with Z-up stage root conversion.
    let f02_usda = r#"#usda 1.0
(
    defaultPrim = "World"
    upAxis = "Z"
)

def Xform "World" {
    double3 xformOp:translate = (10, 20, 30)
    uniform token[] xformOpOrder = ["xformOp:translate"]

    def Xform "SingularShear" {
        matrix4d xformOp:transform = ( (1, 0, 0, 0), (0.5, 0, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )
        uniform token[] xformOpOrder = ["xformOp:transform"]
    }

    def Xform "TinyScaleShear" {
        matrix4d xformOp:transform = ( (0.001, 0, 0, 0), (0.5, 0.001, 0, 0), (0, 0, 0.001, 0), (0, 0, 0, 1) )
        uniform token[] xformOpOrder = ["xformOp:transform"]
    }

    def Xform "NestedParent" {
        double3 xformOp:translate = (100, 200, 300)
        uniform token[] xformOpOrder = ["xformOp:translate"]

        def Xform "NestedReset" {
            double3 xformOp:translate = (1, 2, 3)
            uniform token[] xformOpOrder = ["!resetXformStack!", "xformOp:translate"]
        }
    }
}
"#;
    let f02_stage = crate::UsdSnippet::new(f02_usda).open_stage().unwrap();
    let singular_path = openusd::sdf::path("/World/SingularShear").unwrap();
    let singular_eval = crate::read::xform::read_local_xform_at(&f02_stage, &singular_path, None)
        .unwrap()
        .unwrap();
    assert!(
        singular_eval.full_local_affine.is_some(),
        "zero-scale axis + shear must populate full_local_affine (never silently drop shear)"
    );

    let mut f02_app = App::new();
    f02_app
        .add_plugins((crate::UsdPlugin, bevy::transform::TransformPlugin))
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    f02_app
        .world_mut()
        .insert_non_send(crate::LiveStage::new(f02_stage));
    f02_app.update();

    let (singular_entity, singular_gt, has_full_override) = {
        let fw = f02_app.world_mut();
        let mut found = None;
        for (entity, prim, gt, full) in fw
            .query::<(
                Entity,
                &crate::UsdPrimRef,
                &GlobalTransform,
                Option<&crate::route::xform::UsdFullLocalAffine>,
            )>()
            .iter(fw)
        {
            if prim.path == "/World/SingularShear" {
                found = Some((entity, *gt, full.is_some()));
            }
        }
        found.expect("SingularShear projected")
    };
    assert!(
        has_full_override,
        "SingularShear entity carries UsdFullLocalAffine"
    );
    // In Z-up stage, stage root "/" applies -90 deg X rotation: (x, y, z)_usd -> (x, z, -y)_bevy.
    // For point (0, 2, 0) in SingularShear local space:
    // SingularShear local matrix maps (0, 2, 0) -> (1, 0, 0).
    // Parent /World adds translation (10, 20, 30) -> (11, 20, 30) in USD Z-up space.
    // Stage root Z-up -> Y-up maps (11, 20, 30) -> (11, 30, -20).
    let singular_pt = singular_gt.transform_point(Vec3::new(0.0, 2.0, 0.0));
    assert!(
        (singular_pt - Vec3::new(11.0, 30.0, -20.0)).length() < 1e-3,
        "SingularShear preserves Y-to-X 0.5 shear even with zero Y scale: got {singular_pt:?}"
    );

    // Check TinyScaleShear (determinant = 1e-9 <= 1e-8) also preserves shear.
    let tiny_gt = {
        let fw = f02_app.world_mut();
        fw.query::<(&crate::UsdPrimRef, &GlobalTransform)>()
            .iter(fw)
            .find(|(p, _)| p.path == "/World/TinyScaleShear")
            .map(|(_, gt)| *gt)
            .expect("TinyScaleShear projected")
    };
    let tiny_pt = tiny_gt.transform_point(Vec3::new(0.0, 2.0, 0.0));
    assert!(
        (tiny_pt - Vec3::new(11.0, 30.0, -20.002)).length() < 1e-4,
        "TinyScaleShear preserves Y-to-X 0.5 shear at det=1e-9: got {tiny_pt:?}"
    );

    // Check NestedReset under /World/NestedParent ignores /World and /World/NestedParent
    // translations but preserves the stage root Z-up -> Y-up basis once:
    // local (1, 2, 3) -> stage root Z-up basis -> (1, 3, -2).
    let nested_reset_gt = {
        let fw = f02_app.world_mut();
        fw.query::<(&crate::UsdPrimRef, &GlobalTransform)>()
            .iter(fw)
            .find(|(p, _)| p.path == "/World/NestedParent/NestedReset")
            .map(|(_, gt)| *gt)
            .expect("NestedReset projected")
    };
    assert!(
        (nested_reset_gt.translation() - Vec3::new(1.0, 3.0, -2.0)).length() < 1e-3,
        "NestedReset ignores ancestor transforms while keeping stage root Z-up basis once: got {:?}",
        nested_reset_gt.translation()
    );

    // Multi-step transition: transition SingularShear -> well-conditioned shear -> pure TRS,
    // verifying UsdFullLocalAffine and UsdResidualAffine are cleaned up atomically.
    let well_sheared_eval = {
        let stage = &f02_app
            .world()
            .get_non_send::<crate::LiveStage>()
            .unwrap()
            .stage;
        stage
            .prim(singular_path.clone())
            .unwrap()
            .attribute("xformOp:transform")
            .set(openusd::vt::Value::Matrix4d(openusd::gf::Mat4d([
                1.0, 0.0, 0.0, 0.0, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ])))
            .unwrap();
        crate::read::xform::read_local_xform_at(stage, &singular_path, None).unwrap()
    };
    crate::route::xform::apply_local_xform_evaluation(
        f02_app.world_mut(),
        singular_entity,
        well_sheared_eval,
    );
    assert!(
        f02_app
            .world()
            .get::<crate::route::xform::UsdFullLocalAffine>(singular_entity)
            .is_none(),
        "transition to well-conditioned shear removes UsdFullLocalAffine"
    );
    assert!(
        f02_app
            .world()
            .get::<crate::route::xform::UsdResidualAffine>(singular_entity)
            .is_some(),
        "transition to well-conditioned shear inserts UsdResidualAffine"
    );

    let pure_trs_eval = {
        let stage = &f02_app
            .world()
            .get_non_send::<crate::LiveStage>()
            .unwrap()
            .stage;
        stage
            .prim(singular_path.clone())
            .unwrap()
            .attribute("xformOp:transform")
            .set(openusd::vt::Value::Matrix4d(openusd::gf::Mat4d([
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 4.0, 5.0, 6.0, 1.0,
            ])))
            .unwrap();
        crate::read::xform::read_local_xform_at(stage, &singular_path, None).unwrap()
    };
    crate::route::xform::apply_local_xform_evaluation(
        f02_app.world_mut(),
        singular_entity,
        pure_trs_eval,
    );
    assert!(
        f02_app
            .world()
            .get::<crate::route::xform::UsdResidualAffine>(singular_entity)
            .is_none(),
        "transition to pure TRS removes UsdResidualAffine"
    );
    assert!(
        f02_app
            .world()
            .get::<crate::route::xform::UsdFullLocalAffine>(singular_entity)
            .is_none(),
        "transition to pure TRS keeps UsdFullLocalAffine absent"
    );

    // Non-finite (NaN / Inf) and projective (non-affine) matrices must be rejected with Err.
    {
        let stage = &f02_app
            .world()
            .get_non_send::<crate::LiveStage>()
            .unwrap()
            .stage;
        stage
            .prim(singular_path.clone())
            .unwrap()
            .attribute("xformOp:transform")
            .set(openusd::vt::Value::Matrix4d(openusd::gf::Mat4d([
                f64::NAN,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ])))
            .unwrap();
        assert!(
            crate::read::xform::read_local_xform_at(stage, &singular_path, None).is_err(),
            "NaN matrix element must be rejected with Err"
        );

        stage
            .prim(singular_path.clone())
            .unwrap()
            .attribute("xformOp:transform")
            .set(openusd::vt::Value::Matrix4d(openusd::gf::Mat4d([
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                f64::INFINITY,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ])))
            .unwrap();
        assert!(
            crate::read::xform::read_local_xform_at(stage, &singular_path, None).is_err(),
            "Inf matrix element must be rejected with Err"
        );

        stage
            .prim(singular_path.clone())
            .unwrap()
            .attribute("xformOp:transform")
            .set(openusd::vt::Value::Matrix4d(openusd::gf::Mat4d([
                1.0, 0.0, 0.0, 0.25, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ])))
            .unwrap();
        assert!(
            crate::read::xform::read_local_xform_at(stage, &singular_path, None).is_err(),
            "Projective (non-affine) matrix must be rejected with Err"
        );
    }
}

#[test]
fn attaches_display_name_component_and_omits_plain() {
    let stage = usd_support::open_fixture_stage("usdui.usda");
    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(crate::LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let mut by_path = std::collections::HashMap::new();
    for (prim, dn) in world
        .query::<(&crate::UsdPrimRef, Option<&UsdDisplayName>)>()
        .iter(world)
    {
        by_path.insert(prim.path.clone(), dn.map(|d| d.0.clone()));
    }
    assert_eq!(
        by_path.get("/World").and_then(|o| o.as_deref()),
        Some("Top of the world")
    );
    assert_eq!(
        by_path
            .get("/World/robot_v3_baseLink_geom_0")
            .and_then(|o| o.as_deref()),
        Some("Main Body")
    );
    assert_eq!(by_path.get("/World/Plain").and_then(|o| o.as_deref()), None);
}

#[test]
fn applies_visibility_and_reads_kind() {
    let stage = usd_support::open_fixture_stage("visibility_kind.usda");
    assert_eq!(
        crate::read::geom::read_kind(&stage, &openusd::sdf::path("/World").unwrap())
            .unwrap()
            .as_deref(),
        Some("assembly")
    );
    assert_eq!(
        crate::read::geom::read_kind(&stage, &openusd::sdf::path("/World/Robot").unwrap())
            .unwrap()
            .as_deref(),
        Some("component")
    );
    assert!(
        crate::read::geom::read_kind(&stage, &openusd::sdf::path("/World/Visible").unwrap())
            .unwrap()
            .is_none()
    );

    let mut app = App::new();
    app.add_plugins(crate::UsdPlugin)
        .add_plugins(crate::LiveStagePlugin)
        .init_resource::<Assets<bevy::mesh::Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(crate::LiveStage::new(stage));
    app.update();

    let world = app.world_mut();
    let mut by_path = std::collections::HashMap::new();
    for (prim, vis) in world.query::<(&crate::UsdPrimRef, &Visibility)>().iter(world) {
        by_path.insert(prim.path.clone(), *vis);
    }
    assert_eq!(by_path["/World/Visible"], Visibility::Inherited);
    assert_eq!(by_path["/World/Hidden"], Visibility::Hidden);
    assert_eq!(by_path["/World/HiddenParent"], Visibility::Hidden);
    assert_eq!(
        by_path["/World/HiddenParent/InheritsHidden"],
        Visibility::Inherited
    );
}
