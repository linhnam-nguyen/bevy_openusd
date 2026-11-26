use std::fs;
use std::path::Path;

use anyhow::Result;
use bevy::prelude::*;
use openusd::{sdf, usd::Stage, vt::Value};
use project_protocol::{
    PlacementSpec, ProjectCommitRequest, ProjectCommitTarget, ProjectDeleteSceneRequest,
    ProjectWriteTarget,
};
use usd_bevy::{LiveStage, LiveStagePlugin, ProjectionBudget, ProjectionReadiness, UsdPlugin};
use usd_git::GitRepository;
use usd_project::{ProjectRoot, SceneId};

use crate::animation::{UsdStageTime, tests::tick_stage_time};
use crate::cache::contract::{
    CachedTransform, SceneCacheActivation, SceneCacheAddress, SceneCacheDescriptorV3,
    SceneCacheEntry, SceneCacheEntryKind, SceneCacheIndex, SceneCacheOccurrence, SceneCacheState,
};
use crate::document::stages::metadata::TimeCodeRange;
use crate::project::catalog::manifest_store::ManifestStore;
use crate::project::models::wrapper::model_wrapper_path;
use crate::project::scene::authoring::{scene_member_path, scene_path};
use crate::project::scene::inspection::inspect_composition;
use crate::project::scene::timeline::{map_child_domain_to_parent, retime_scene_member_atomic};
use crate::project::service::{ProjectApplicationService, ProjectModelPreparationQueue};
use crate::viewport::stage::SceneCachePresentation;
use usd_model::HashDigest;

fn write_animated_xform_usda(
    path: &Path,
    prim_name: &str,
    start: f64,
    end: f64,
    tcps: f64,
    fps: Option<f64>,
    end_x: f64,
) -> Result<()> {
    let fps_line = fps
        .map(|rate| format!("    framesPerSecond = {rate}\n"))
        .unwrap_or_default();
    let content = format!(
        r#"#usda 1.0
(
    defaultPrim = "{prim_name}"
    startTimeCode = {start}
    endTimeCode = {end}
    timeCodesPerSecond = {tcps}
{fps_line}    metersPerUnit = 1
    upAxis = "Y"
)

def Xform "{prim_name}"
{{
    double3 xformOp:translate.timeSamples = {{
        {start}: (0, 0, 0),
        {end}: ({end_x}, 0, 0),
    }}
    uniform token[] xformOpOrder = ["xformOp:translate"]

    def Cube "Body"
    {{
        double size = 1.0
    }}
}}
"#
    );
    fs::write(path, content)?;
    Ok(())
}

fn write_rigid_skeletal_visibility_usda(path: &Path) -> Result<()> {
    let content = r#"#usda 1.0
(
    defaultPrim = "Rig"
    startTimeCode = 0
    endTimeCode = 48
    timeCodesPerSecond = 24
    framesPerSecond = 24
    metersPerUnit = 1
    upAxis = "Y"
)

def Xform "Rig"
{
    def Xform "Mover"
    {
        double3 xformOp:translate.timeSamples = {
            0: (0, 0, 0),
            48: (12, 0, 0),
        }
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }

    def Cube "Beacon"
    {
        double size = 1.0
        token visibility.timeSamples = {
            0: "inherited",
            24: "invisible",
            48: "inherited",
        }
    }

    def SkelRoot "Arm"
    {
        def Skeleton "Skel"
        {
            uniform token[] joints = ["Root", "Root/Tip"]
            uniform matrix4d[] bindTransforms = [
                ((1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1)),
                ((1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 1, 0, 1)),
            ]
            uniform matrix4d[] restTransforms = [
                ((1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1)),
                ((1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 1, 0, 1)),
            ]
            rel skel:animationSource = </Rig/Arm/Anim>
        }

        def SkelAnimation "Anim"
        {
            uniform token[] joints = ["Root", "Root/Tip"]
            float3[] translations.timeSamples = {
                0: [(0, 0, 0), (0, 1, 0)],
                48: [(0, 0, 0), (0, 5, 0)],
            }
            quatf[] rotations = [(1, 0, 0, 0), (1, 0, 0, 0)]
            half3[] scales = [(1, 1, 1), (1, 1, 1)]
        }

        def Mesh "Skin" (
            prepend apiSchemas = ["SkelBindingAPI"]
        )
        {
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
            rel skel:skeleton = </Rig/Arm/Skel>
            int[] primvars:skel:jointIndices = [1, 1, 1] (
                elementSize = 1
                interpolation = "vertex"
            )
            float[] primvars:skel:jointWeights = [1, 1, 1] (
                elementSize = 1
                interpolation = "vertex"
            )
        }
    }
}
"#;
    fs::write(path, content)?;
    Ok(())
}

fn root_scene_id(project_root: &Path) -> Result<SceneId> {
    let manifest = ManifestStore::read_validated(project_root)?;
    let ProjectRoot::Scene(scene_id) = manifest.raw().root else {
        anyhow::bail!("expected Project root Scene");
    };
    Ok(scene_id)
}

fn evaluate_stage_in_live_app(
    stage_path: &Path,
    sample_seconds: f64,
    with_cached_presentation: bool,
) -> Result<(
    UsdStageTime,
    Vec<(String, Vec3)>,
    Vec<(String, Vec3)>,
    Vec<(String, Visibility)>,
)> {
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
        .insert_resource(ProjectionBudget::unlimited())
        .insert_resource(Time::<()>::default())
        .init_resource::<UsdStageTime>()
        .add_systems(
            Update,
            tick_stage_time.after(usd_bevy::LiveStageSet::Reconcile),
        );

    let stage = Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(stage_path.to_str().expect("valid UTF-8 stage path"))?;
    app.world_mut().insert_non_send(LiveStage::new(stage));

    if with_cached_presentation {
        let scene_id = SceneId::new_v4();
        let mut descriptor = SceneCacheDescriptorV3::invalidated(
            scene_id,
            1,
            HashDigest::new([0; HashDigest::BYTE_LEN]),
        );
        descriptor.state = SceneCacheState::Ready;
        // Populate nonempty static cache entries covering the root and animated
        // descendant paths to verify cached residency never overwrites live animation.
        let mut entries = Vec::new();
        let mut prim_paths = Vec::new();
        let _ = app
            .world()
            .get_non_send::<LiveStage>()
            .expect("live stage")
            .stage
            .traverse(openusd::usd::PrimPredicate::default(), |path: &sdf::Path| {
                prim_paths.push(path.as_str().to_owned());
            });
        for prim_path in prim_paths {
            entries.push(SceneCacheEntry {
                address: SceneCacheAddress {
                    scene_id,
                    occurrence: SceneCacheOccurrence::PrimPath(prim_path.clone()),
                },
                parent: None,
                transform: CachedTransform::Placement(usd_project::ScenePlacementTransform::default()),
                bounds: None,
                cacheable: true,
                bim_enabled: false,
                geometry: None,
                material: None,
                animation: None,
                semantic_key: None,
                kind: SceneCacheEntryKind::OwnedPrim { prim_path },
                content_hash: None,
            });
        }
        descriptor.prim_count = entries.len() as u64;
        descriptor.prim_count_ready = true;
        assert!(
            !entries.is_empty(),
            "nonempty SceneCachePresentation fixture must contain resident entries"
        );
        let index = SceneCacheIndex {
            schema_version: crate::cache::contract::SCENE_CACHE_INDEX_SCHEMA_VERSION,
            scene_id,
            generation: 1,
            entries,
        };
        let activation = SceneCacheActivation { descriptor, index };
        app.world_mut()
            .insert_resource(SceneCachePresentation::from_activation(&activation));
    }

    for _ in 0..512 {
        app.update();
        if app
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness()
            == ProjectionReadiness::Ready
        {
            break;
        }
    }
    assert_eq!(
        app.world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Ready,
        "projection did not settle"
    );

    {
        let mut clock = app.world_mut().resource_mut::<UsdStageTime>();
        clock.playing = false;
        clock.seconds = sample_seconds;
    }
    app.update();
    app.update();

    let clock = *app.world().resource::<UsdStageTime>();
    let mut transforms = Vec::new();
    let mut visibilities = Vec::new();
    let mut global_transforms = Vec::new();
    {
        let mut query = app.world_mut().query::<(
            &usd_bevy::UsdPrimRef,
            &Transform,
            &GlobalTransform,
            Option<&Visibility>,
        )>();
        for (prim_ref, transform, global_transform, visibility) in query.iter(app.world()) {
            transforms.push((prim_ref.path.clone(), transform.translation));
            global_transforms.push((prim_ref.path.clone(), global_transform.translation()));
            if let Some(vis) = visibility {
                visibilities.push((prim_ref.path.clone(), *vis));
            }
        }
    }
    let _ = global_transforms;
    let mut joints = Vec::new();
    {
        let mut joint_query = app
            .world_mut()
            .query::<(&usd_bevy::route::skel::UsdJoint, &Transform)>();
        for (joint, transform) in joint_query.iter(app.world()) {
            joints.push((joint.path.clone(), transform.translation));
        }
    }
    Ok((clock, transforms, joints, visibilities))
}

fn evaluate_stage_world_transforms_in_live_app(
    stage_path: &Path,
    sample_seconds: f64,
) -> Result<(UsdStageTime, Vec<(String, Vec3)>)> {
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
        .insert_resource(ProjectionBudget::unlimited())
        .insert_resource(Time::<()>::default())
        .init_resource::<UsdStageTime>()
        .add_systems(
            Update,
            tick_stage_time.after(usd_bevy::LiveStageSet::Reconcile),
        );

    let stage = Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(stage_path.to_str().expect("valid UTF-8 stage path"))?;
    app.world_mut().insert_non_send(LiveStage::new(stage));

    for _ in 0..512 {
        app.update();
        if app
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness()
            == ProjectionReadiness::Ready
        {
            break;
        }
    }
    assert_eq!(
        app.world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Ready,
        "projection did not settle"
    );

    {
        let mut clock = app.world_mut().resource_mut::<UsdStageTime>();
        clock.playing = false;
        clock.seconds = sample_seconds;
    }
    app.update();
    app.update();

    let clock = *app.world().resource::<UsdStageTime>();
    let mut world_transforms = Vec::new();
    let mut query = app
        .world_mut()
        .query::<(&usd_bevy::UsdPrimRef, &GlobalTransform)>();
    for (prim_ref, global_transform) in query.iter(app.world()) {
        world_transforms.push((prim_ref.path.clone(), global_transform.translation()));
    }
    Ok((clock, world_transforms))
}

#[test]
fn map_child_domain_handles_rates_offsets_negative_scale_and_rejects_invalid() -> Result<()> {
    // Differing rates: child 30 tcps [0, 900] (30s) placed at 5s in 24 tcps parent (offset = 120)
    let mapped = map_child_domain_to_parent(
        TimeCodeRange::new(0.0, 900.0)?,
        30.0,
        24.0,
        sdf::LayerOffset::new(120.0, 1.0),
    )?;
    assert!((mapped.start - 120.0).abs() < 1e-6);
    assert!((mapped.end - 840.0).abs() < 1e-6);

    // Negative scale orders endpoints [min, max]
    let reversed = map_child_domain_to_parent(
        TimeCodeRange::new(0.0, 100.0)?,
        24.0,
        24.0,
        sdf::LayerOffset::new(200.0, -0.5),
    )?;
    assert!((reversed.start - 150.0).abs() < 1e-6);
    assert!((reversed.end - 200.0).abs() < 1e-6);

    // Zero or non-finite scale/rate rejected
    assert!(
        map_child_domain_to_parent(
            TimeCodeRange::new(0.0, 10.0)?,
            24.0,
            24.0,
            sdf::LayerOffset::new(0.0, 0.0),
        )
        .is_err()
    );
    assert!(
        map_child_domain_to_parent(
            TimeCodeRange::new(0.0, 10.0)?,
            0.0,
            24.0,
            sdf::LayerOffset::IDENTITY,
        )
        .is_err()
    );
    assert!(
        map_child_domain_to_parent(
            TimeCodeRange::new(0.0, 10.0)?,
            24.0,
            f64::NAN,
            sdf::LayerOffset::IDENTITY,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn acceptance_example_crane_turbine_bridge_aggregates_to_zero_to_forty_seconds() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let registry_path = dir.path().join("registry.json");
    let mut service = ProjectApplicationService::open(&registry_path)?;
    let project = service.create_project(dir.path(), "PortSite")?;
    let project_root = dir.path().join("PortSite");
    let root_id = root_scene_id(&project_root)?;

    // Create 3 animated USD sources with differing timeCodesPerSecond:
    // - Crane:   10 s at 24 tcps -> [0, 240]
    // - Turbine: 30 s at 30 tcps -> [0, 900]
    // - Bridge:  20 s at 60 tcps -> [0, 1200]
    let crane_src = dir.path().join("crane.usda");
    let turbine_src = dir.path().join("turbine.usda");
    let bridge_src = dir.path().join("bridge.usda");
    write_animated_xform_usda(&crane_src, "Crane", 0.0, 240.0, 24.0, Some(24.0), 10.0)?;
    write_animated_xform_usda(&turbine_src, "Turbine", 0.0, 900.0, 30.0, Some(30.0), 30.0)?;
    write_animated_xform_usda(&bridge_src, "Bridge", 0.0, 1200.0, 60.0, Some(60.0), 20.0)?;

    let crane_insp = inspect_composition(&crane_src)?;
    let turbine_insp = inspect_composition(&turbine_src)?;
    let bridge_insp = inspect_composition(&bridge_src)?;

    let crane = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &crane_src,
        &crane_insp,
        "Crane".to_owned(),
        "op-crane".to_owned(),
        1,
        PlacementSpec::default(),
    )?;
    let turbine = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &turbine_src,
        &turbine_insp,
        "Turbine".to_owned(),
        "op-turbine".to_owned(),
        2,
        PlacementSpec::default(),
    )?;
    let bridge = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &bridge_src,
        &bridge_insp,
        "Bridge".to_owned(),
        "op-bridge".to_owned(),
        3,
        PlacementSpec::default(),
    )?;

    // Retime turbine to start at 5 s (= 120 tc at 24 tcps) and bridge at 20 s (= 480 tc at 24 tcps).
    let manifest = ManifestStore::read_validated(&project_root)?;
    retime_scene_member_atomic(
        &project_root,
        manifest.raw(),
        root_id,
        turbine.placement_id.expect("turbine placement"),
        sdf::LayerOffset::new(120.0, 1.0),
    )?;
    retime_scene_member_atomic(
        &project_root,
        manifest.raw(),
        root_id,
        bridge.placement_id.expect("bridge placement"),
        sdf::LayerOffset::new(480.0, 1.0),
    )?;

    let root_stage = Stage::open(scene_path(&project_root, root_id).to_string_lossy().as_ref())?;
    assert!(root_stage.has_authored_time_code_range());
    assert!((root_stage.start_time_code() - 0.0).abs() < 1e-6);
    assert!((root_stage.end_time_code() - 960.0).abs() < 1e-6);
    assert!((root_stage.time_codes_per_second() - 24.0).abs() < 1e-6);
    let duration_seconds =
        (root_stage.end_time_code() - root_stage.start_time_code()) / root_stage.time_codes_per_second();
    assert!((duration_seconds - 40.0).abs() < 1e-6);

    // Verify actual OpenUSD composed sample times match (no double rate conversion!):
    // - Crane samples at [0, 240]
    // - Turbine samples at [120, 840] (5s..35s at 24 tcps)
    // - Bridge samples at [480, 960] (20s..40s at 24 tcps)
    let crane_attr = format!(
        "{}/Source.xformOp:translate",
        scene_member_path(crane.placement_id.unwrap())
    );
    let turbine_attr = format!(
        "{}/Source.xformOp:translate",
        scene_member_path(turbine.placement_id.unwrap())
    );
    let bridge_attr = format!(
        "{}/Source.xformOp:translate",
        scene_member_path(bridge.placement_id.unwrap())
    );
    assert_eq!(
        root_stage.time_sample_times(crane_attr.as_str())?,
        Some(vec![0.0, 240.0])
    );
    assert_eq!(
        root_stage.time_sample_times(turbine_attr.as_str())?,
        Some(vec![120.0, 840.0])
    );
    assert_eq!(
        root_stage.time_sample_times(bridge_attr.as_str())?,
        Some(vec![480.0, 960.0])
    );
    // Time-sample presence alone does not establish transform parity.
    for (attribute, sample, expected) in [(crane_attr.as_str(), 120.0, 5.0),
        (turbine_attr.as_str(), 480.0, 15.0), (bridge_attr.as_str(), 720.0, 10.0)] {
        let prim_path = attribute.strip_suffix(".xformOp:translate").expect("fixture attribute path");
        let xform = usd_bevy::read::xform::read_transform_at(&root_stage, &sdf::path(prim_path)?, Some(sample))?
            .expect("composed source root transform");
        assert!((f64::from(xform.translate[0]) - expected).abs() < 1e-4,
            "source root transform must be evaluated at the mapped time: {prim_path}");
    }
    Ok(())
}

#[test]
fn nested_propagation_import_linked_sync_and_deletion_update_all_ancestors() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let registry_path = dir.path().join("registry.json");
    let mut service = ProjectApplicationService::open(&registry_path)?;
    let project = service.create_project(dir.path(), "NestedProject")?;
    let project_root = dir.path().join("NestedProject");
    let root_id = root_scene_id(&project_root)?;

    // Root -> MidScene -> (LinkedLeafScene + ModelLeaf)
    let mid = service.create_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        "MidScene",
    )?;
    let mid_id = mid.scene_id;
    let mid_placement = mid.placement_id.expect("mid placement");

    // Retime MidScene inside Root: offset = 24 (1s), scale = 2.0
    let manifest = ManifestStore::read_validated(&project_root)?;
    retime_scene_member_atomic(
        &project_root,
        manifest.raw(),
        root_id,
        mid_placement,
        sdf::LayerOffset::new(24.0, 2.0),
    )?;

    // Link a 10s scene (24 tcps, [0, 240]) into MidScene
    let leaf_src = dir.path().join("leaf.usda");
    write_animated_xform_usda(&leaf_src, "Leaf", 0.0, 240.0, 24.0, None, 10.0)?;
    let leaf_insp = inspect_composition(&leaf_src)?;
    let leaf = service.link_scene(
        project.id,
        ProjectWriteTarget::Scene(mid_id),
        &leaf_src,
        &leaf_insp,
        "LinkedLeaf".to_owned(),
        "op-leaf".to_owned(),
        1,
        PlacementSpec::default(),
    )?;

    // MidScene should be [0, 240]; Root (offset=24, scale=2) should be [24, 504]!
    let mid_stage = Stage::open(scene_path(&project_root, mid_id).to_string_lossy().as_ref())?;
    assert_eq!(
        (mid_stage.start_time_code(), mid_stage.end_time_code()),
        (0.0, 240.0)
    );
    let root_stage = Stage::open(scene_path(&project_root, root_id).to_string_lossy().as_ref())?;
    assert_eq!(
        (root_stage.start_time_code(), root_stage.end_time_code()),
        (24.0, 504.0)
    );

    // Linked-source sync: extend leaf.usda to [0, 360] -> MidScene becomes [0, 360], Root becomes [24, 744]
    write_animated_xform_usda(&leaf_src, "Leaf", 0.0, 360.0, 24.0, None, 20.0)?;
    service.sync_linked_scene(
        project.id,
        leaf.scene_id,
        "op-sync".to_owned(),
        2,
    )?;
    let mid_stage = Stage::open(scene_path(&project_root, mid_id).to_string_lossy().as_ref())?;
    assert_eq!(
        (mid_stage.start_time_code(), mid_stage.end_time_code()),
        (0.0, 360.0)
    );
    let root_stage = Stage::open(scene_path(&project_root, root_id).to_string_lossy().as_ref())?;
    assert_eq!(
        (root_stage.start_time_code(), root_stage.end_time_code()),
        (24.0, 744.0)
    );

    // Publish an animated Model (30 tcps, [0, 150] = 5s = [0, 120] at 24 tcps) into MidScene
    let model_src = dir.path().join("prop.usda");
    write_animated_xform_usda(&model_src, "Prop", 0.0, 150.0, 30.0, Some(30.0), 5.0)?;
    let prep = ProjectModelPreparationQueue::default();
    let prep_result = prep.prepare("op-model".to_owned(), 3, model_src.clone());
    assert!(prep_result.inspection.is_ok(), "model prep succeeded");
    service.publish_model(
        &prep,
        project.id,
        ProjectWriteTarget::Scene(mid_id),
        &model_src,
        "op-model".to_owned(),
        3,
        PlacementSpec::default(),
    )?;

    // Delete LinkedLeaf -> MidScene shrinks to Model's [0, 120], Root shrinks to [24, 264]!
    service.delete_scene(ProjectDeleteSceneRequest {
        project_id: project.id,
        scene_id: leaf.scene_id,
    })?;
    let mid_stage = Stage::open(scene_path(&project_root, mid_id).to_string_lossy().as_ref())?;
    assert_eq!(
        (mid_stage.start_time_code(), mid_stage.end_time_code()),
        (0.0, 120.0)
    );
    let root_stage = Stage::open(scene_path(&project_root, root_id).to_string_lossy().as_ref())?;
    assert_eq!(
        (root_stage.start_time_code(), root_stage.end_time_code()),
        (24.0, 264.0)
    );
    Ok(())
}

#[test]
fn parent_authored_animation_is_unioned_and_user_authored_bounds_are_preserved() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let registry_path = dir.path().join("registry.json");
    let mut service = ProjectApplicationService::open(&registry_path)?;
    let project = service.create_project(dir.path(), "UserBoundsProject")?;
    let project_root = dir.path().join("UserBoundsProject");
    let root_id = root_scene_id(&project_root)?;
    let root_path = scene_path(&project_root, root_id);

    // Author parent-owned animation [0, 120] directly on Root's root layer (no explicit range yet).
    {
        let stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        let prim = stage.define_prim("/SceneRoot/ParentAnim")?.set_type_name("Xform")?;
        let attr = prim.create_attribute("xformOp:translate", sdf::ValueTypeName::DOUBLE3)?;
        attr.clone().set_at(
            Value::Vec3d(openusd::gf::Vec3d {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            openusd::usd::TimeCode::new(0.0),
        )?;
        attr.set_at(
            Value::Vec3d(openusd::gf::Vec3d {
                x: 5.0,
                y: 0.0,
                z: 0.0,
            }),
            openusd::usd::TimeCode::new(120.0),
        )?;
        stage.root_layer().export(root_path.to_string_lossy().as_ref())?;
    }

    // Adopt a shorter child [24, 72] -> union must include parent-authored [0, 120]!
    let child_src = dir.path().join("child.usda");
    write_animated_xform_usda(&child_src, "Child", 24.0, 72.0, 24.0, None, 2.0)?;
    let child_insp = inspect_composition(&child_src)?;
    service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &child_src,
        &child_insp,
        "Child".to_owned(),
        "op-child".to_owned(),
        1,
        PlacementSpec::default(),
    )?;
    let stage = Stage::open(root_path.to_string_lossy().as_ref())?;
    assert_eq!((stage.start_time_code(), stage.end_time_code()), (0.0, 120.0));

    // Now simulate an intentional user edit setting a deliberate shorter range [10, 50].
    stage.set_start_time_code(10.0)?;
    stage.set_end_time_code(50.0)?;
    stage.root_layer().export(root_path.to_string_lossy().as_ref())?;

    // Adopting a much longer child [0, 500] MUST NOT overwrite the intentional [10, 50] range!
    let long_src = dir.path().join("long.usda");
    write_animated_xform_usda(&long_src, "Long", 0.0, 500.0, 24.0, None, 50.0)?;
    let long_insp = inspect_composition(&long_src)?;
    service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &long_src,
        &long_insp,
        "Long".to_owned(),
        "op-long".to_owned(),
        2,
        PlacementSpec::default(),
    )?;
    let stage = Stage::open(root_path.to_string_lossy().as_ref())?;
    assert_eq!((stage.start_time_code(), stage.end_time_code()), (10.0, 50.0));
    Ok(())
}

#[test]
fn legacy_missing_metadata_repair_is_explicit_idempotent_and_preserves_git_history() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let registry_path = dir.path().join("registry.json");
    let mut service = ProjectApplicationService::open(&registry_path)?;
    let project = service.create_project(dir.path(), "LegacyProject")?;
    let project_root = dir.path().join("LegacyProject");
    let root_id = root_scene_id(&project_root)?;

    let model_src = dir.path().join("legacy_model.usda");
    write_animated_xform_usda(&model_src, "LegacyModel", 0.0, 120.0, 30.0, Some(30.0), 10.0)?;
    let prep = ProjectModelPreparationQueue::default();
    let prep_result = prep.prepare("op-m".to_owned(), 1, model_src.clone());
    assert!(prep_result.inspection.is_ok(), "model prep succeeded");
    let published_model = service.publish_model(
        &prep,
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &model_src,
        "op-m".to_owned(),
        1,
        PlacementSpec::default(),
    )?;

    // Strip timeline metadata from both Model wrapper and Root Scene to simulate legacy files.
    let model_path = model_wrapper_path(&project_root, published_model.model_id);
    for path in [&model_path, &scene_path(&project_root, root_id)] {
        let stage = Stage::open(path.to_string_lossy().as_ref())?;
        let _ = stage.clear_stage_metadata(sdf::FieldKey::StartTimeCode.as_str());
        let _ = stage.clear_stage_metadata(sdf::FieldKey::EndTimeCode.as_str());
        let _ = stage.clear_stage_metadata(sdf::FieldKey::TimeCodesPerSecond.as_str());
        let _ = stage.clear_stage_metadata(sdf::FieldKey::FramesPerSecond.as_str());
        stage.root_layer().export(path.to_string_lossy().as_ref())?;
    }

    // Commit legacy state to Git and record HEAD.
    service.commit(ProjectCommitRequest {
        project_id: project.id,
        target: ProjectCommitTarget::Project,
        message: "legacy baseline".to_owned(),
    })?;
    let repo = usd_git::Repository::open(&project_root)?;
    let head_before = repo.head()?.expect("legacy head").id().clone();

    // Read-only inspection detects both Model wrapper and Root Scene without mutating disk.
    let plan = service.inspect_timeline_repairs(project.id)?;
    assert_eq!(plan.entries.len(), 2);
    assert!(
        !Stage::open(scene_path(&project_root, root_id).to_string_lossy().as_ref())?
            .has_authored_time_code_range(),
        "inspection must be read-only"
    );

    // Explicit repair fixes both layers transactionally.
    let report = service.repair_project_timelines(project.id)?;
    assert_eq!(report.repaired_models, vec![published_model.model_id]);
    assert_eq!(report.repaired_scenes, vec![root_id]);

    let repaired_model = Stage::open(model_path.to_string_lossy().as_ref())?;
    assert_eq!(
        (
            repaired_model.start_time_code(),
            repaired_model.end_time_code(),
            repaired_model.time_codes_per_second(),
            repaired_model.frames_per_second()
        ),
        (0.0, 120.0, 30.0, 30.0)
    );
    // 120 tc at 30 tcps = 4s = 96 tc at 24 tcps on Root Scene.
    let repaired_root =
        Stage::open(scene_path(&project_root, root_id).to_string_lossy().as_ref())?;
    assert_eq!(
        (repaired_root.start_time_code(), repaired_root.end_time_code()),
        (0.0, 96.0)
    );

    // Idempotent: second inspection finds zero repairs needed.
    assert!(service.inspect_timeline_repairs(project.id)?.is_empty());

    // Git history was never rewritten: HEAD is unchanged and working tree holds the repair diff.
    let head_after = repo.head()?.expect("head after repair").id().clone();
    assert_eq!(head_before, head_after);
    assert!(repo.working_tree_status()?.dirty);
    Ok(())
}

#[test]
fn live_evaluator_parity_for_rigid_skeletal_visibility_and_cached_residency() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let source_path = dir.path().join("rig.usda");
    write_rigid_skeletal_visibility_usda(&source_path)?;

    let registry_path = dir.path().join("registry.json");
    let mut service = ProjectApplicationService::open(&registry_path)?;
    let project = service.create_project(dir.path(), "ParityProject")?;
    let project_root = dir.path().join("ParityProject");
    let root_id = root_scene_id(&project_root)?;

    // Adopt as Scene under Project root.
    let insp = inspect_composition(&source_path)?;
    let adopted = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &source_path,
        &insp,
        "RigScene".to_owned(),
        "op-rig".to_owned(),
        1,
        PlacementSpec::default(),
    )?;

    // Also publish as Model under a sibling Scene.
    let sibling = service.create_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        "ModelHost",
    )?;
    let prep = ProjectModelPreparationQueue::default();
    let prep_result = prep.prepare("op-rig-model".to_owned(), 2, source_path.clone());
    assert!(prep_result.inspection.is_ok(), "model prep succeeded");
    let published_model = service.publish_model(
        &prep,
        project.id,
        ProjectWriteTarget::Scene(sibling.scene_id),
        &source_path,
        "op-rig-model".to_owned(),
        2,
        PlacementSpec::default(),
    )?;

    let paths = [
        ("standalone", source_path.clone(), false),
        (
            "model_wrapper",
            model_wrapper_path(&project_root, published_model.model_id),
            false,
        ),
        (
            "adopted_scene",
            scene_path(&project_root, adopted.scene_id),
            false,
        ),
        ("parent_scene", scene_path(&project_root, sibling.scene_id), false),
        ("project_root_no_cache", scene_path(&project_root, root_id), false),
        ("project_root_with_cache", scene_path(&project_root, root_id), true),
    ];

    for (label, path, cached) in paths {
        // Sample at t = 1.0 s (time code 24, midpoint of 0..48 @ 24 tcps):
        // - Rig xform translation = (6, 0, 0)
        // - Skeletal Tip joint translation = (0, 3, 0)
        // - Beacon visibility = Visibility::Hidden (authored "invisible" at tc=24)
        let (clock, transforms, joints, visibilities) =
            evaluate_stage_in_live_app(&path, 1.0, cached)?;
        assert!(
            (clock.duration_seconds() - 2.0).abs() < 1e-6,
            "{label}: expected 2.0s duration, got {}",
            clock.duration_seconds()
        );
        let rig_tx = transforms
            .iter()
            .find(|(p, _)| p.ends_with("/Mover"))
            .map(|(_, t)| *t)
            .expect("animated mover transform present");
        assert!(
            (rig_tx.x - 6.0).abs() < 0.05,
            "{label}: expected midpoint x ~= 6.0, got {rig_tx:?}"
        );
        let tip_joint = joints
            .iter()
            .find(|(p, _)| p.ends_with("Tip"))
            .map(|(_, t)| *t)
            .expect("animated skeletal tip joint present");
        assert!(
            (tip_joint.y - 3.0).abs() < 0.05,
            "{label}: expected skeletal Tip joint midpoint y ~= 3.0, got {tip_joint:?}"
        );
        let beacon_vis = visibilities
            .iter()
            .find(|(p, _)| p.ends_with("/Beacon"))
            .map(|(_, v)| *v)
            .expect("animated beacon visibility present");
        assert_eq!(
            beacon_vis,
            Visibility::Hidden,
            "{label}: expected Beacon hidden at t=1.0s"
        );
    }
    Ok(())
}

#[test]
fn static_and_unavailable_domains_have_distinct_clock_states() -> Result<()> {
    use crate::animation::PlaybackDomainReadiness;
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("static.usda");
    fs::write(&source, "#usda 1.0\n( defaultPrim = \"Static\" )\ndef Xform \"Static\" {}\n")?;
    let (clock, _, _, _) = evaluate_stage_in_live_app(&source, 0.0, false)?;
    assert_eq!(clock.domain_readiness, PlaybackDomainReadiness::NoPlayableAnimation);
    let source = dir.path().join("partial.usda");
    fs::write(&source, "#usda 1.0\n( defaultPrim = \"Partial\" startTimeCode = 0 )\ndef Xform \"Partial\" {}\n")?;
    let (clock, _, _, _) = evaluate_stage_in_live_app(&source, 0.0, false)?;
    assert_eq!(clock.domain_readiness, PlaybackDomainReadiness::Unavailable);
    assert!(!clock.playing);
    Ok(())
}

#[test]
fn positive_elapsed_duration_must_be_representable() -> Result<()> {
    let tiny = TimeCodeRange::new(0.0, f64::from_bits(1))?;
    assert!(tiny.duration_seconds(24.0).is_err());
    assert_eq!(TimeCodeRange::new(0.0, 5.0)?.duration_seconds(0.5)?, 10.0);
    assert_eq!(TimeCodeRange::new(1.0, 1.0)?.duration_seconds(24.0)?, 0.0);
    Ok(())
}

#[test]
fn adopted_scene_aggregates_source_members_and_local_motion() -> Result<()> {
    use crate::project::scene::timeline::save_scene_with_timelines_atomic;
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "AdoptedUnion")?;
    let project_root = dir.path().join("AdoptedUnion");
    let root = root_scene_id(&project_root)?;
    let source = dir.path().join("source.usda");
    write_animated_xform_usda(&source, "Crane", 0.0, 240.0, 24.0, None, 10.0)?;
    let inspection = inspect_composition(&source)?;
    let adopted = service.adopt_scene(project.id, ProjectWriteTarget::Scene(root), &source,
        &inspection, "Crane".to_owned(), "crane".to_owned(), 1, PlacementSpec::default())?;
    let path = scene_path(&project_root, adopted.scene_id);
    let edited = Stage::open(path.to_string_lossy().as_ref())?;
    author_scene_motion(&edited, 480.0)?;
    let manifest = ManifestStore::read_validated(&project_root)?;
    save_scene_with_timelines_atomic(&edited, &project_root, manifest.raw(), adopted.scene_id, &path)?;
    assert_eq!(Stage::open(path.to_string_lossy().as_ref())?.end_time_code(), 480.0);
    let model_source = dir.path().join("long.usda");
    write_animated_xform_usda(&model_source, "Long", 0.0, 900.0, 30.0, None, 20.0)?;
    let preparation = ProjectModelPreparationQueue::default();
    assert!(preparation.prepare("long".to_owned(), 2, model_source.clone()).inspection.is_ok());
    let model = service.publish_model(&preparation, project.id, ProjectWriteTarget::Scene(adopted.scene_id),
        &model_source, "long".to_owned(), 2, PlacementSpec::default())?;
    assert_eq!(Stage::open(path.to_string_lossy().as_ref())?.end_time_code(), 720.0);
    assert_eq!(Stage::open(scene_path(&project_root, root).to_string_lossy().as_ref())?.end_time_code(), 720.0);
    // The local and source intervals remain after removing the longer member.
    service.delete_model(project_protocol::ProjectDeleteModelRequest { project_id: project.id, model_id: model.model_id })?;
    assert_eq!(Stage::open(path.to_string_lossy().as_ref())?.end_time_code(), 480.0);
    assert_eq!(Stage::open(scene_path(&project_root, root).to_string_lossy().as_ref())?.end_time_code(), 480.0);
    Ok(())
}

#[test]
fn positive_time_mapping_must_not_collapse_at_parent_precision() -> Result<()> {
    let range = TimeCodeRange::new(0.0, 1.0)?;
    assert!(map_child_domain_to_parent(range, 24.0, 24.0, sdf::LayerOffset::new(1.0e30, 1.0)).is_err());
    assert!(map_child_domain_to_parent(range, 1.0e300, 1.0e-300, sdf::LayerOffset::IDENTITY).is_err());
    Ok(())
}

#[test]
fn standalone_and_wrapped_fractional_rate_and_inferred_range() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("slow.usda");
    write_animated_xform_usda(&source, "Slow", 0.0, 5.0, 0.5, None, 10.0)?;
    let (clock, _, _, _) = evaluate_stage_in_live_app(&source, 5.0, false)?;
    assert_eq!(clock.time_codes_per_second, 0.5);
    assert!((clock.duration_seconds() - 10.0).abs() < 1e-9);
    let stage = Stage::open(source.to_string_lossy().as_ref())?;
    stage.clear_stage_metadata(sdf::FieldKey::StartTimeCode.as_str())?;
    stage.clear_stage_metadata(sdf::FieldKey::EndTimeCode.as_str())?;
    stage.root_layer().export(source.to_string_lossy().as_ref())?;
    let (clock, _, _, _) = evaluate_stage_in_live_app(&source, 5.0, false)?;
    assert!((clock.duration_seconds() - 10.0).abs() < 1e-9);
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "FractionalPlayback")?;
    let project_root = dir.path().join("FractionalPlayback");
    let root = root_scene_id(&project_root)?;
    let inspection = inspect_composition(&source)?;
    let adopted = service.adopt_scene(project.id, ProjectWriteTarget::Scene(root), &source,
        &inspection, "Slow".to_owned(), "slow".to_owned(), 1, PlacementSpec::default())?;
    for id in [adopted.scene_id, root] {
        let (clock, _, _, _) = evaluate_stage_in_live_app(&scene_path(&project_root, id), 5.0, false)?;
        assert!((clock.duration_seconds() - 10.0).abs() < 1e-9, "fractional duration through composition");
    }
    Ok(())
}

#[test]
fn legacy_wrapper_repair_preserves_explicit_shorter_bounds() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "IntentionalBounds")?;
    let project_root = dir.path().join("IntentionalBounds");
    let root = root_scene_id(&project_root)?;
    let source = dir.path().join("source.usda");
    write_animated_xform_usda(&source, "SourceRoot", 0.0, 500.0, 24.0, Some(30.0), 10.0)?;
    let inspection = inspect_composition(&source)?;
    let adopted = service.adopt_scene(project.id, ProjectWriteTarget::Scene(root), &source,
        &inspection, "Source".to_owned(), "source".to_owned(), 1, PlacementSpec::default())?;
    let path = scene_path(&project_root, adopted.scene_id);
    let wrapper = Stage::open(path.to_string_lossy().as_ref())?;
    wrapper.set_start_time_code(10.0)?;
    wrapper.set_end_time_code(50.0)?;
    wrapper.clear_stage_metadata(sdf::FieldKey::FramesPerSecond.as_str())?;
    wrapper.root_layer().export(path.to_string_lossy().as_ref())?;
    let before = fs::read(&path)?;
    let plan = service.inspect_timeline_repairs(project.id)?;
    let entry = plan.entries.iter().find(|entry| entry.layer_path == path).expect("missing FPS repair");
    assert!(!entry.repair_range && !entry.repair_time_codes_per_second && entry.repair_frames_per_second);
    assert_eq!(fs::read(&path)?, before, "inspection is read-only");
    service.repair_project_timelines(project.id)?;
    let repaired = Stage::open(path.to_string_lossy().as_ref())?;
    assert_eq!((repaired.start_time_code(), repaired.end_time_code(), repaired.frames_per_second()), (10.0, 50.0, 30.0));
    assert!(service.inspect_timeline_repairs(project.id)?.is_empty());
    Ok(())
}

fn author_scene_motion(stage: &Stage, end: f64) -> Result<()> {
    let prim = stage.define_prim("/SceneRoot/OwnMotion")?.set_type_name("Xform")?;
    let attr = prim.create_attribute("xformOp:translate", sdf::ValueTypeName::DOUBLE3)?;
    attr.clone().set_at(Value::Vec3d(openusd::gf::Vec3d { x: 0.0, y: 0.0, z: 0.0 }), openusd::usd::TimeCode::new(0.0))?;
    attr.set_at(Value::Vec3d(openusd::gf::Vec3d { x: 1.0, y: 0.0, z: 0.0 }), openusd::usd::TimeCode::new(end))?;
    prim.create_attribute("xformOpOrder", "token[]")?.set(Value::TokenVec(vec!["xformOp:translate".into()]))?;
    Ok(())
}

#[test]
fn memberless_parent_motion_is_repaired_and_saved_seed_domain_recomputed() -> Result<()> {
    use crate::project::scene::timeline::save_scene_with_timelines_atomic;
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "OwnMotion")?;
    let project_root = dir.path().join("OwnMotion");
    let root = root_scene_id(&project_root)?;
    let path = scene_path(&project_root, root);
    let stage = Stage::open(path.to_string_lossy().as_ref())?;
    author_scene_motion(&stage, 120.0)?;
    stage.root_layer().export(path.to_string_lossy().as_ref())?;
    assert_eq!(service.inspect_timeline_repairs(project.id)?.entries.len(), 1);
    service.repair_project_timelines(project.id)?;
    let edited = Stage::open(path.to_string_lossy().as_ref())?;
    assert_eq!(edited.end_time_code(), 120.0);
    author_scene_motion(&edited, 480.0)?;
    let manifest = ManifestStore::read_validated(&project_root)?;
    let updated = save_scene_with_timelines_atomic(&edited, &project_root, manifest.raw(), root, &path)?;
    assert_eq!(updated, vec![root]);
    assert_eq!(Stage::open(path.to_string_lossy().as_ref())?.end_time_code(), 480.0);
    assert_eq!(edited.end_time_code(), 480.0);
    Ok(())
}

#[test]
fn mismatched_managed_reference_is_rejected_before_aggregation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "WrongArc")?;
    let project_root = dir.path().join("WrongArc");
    let root = root_scene_id(&project_root)?;
    let source = dir.path().join("clip.usda");
    write_animated_xform_usda(&source, "Clip", 0.0, 240.0, 24.0, None, 10.0)?;
    let inspection = inspect_composition(&source)?;
    let child = service.adopt_scene(project.id, ProjectWriteTarget::Scene(root), &source,
        &inspection, "Clip".to_owned(), "clip".to_owned(), 1, PlacementSpec::default())?;
    let path = scene_path(&project_root, root);
    let before = fs::read(&path)?;
    let stage = Stage::open(path.to_string_lossy().as_ref())?;
    let members = crate::project::scene::authoring::read_scene_members(&path, root)?;
    let member = members.iter().find(|m| m.target == usd_project::SceneMemberTarget::Scene(child.scene_id)).expect("member");
    let member_path = scene_member_path(member.id);
    let spec = stage.root_layer().prim(sdf::path(&member_path)?)?.expect("member spec");
    let Some(Value::ReferenceListOp(references)) = spec.field("references")? else { anyhow::bail!("member reference missing"); };
    let mut wrong = references.iter().next().expect("reference").clone();
    wrong.prim_path = sdf::path("/WrongRoot")?;
    stage.prim(member_path.as_str())?.set_metadata("references", Value::ReferenceListOp(sdf::ReferenceListOp::prepended([wrong])))?;
    let result = crate::project::scene::timeline::recompute_scene_stage_timeline(&stage, &project_root, &path,
        &members, &crate::project::scene::timeline::TimelineStageOverrides::default());
    assert!(result.is_err());
    assert_eq!(fs::read(path)?, before);
    Ok(())
}

#[test]
fn required_missing_child_does_not_publish_truncated_domain() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "MissingChild")?;
    let project_root = dir.path().join("MissingChild");
    let root = root_scene_id(&project_root)?;
    let source = dir.path().join("long.usda");
    write_animated_xform_usda(&source, "Long", 0.0, 960.0, 24.0, None, 10.0)?;
    let inspection = inspect_composition(&source)?;
    let child = service.adopt_scene(project.id, ProjectWriteTarget::Scene(root), &source,
        &inspection, "Long".to_owned(), "long".to_owned(), 1, PlacementSpec::default())?;
    let root_path = scene_path(&project_root, root);
    let before = fs::read(&root_path)?;
    fs::remove_file(scene_path(&project_root, child.scene_id))?;
    assert!(service.inspect_timeline_repairs(project.id).is_err());
    assert_eq!(fs::read(root_path)?, before);
    Ok(())
}

#[test]
fn default_prim_root_animation_evaluates_normalized_world_transforms_across_composition()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("animated_root.usda");
    write_animated_xform_usda(&source, "Crane", 0.0, 240.0, 24.0, Some(24.0), 10.0)?;

    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "RootMotionParity")?;
    let project_root = dir.path().join("RootMotionParity");
    let root_id = root_scene_id(&project_root)?;

    let insp = inspect_composition(&source)?;
    let adopted = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &source,
        &insp,
        "CraneScene".to_owned(),
        "op-crane-scene".to_owned(),
        1,
        PlacementSpec::default(),
    )?;

    let prep = ProjectModelPreparationQueue::default();
    let prep_result = prep.prepare("op-crane-model".to_owned(), 2, source.clone());
    assert!(prep_result.inspection.is_ok(), "model prep succeeded");
    let model = service.publish_model(
        &prep,
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &source,
        "op-crane-model".to_owned(),
        2,
        PlacementSpec::default(),
    )?;

    let cases = [
        ("standalone", source.clone(), "/Crane"),
        (
            "model_wrapper",
            model_wrapper_path(&project_root, model.model_id),
            "/ModelRoot/Source",
        ),
        (
            "adopted_scene",
            scene_path(&project_root, adopted.scene_id),
            "/SceneRoot/Source",
        ),
        (
            "project_root",
            scene_path(&project_root, root_id),
            "/Source",
        ),
    ];

    for (label, path, suffix) in cases {
        for (sample_s, expected_x) in [(0.0, 0.0_f32), (5.0, 5.0_f32), (10.0, 10.0_f32)] {
            let (clock, transforms, _, _) = evaluate_stage_in_live_app(&path, sample_s, false)?;
            assert!(
                (clock.duration_seconds() - 10.0).abs() < 1e-6,
                "{label}: expected 10s duration"
            );
            let actual = transforms
                .iter()
                .find(|(p, _)| p.ends_with(suffix))
                .map(|(_, t)| *t)
                .unwrap_or_else(|| panic!("{label}: missing prim ending with {suffix}"));
            assert!(
                (actual.x - expected_x).abs() < 0.05,
                "{label} @ {sample_s}s: expected x ~= {expected_x}, got {actual:?}"
            );
        }
    }

    // Verify non-Y-up / centimeter (Z-up, metersPerUnit = 0.01) root animation
    // preserves both source motion (including Z-axis -> world Y-axis rotation)
    // and non-default member placement in actual GlobalTransform world space.
    let zup_cm = dir.path().join("zup_cm_root.usda");
    fs::write(
        &zup_cm,
        r#"#usda 1.0
(
    defaultPrim = "Lift"
    startTimeCode = 0
    endTimeCode = 240
    timeCodesPerSecond = 24
    metersPerUnit = 0.01
    upAxis = "Z"
)
def Xform "Lift"
{
    double3 xformOp:translate.timeSamples = {
        0: (0, 0, 0),
        240: (1000, 0, 600),
    }
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Cube "Body" { double size = 100.0 }
}
"#,
    )?;
    let zup_insp = inspect_composition(&zup_cm)?;
    let offset_placement = PlacementSpec::Matrix("1 0 0 0\n0 1 0 0\n0 0 1 0\n100 0 0 1".to_owned());
    let zup_scene = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &zup_cm,
        &zup_insp,
        "ZupLift".to_owned(),
        "op-zup".to_owned(),
        3,
        offset_placement,
    )?;
    let (clock, world_transforms) = evaluate_stage_world_transforms_in_live_app(
        &scene_path(&project_root, zup_scene.scene_id),
        5.0,
    )?;
    assert!((clock.duration_seconds() - 10.0).abs() < 1e-6);
    let lift_world = world_transforms
        .iter()
        .find(|(p, _)| p.ends_with("/SceneRoot/Source"))
        .map(|(_, t)| *t)
        .expect("normalized Z-up root present");
    // At midpoint (5s): (500 cm X, 0, 300 cm Z) * 0.01 m/cm rotated Z-up -> Y-up
    // yields world (5.0 m X, 3.0 m Y, 0.0 m Z).
    assert!(
        (lift_world.x - 5.0).abs() < 0.05 && (lift_world.y - 3.0).abs() < 0.05,
        "Z-up cm normalized world midpoint expected (~5.0, ~3.0, 0.0), got {lift_world:?}"
    );
    // In the Project root under non-default placement (+100 m X), GlobalTransform
    // must compose parent placement with normalized animated root motion.
    let (_root_clock, root_worlds) =
        evaluate_stage_world_transforms_in_live_app(&scene_path(&project_root, root_id), 5.0)?;
    let placed_zup_world = root_worlds
        .iter()
        .find(|(p, t)| p.ends_with("/Source") && (t.x - 105.0).abs() < 0.25)
        .map(|(_, t)| *t)
        .unwrap_or_else(|| {
            panic!("expected placed Z-up /Source at world x~=105.0, got {root_worlds:?}")
        });
    assert!(
        (placed_zup_world.x - 105.0).abs() < 0.05 && (placed_zup_world.y - 3.0).abs() < 0.05,
        "placed Z-up root world midpoint expected (~105.0, ~3.0, 0.0), got {placed_zup_world:?}"
    );
    Ok(())
}

#[test]
fn git_branch_switch_restores_historical_unrepaired_timeline_metadata() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "HistoryRestore")?;
    let project_root = dir.path().join("HistoryRestore");
    let root_id = root_scene_id(&project_root)?;

    let src = dir.path().join("clip.usda");
    write_animated_xform_usda(&src, "Clip", 0.0, 120.0, 24.0, Some(24.0), 10.0)?;
    let insp = inspect_composition(&src)?;
    let _ = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root_id),
        &src,
        &insp,
        "Clip".to_owned(),
        "op-clip".to_owned(),
        1,
        PlacementSpec::default(),
    )?;

    // Strip Root timeline range to simulate a legacy committed revision.
    let root_path = scene_path(&project_root, root_id);
    {
        let stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        let _ = stage.clear_stage_metadata(sdf::FieldKey::StartTimeCode.as_str());
        let _ = stage.clear_stage_metadata(sdf::FieldKey::EndTimeCode.as_str());
        stage.root_layer().export(root_path.to_string_lossy().as_ref())?;
    }
    service.commit(ProjectCommitRequest {
        project_id: project.id,
        target: ProjectCommitTarget::Project,
        message: "historical legacy state".to_owned(),
    })?;

    // Record legacy HEAD, apply explicit repair, and commit as a new revision.
    let repo = usd_git::Repository::open(&project_root)?;
    let legacy_head = repo.head()?.expect("legacy head").id().clone();
    service.repair_project_timelines(project.id)?;
    assert_eq!(
        Stage::open(root_path.to_string_lossy().as_ref())?.end_time_code(),
        120.0
    );
    service.commit(ProjectCommitRequest {
        project_id: project.id,
        target: ProjectCommitTarget::Project,
        message: "explicit timeline repair".to_owned(),
    })?;

    // Materialize the historical revision: it must restore the exact un-repaired source.
    let restored_dir = dir.path().join("restored_historical");
    repo.materialize_revision(&legacy_head, &restored_dir)?;
    let restored_root_path = scene_path(&restored_dir, root_id);
    let restored_stage = Stage::open(restored_root_path.to_string_lossy().as_ref())?;
    assert!(
        !restored_stage.has_authored_time_code_range(),
        "historical revision materialization must preserve original un-repaired metadata"
    );
    Ok(())
}

#[test]
fn intentional_parent_bounds_still_qualify_required_members_and_wrapper_sources() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "IntentionalQualify")?;
    let project_root = dir.path().join("IntentionalQualify");
    let root = root_scene_id(&project_root)?;
    let source = dir.path().join("child.usda");
    write_animated_xform_usda(&source, "Child", 0.0, 960.0, 24.0, Some(24.0), 10.0)?;
    let inspection = inspect_composition(&source)?;
    let child = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root),
        &source,
        &inspection,
        "Child".to_owned(),
        "op-child".to_owned(),
        1,
        PlacementSpec::default(),
    )?;

    // Author intentional shorter bounds [10, 50] on the parent root Scene.
    let root_path = scene_path(&project_root, root);
    {
        let stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        stage.set_start_time_code(10.0)?;
        stage.set_end_time_code(50.0)?;
        stage.root_layer().export(root_path.to_string_lossy().as_ref())?;
    }
    let root_bytes = fs::read(&root_path)?;
    let members = crate::project::scene::authoring::read_scene_members(&root_path, root)?;

    // 1. Valid intentional bounds are preserved unchanged while qualifying dependencies.
    {
        let stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        let outcome = crate::project::scene::timeline::recompute_scene_stage_timeline(
            &stage,
            &project_root,
            &root_path,
            &members,
            &crate::project::scene::timeline::TimelineStageOverrides::default(),
        )?;
        assert_eq!(
            outcome,
            crate::project::scene::timeline::TimelineUpdateOutcome::PreservedUserRange
        );
        assert_eq!((stage.start_time_code(), stage.end_time_code()), (10.0, 50.0));
        assert!(service.inspect_timeline_repairs(project.id)?.is_empty());
    }

    // 2. Mismatched member reference prim on intentional parent is rejected in BOTH
    //    recompute_scene_stage_timeline and read-only inspect_timeline_repairs.
    let member = members
        .iter()
        .find(|m| m.target == usd_project::SceneMemberTarget::Scene(child.scene_id))
        .expect("child member");
    let member_path = scene_member_path(member.id);
    {
        let stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        let spec = stage.root_layer().prim(sdf::path(&member_path)?)?.expect("member spec");
        let Some(Value::ReferenceListOp(refs)) = spec.field("references")? else {
            anyhow::bail!("missing member reference");
        };
        let mut wrong = refs.iter().next().expect("ref").clone();
        wrong.prim_path = sdf::path("/WrongRoot")?;
        stage.prim(member_path.as_str())?.set_metadata(
            "references",
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended([wrong])),
        )?;
        assert!(
            crate::project::scene::timeline::recompute_scene_stage_timeline(
                &stage,
                &project_root,
                &root_path,
                &members,
                &crate::project::scene::timeline::TimelineStageOverrides::default(),
            )
            .is_err(),
            "recompute must reject mismatched reference even with intentional bounds"
        );
        stage.root_layer().export(root_path.to_string_lossy().as_ref())?;
        assert!(
            service.inspect_timeline_repairs(project.id).is_err(),
            "read-only repair inspection must reject mismatched reference even with intentional bounds"
        );
    }
    fs::write(&root_path, &root_bytes)?;

    // 3. Invalid partial child metadata is rejected under intentional parent bounds.
    let child_path = scene_path(&project_root, child.scene_id);
    let child_bytes = fs::read(&child_path)?;
    {
        let child_stage = Stage::open(child_path.to_string_lossy().as_ref())?;
        child_stage.clear_stage_metadata(sdf::FieldKey::EndTimeCode.as_str())?;
        child_stage.root_layer().export(child_path.to_string_lossy().as_ref())?;
        let parent_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        assert!(
            crate::project::scene::timeline::recompute_scene_stage_timeline(
                &parent_stage,
                &project_root,
                &root_path,
                &members,
                &crate::project::scene::timeline::TimelineStageOverrides::default(),
            )
            .is_err(),
            "intentional parent must reject invalid partial child metadata"
        );
    }
    fs::write(&child_path, &child_bytes)?;

    // 4. Missing required managed child file is rejected under intentional parent bounds.
    {
        let parent_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        fs::remove_file(&child_path)?;
        assert!(
            crate::project::scene::timeline::recompute_scene_stage_timeline(
                &parent_stage,
                &project_root,
                &root_path,
                &members,
                &crate::project::scene::timeline::TimelineStageOverrides::default(),
            )
            .is_err(),
            "intentional parent must reject missing managed child layer"
        );
        assert_eq!(fs::read(&root_path)?, root_bytes);
    }
    fs::write(&child_path, &child_bytes)?;

    // 5. Missing adopted wrapper source under intentional bounds on adopted Scene is rejected.
    {
        let adopted_stage = Stage::open(child_path.to_string_lossy().as_ref())?;
        adopted_stage.set_start_time_code(5.0)?;
        adopted_stage.set_end_time_code(25.0)?;
        adopted_stage.root_layer().export(child_path.to_string_lossy().as_ref())?;
        let source_spec = adopted_stage
            .root_layer()
            .prim(sdf::path("/SceneRoot/Source")?)?
            .expect("source spec");
        let Some(Value::ReferenceListOp(refs)) = source_spec.field("references")? else {
            anyhow::bail!("missing source reference");
        };
        let localized_source = child_path
            .parent()
            .expect("parent")
            .join(&refs.iter().next().expect("ref").asset_path);
        fs::remove_file(&localized_source)?;
        let reopened = Stage::open(child_path.to_string_lossy().as_ref())?;
        assert!(
            crate::project::scene::timeline::recompute_scene_stage_timeline(
                &reopened,
                &project_root,
                &child_path,
                &[],
                &crate::project::scene::timeline::TimelineStageOverrides::default(),
            )
            .is_err(),
            "intentional adopted Scene must reject missing wrapper source"
        );
    }
    Ok(())
}

#[test]
fn linked_scene_refresh_rejects_single_timestamp_local_samples_and_allows_clean_sync() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "LinkedSingleSample")?;
    let project_root = dir.path().join("LinkedSingleSample");
    let root = root_scene_id(&project_root)?;
    let source = dir.path().join("linked_src.usda");
    write_animated_xform_usda(&source, "Crane", 0.0, 240.0, 24.0, Some(24.0), 10.0)?;
    let inspection = inspect_composition(&source)?;
    let adopted = service.link_scene(
        project.id,
        ProjectWriteTarget::Scene(root),
        &source,
        &inspection,
        "Crane".to_owned(),
        "op-link".to_owned(),
        1,
        PlacementSpec::default(),
    )?;

    // 1. Clean source-only linked Scene syncs and propagates updated 15s (360) domain.
    write_animated_xform_usda(&source, "Crane", 0.0, 360.0, 24.0, Some(24.0), 15.0)?;
    service.sync_linked_scene(
        project.id,
        adopted.scene_id,
        "op-sync-clean".to_owned(),
        2,
    )?;
    let child_path = scene_path(&project_root, adopted.scene_id);
    let root_path = scene_path(&project_root, root);
    assert_eq!(Stage::open(child_path.to_string_lossy().as_ref())?.end_time_code(), 360.0);
    assert_eq!(Stage::open(root_path.to_string_lossy().as_ref())?.end_time_code(), 360.0);

    // 2. Author a SINGLE local keyframe at time 120 (zero positive span, non-empty samples).
    {
        let stage = Stage::open(child_path.to_string_lossy().as_ref())?;
        let prim = stage.define_prim("/SceneRoot/LocalKey")?.set_type_name("Xform")?;
        let attr = prim.create_attribute("xformOp:translate", sdf::ValueTypeName::DOUBLE3)?;
        attr.set_at(
            Value::Vec3d(openusd::gf::Vec3d { x: 1.0, y: 2.0, z: 3.0 }),
            openusd::usd::TimeCode::new(120.0),
        )?;
        stage.root_layer().export(child_path.to_string_lossy().as_ref())?;
    }
    let child_before = fs::read(&child_path)?;
    let root_before = fs::read(&root_path)?;

    write_animated_xform_usda(&source, "Crane", 0.0, 480.0, 24.0, Some(24.0), 20.0)?;
    assert!(
        service
            .sync_linked_scene(
                project.id,
                adopted.scene_id,
                "op-sync-single-key".to_owned(),
                3,
            )
            .is_err(),
        "linked refresh must reject discarding a single local authored keyframe"
    );
    assert_eq!(fs::read(&child_path)?, child_before);
    assert_eq!(fs::read(&root_path)?, root_before);

    // 3. Author a second local attribute ALSO keyed at time 120 (still zero positive span).
    {
        let stage = Stage::open(child_path.to_string_lossy().as_ref())?;
        let prim = stage.prim("/SceneRoot/LocalKey")?;
        let vis = prim.create_attribute("visibility", sdf::ValueTypeName::TOKEN)?;
        vis.set_at(Value::Token("invisible".into()), openusd::usd::TimeCode::new(120.0))?;
        stage.root_layer().export(child_path.to_string_lossy().as_ref())?;
    }
    let child_before_two = fs::read(&child_path)?;
    assert!(
        service
            .sync_linked_scene(
                project.id,
                adopted.scene_id,
                "op-sync-two-attrs-same-time".to_owned(),
                4,
            )
            .is_err(),
        "linked refresh must reject discarding multiple local attributes keyed at the same time"
    );
    assert_eq!(fs::read(&child_path)?, child_before_two);
    assert_eq!(fs::read(&root_path)?, root_before);
    Ok(())
}

#[test]
fn ancestor_recompute_and_repair_reject_indirect_missing_wrapper_source_and_recover() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let mut service = ProjectApplicationService::open(&dir.path().join("registry.json"))?;
    let project = service.create_project(dir.path(), "IndirectMissingSource")?;
    let project_root = dir.path().join("IndirectMissingSource");
    let root = root_scene_id(&project_root)?;
    let source = dir.path().join("crane.usda");
    write_animated_xform_usda(&source, "Crane", 0.0, 240.0, 24.0, Some(24.0), 10.0)?;
    let inspection = inspect_composition(&source)?;
    let child = service.adopt_scene(
        project.id,
        ProjectWriteTarget::Scene(root),
        &source,
        &inspection,
        "Crane".to_owned(),
        "op-crane".to_owned(),
        1,
        PlacementSpec::default(),
    )?;

    let root_path = scene_path(&project_root, root);
    let child_path = scene_path(&project_root, child.scene_id);
    let members = crate::project::scene::authoring::read_scene_members(&root_path, root)?;

    // Locate and save the child wrapper's localized source bytes, then delete it
    // while keeping the child wrapper file and its authored [0, 240] bounds intact.
    let child_stage = Stage::open(child_path.to_string_lossy().as_ref())?;
    assert!(child_stage.has_authored_time_code_range());
    let source_spec = child_stage
        .root_layer()
        .prim(sdf::path("/SceneRoot/Source")?)?
        .expect("source spec");
    let Some(Value::ReferenceListOp(refs)) = source_spec.field("references")? else {
        anyhow::bail!("missing child wrapper source reference");
    };
    let localized_source = child_path
        .parent()
        .expect("parent")
        .join(&refs.iter().next().expect("ref").asset_path);
    let source_bytes = fs::read(&localized_source)?;
    let root_bytes = fs::read(&root_path)?;
    fs::remove_file(&localized_source)?;

    // 1. Ancestor recompute with generated bounds rejects missing indirect source.
    {
        let parent_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        assert!(
            crate::project::scene::timeline::recompute_scene_stage_timeline(
                &parent_stage,
                &project_root,
                &root_path,
                &members,
                &crate::project::scene::timeline::TimelineStageOverrides::default(),
            )
            .is_err(),
            "ancestor recompute (generated bounds) must reject missing child wrapper source"
        );
        assert_eq!(fs::read(&root_path)?, root_bytes);
    }

    // 2. Ancestor recompute with intentional shorter bounds also rejects missing indirect source.
    {
        let parent_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        parent_stage.set_start_time_code(10.0)?;
        parent_stage.set_end_time_code(50.0)?;
        assert!(
            crate::project::scene::timeline::recompute_scene_stage_timeline(
                &parent_stage,
                &project_root,
                &root_path,
                &members,
                &crate::project::scene::timeline::TimelineStageOverrides::default(),
            )
            .is_err(),
            "ancestor recompute (intentional bounds) must reject missing child wrapper source"
        );
        assert_eq!(fs::read(&root_path)?, root_bytes);
    }

    // 3. Read-only repair inspection rejects missing indirect source without mutating files.
    assert!(service.inspect_timeline_repairs(project.id).is_err());
    assert_eq!(fs::read(&root_path)?, root_bytes);

    // 4. Restoring the localized source recovers cleanly.
    fs::write(&localized_source, &source_bytes)?;
    {
        let parent_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
        crate::project::scene::timeline::recompute_scene_stage_timeline(
            &parent_stage,
            &project_root,
            &root_path,
            &members,
            &crate::project::scene::timeline::TimelineStageOverrides::default(),
        )?;
        assert_eq!(parent_stage.end_time_code(), 240.0);
    }
    assert!(service.inspect_timeline_repairs(project.id)?.is_empty());
    Ok(())
}
