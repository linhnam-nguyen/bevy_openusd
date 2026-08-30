use crate::live::reconcile::ReconcileStats;
use crate::live::{
    LiveRevision, NativeInstanceDependencyIndex, ProjectionPlan, StageChange, StageChangeBatch,
    apply_change_batch,
};
use crate::{LiveStage, LiveStagePlugin, PrimEntities, UsdPlugin, UsdPurpose};
use anyhow::Result;
use bevy::asset::Assets;
use bevy::material::AlphaMode;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::{App, Transform, Vec3, Visibility};
use openusd::sdf;
use openusd::usd::{EditTargetArc, PrimPredicate, Stage};
use openusd::{gf::Vec3f, sdf::Value};

fn characterization_stage() -> Stage {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stages/native_instance_characterization.usda");
    Stage::open(path.to_str().expect("fixture path is valid"))
        .expect("native instance fixture opens")
}

fn projected_app() -> App {
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut()
        .insert_non_send(LiveStage::new(characterization_stage()));
    app.update();
    app
}

fn projected_entity(app: &App, path: &str) -> bevy::prelude::Entity {
    app.world()
        .resource::<PrimEntities>()
        .entity(path)
        .unwrap_or_else(|| panic!("{path} entity exists"))
}

#[test]
fn native_instance_exposes_shared_prototype_and_proxy_meshes() -> Result<()> {
    let stage = characterization_stage();
    let window_a = stage.prim(sdf::path("/World/Window_A")?);
    let window_b = stage.prim(sdf::path("/World/Window_B")?);

    assert!(window_a.is_instance()?);
    assert!(window_b.is_instance()?);
    assert!(window_a.is_instanceable()?);
    assert_eq!(window_a.prototype()?, window_b.prototype()?);
    let prototype = window_a.prototype()?.expect("instance has a prototype");
    assert!(stage.prim(prototype.clone()).is_prototype());

    let child_paths = window_a
        .children()?
        .into_iter()
        .map(|child| child.path().as_str().to_string())
        .collect::<Vec<_>>();
    assert!(child_paths.iter().any(|path| path.ends_with("/Frame")));
    assert!(child_paths.iter().any(|path| path.ends_with("/Glass")));

    let mut proxy_paths = Vec::new();
    stage.traverse(PrimPredicate::DEFAULT_PROXIES, |path| {
        if path.as_str().starts_with("/World/Window_A/") {
            proxy_paths.push(path.as_str().to_string());
        }
    })?;
    for mesh in ["/World/Window_A/Frame", "/World/Window_A/Glass"] {
        assert!(
            proxy_paths.iter().any(|path| path == mesh),
            "missing {mesh}"
        );
        let proxy = stage.prim(sdf::path(mesh)?);
        assert!(proxy.is_instance_proxy()?);
        assert_eq!(
            proxy
                .prim_in_prototype()?
                .expect("proxy target")
                .path()
                .as_str(),
            prototype
                .append_path(mesh.rsplit('/').next().expect("mesh name"))?
                .as_str()
        );
    }
    Ok(())
}

#[test]
fn projection_plan_includes_scene_scoped_instance_proxies() -> Result<()> {
    let stage = characterization_stage();
    let plan = ProjectionPlan::from_stage(&stage)?;
    let paths = plan.paths().map(str::to_owned).collect::<Vec<_>>();

    assert!(paths.iter().any(|path| path == "/World/Window_A"));
    assert!(paths.iter().any(|path| path == "/World/Window_B"));
    assert!(paths.iter().any(|path| path == "/World/Control/Mesh"));
    assert!(paths.iter().any(|path| path == "/World/Window_A/Frame"));
    assert!(paths.iter().any(|path| path == "/World/Window_A/Glass"));
    let frame_index = paths
        .iter()
        .position(|path| path == "/World/Window_A/Frame")
        .expect("frame proxy is planned");
    let window_index = paths
        .iter()
        .position(|path| path == "/World/Window_A")
        .expect("instance root is planned");
    assert!(window_index < frame_index);
    Ok(())
}

#[test]
fn native_instance_proxy_meshes_share_render_handles() {
    let app = projected_app();
    let frame_a = projected_entity(&app, "/World/Window_A/Frame");
    let frame_b = projected_entity(&app, "/World/Window_B/Frame");

    let mesh_a = app.world().get::<Mesh3d>(frame_a).expect("frame A mesh");
    let mesh_b = app.world().get::<Mesh3d>(frame_b).expect("frame B mesh");
    assert_eq!(mesh_a.0, mesh_b.0, "instance proxies share a mesh handle");
    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(frame_a)
            .expect("frame A material")
            .0,
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(frame_b)
            .expect("frame B material")
            .0,
        "instance proxies share a material handle"
    );
    assert_eq!(
        app.world().resource::<Assets<Mesh>>().len(),
        3,
        "prototype frame, glass, and control geometry are interned"
    );
}

#[test]
fn native_instance_proxy_preserves_presentation_semantics() {
    let app = projected_app();
    let window_a = projected_entity(&app, "/World/Window_A");
    let window_b = projected_entity(&app, "/World/Window_B");
    let frame_a = projected_entity(&app, "/World/Window_A/Frame");
    let frame_b = projected_entity(&app, "/World/Window_B/Frame");
    let glass_b = projected_entity(&app, "/World/Window_B/Glass");

    assert_eq!(
        app.world().get::<Transform>(window_a).unwrap().translation,
        Vec3::new(-3.0, 0.0, 0.0)
    );
    assert_eq!(
        app.world().get::<Transform>(window_b).unwrap().translation,
        Vec3::new(3.0, 0.0, 0.0)
    );
    assert_eq!(
        app.world().get::<Visibility>(window_a),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        app.world().get::<Visibility>(frame_a),
        Some(&Visibility::Inherited)
    );
    assert_eq!(
        app.world().get::<Visibility>(frame_b),
        Some(&Visibility::Inherited)
    );

    for path in ["/World/Window_B/Frame", "/World/Window_B/Glass"] {
        let entity = projected_entity(&app, path);
        assert_eq!(
            app.world()
                .get::<UsdPurpose>(entity)
                .map(|purpose| purpose.0.as_str()),
            Some("proxy")
        );
    }

    let frame_material = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(frame_b)
        .expect("frame material")
        .0
        .clone();
    let frame = app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(&frame_material)
        .expect("frame material asset");
    assert_eq!(frame.alpha_mode, AlphaMode::Opaque);
    assert_eq!(frame.base_color.to_srgba().alpha, 1.0);

    let glass_material = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(glass_b)
        .expect("glass material")
        .0
        .clone();
    let glass = app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(&glass_material)
        .expect("glass material asset");
    assert_eq!(glass.alpha_mode, AlphaMode::Blend);
    assert_eq!(glass.base_color.to_srgba().alpha, 0.0);
}

#[test]
fn native_instance_reference_target_maps_proxy_to_source_path() -> Result<()> {
    let stage = characterization_stage();
    let instance = stage.prim(sdf::path("/World/Window_A")?);
    let proxy = stage.prim(sdf::path("/World/Window_A/Frame")?);
    let target = instance.edit_target_for_arc(EditTargetArc::Reference)?;
    assert_eq!(
        target
            .map_to_spec_path(proxy.path())
            .expect("reference target maps proxy"),
        sdf::path("/World/WindowPrototype/Frame")?
    );

    let mut index = NativeInstanceDependencyIndex::default();
    index.rebuild(&stage)?;
    assert_eq!(index.len(), 4, "all native proxy meshes are indexed");
    assert_eq!(
        index.dependents_for_path("/World/WindowPrototype/Frame"),
        std::collections::HashSet::from([
            "/World/Window_A/Frame".to_string(),
            "/World/Window_B/Frame".to_string(),
        ])
    );
    Ok(())
}

#[test]
fn shared_prototype_change_patches_only_scene_consumers() {
    let mut app = projected_app();
    let frame_a = projected_entity(&app, "/World/Window_A/Frame");
    let frame_b = projected_entity(&app, "/World/Window_B/Frame");
    let before = app.world().get::<Mesh3d>(frame_a).unwrap().0.clone();

    let live = app
        .world_mut()
        .remove_non_send::<LiveStage>()
        .expect("live stage exists");
    live.stage
        .prim(sdf::path("/World/WindowPrototype/Frame").unwrap())
        .attribute("points")
        .set(Value::Vec3fVec(vec![
            Vec3f::from([-1.5, 0.0, 0.0]),
            Vec3f::from([1.5, 0.0, 0.0]),
            Vec3f::from([1.5, 2.0, 0.0]),
            Vec3f::from([-1.5, 2.0, 0.0]),
        ]))
        .expect("prototype edit succeeds");
    let _ = live.drain_change_batch();
    let mut map = app
        .world_mut()
        .remove_resource::<PrimEntities>()
        .expect("prim map exists");
    let batch = StageChangeBatch {
        revision: LiveRevision(2),
        changes: vec![StageChange {
            resynced: Vec::new(),
            changed_info: vec!["/World/WindowPrototype/Frame.points".to_string()],
        }],
    };
    apply_change_batch(app.world_mut(), &live, &mut map, &batch);
    app.world_mut().insert_resource(map);
    app.world_mut().insert_non_send(live);

    let updated_a = app.world().get::<Mesh3d>(frame_a).unwrap().0.clone();
    let updated_b = app.world().get::<Mesh3d>(frame_b).unwrap().0.clone();
    assert_ne!(updated_a, before);
    assert_eq!(updated_a, updated_b);
    assert_eq!(
        app.world().resource::<ReconcileStats>().patched_entities,
        3,
        "source plus two scene proxies are patched without a stage scan"
    );
}
