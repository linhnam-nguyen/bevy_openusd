use crate::live::ProjectionPlan;
use crate::{LiveStage, LiveStagePlugin, PrimEntities, UsdPlugin};
use anyhow::Result;
use bevy::asset::Assets;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::App;
use openusd::sdf;
use openusd::usd::{PrimPredicate, Stage};

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
