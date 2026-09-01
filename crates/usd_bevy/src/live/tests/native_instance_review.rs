use anyhow::Result;
use bevy::asset::Assets;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::StandardMaterial;
use bevy::prelude::{App, Transform, Vec3};
use openusd::gf::Vec3f;
use openusd::sdf::Value;
use openusd::usd::Stage;

use crate::live::{
    LiveStage, LiveStagePlugin, NativeInstanceDependencyIndex, PathStore, PendingStageChanges,
    PrimEntities,
};
use crate::{UsdPlugin, UsdSnippet, author_transform};

fn fixture_stage(fixture: &str) -> Stage {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stages")
        .join(fixture);
    Stage::open(path.to_str().expect("fixture path is valid"))
        .expect("native instance fixture opens")
}

fn projected_app(stage: Stage) -> App {
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(bevy::transform::TransformPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    app.world_mut().insert_non_send(LiveStage::new(stage));
    app.update();
    app
}

fn characterization_app() -> App {
    projected_app(fixture_stage("native_instance_characterization.usda"))
}

fn projected_entity(app: &App, path: &str) -> bevy::prelude::Entity {
    let world = app.world();
    world
        .resource::<PrimEntities>()
        .entity(world.resource::<PathStore>(), path)
        .unwrap_or_else(|| panic!("{path} entity exists"))
}

fn add_triangle_mesh(stage: &Stage, path: &str) -> Result<()> {
    let mesh = stage.define_prim(path)?.set_type_name("Mesh")?;
    mesh.create_attribute("faceVertexCounts", "int[]")?
        .set(Value::IntVec(vec![3]))?;
    mesh.create_attribute("faceVertexIndices", "int[]")?
        .set(Value::IntVec(vec![0, 1, 2]))?;
    mesh.create_attribute("points", "point3f[]")?
        .set(Value::Vec3fVec(vec![
            Vec3f::from([0.0, 0.0, 0.0]),
            Vec3f::from([1.0, 0.0, 0.0]),
            Vec3f::from([0.0, 1.0, 0.0]),
        ]))?;
    Ok(())
}

fn nested_consumers_stage() -> Stage {
    UsdSnippet::new(
        r#"#usda 1.0
def Xform "InnerPrototype"
{
    def Mesh "Leaf"
    {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    }
}

def Xform "OtherPrototype"
{
    def Mesh "OtherLeaf"
    {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    }
}

def Xform "OuterPrototype"
{
    def Xform "Nested" (
        instanceable = true
        references = </InnerPrototype>
    )
    {
    }
}

def Xform "Outer_A" (
    instanceable = true
    references = </OuterPrototype>
)
{
}

def Xform "Outer_B" (
    instanceable = true
    references = </OuterPrototype>
)
{
}

def Xform "Other_C" (
    instanceable = true
    references = </OtherPrototype>
)
{
}
"#,
    )
    .open_stage()
    .expect("nested native instance fixture opens")
}

#[test]
fn native_instance_structural_resync_projects_and_removes_all_proxy_consumers() -> Result<()> {
    let mut app = characterization_app();

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        add_triangle_mesh(&live.stage, "/World/WindowPrototype/Mullion")?;
    }
    app.update();

    assert!(
        app.world()
            .resource::<PendingStageChanges>()
            .batch()
            .is_some_and(|batch| batch.has_resync()),
        "prototype add must be delivered through the real stage-change sink"
    );

    let mullion_a = projected_entity(&app, "/World/Window_A/Mullion");
    let mullion_b = projected_entity(&app, "/World/Window_B/Mullion");
    assert!(app.world().get::<Mesh3d>(mullion_a).is_some());
    assert!(app.world().get::<Mesh3d>(mullion_b).is_some());
    assert_eq!(
        app.world()
            .get::<Mesh3d>(mullion_a)
            .expect("mullion A mesh")
            .0,
        app.world()
            .get::<Mesh3d>(mullion_b)
            .expect("mullion B mesh")
            .0,
        "new proxy consumers share the source mesh handle"
    );
    let world = app.world();
    let paths = world.resource::<PathStore>();
    let dependent_names = world
        .resource::<NativeInstanceDependencyIndex>()
        .dependents_for_path(paths, "/World/WindowPrototype/Mullion")
        .iter()
        .filter_map(|path| paths.path(*path))
        .map(str::to_owned)
        .collect::<std::collections::HashSet<_>>();
    assert!(
        dependent_names.is_superset(&std::collections::HashSet::from([
            "/World/Window_A/Mullion".to_owned(),
            "/World/Window_B/Mullion".to_owned(),
        ]))
    );

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        live.stage.remove_prim("/World/WindowPrototype/Mullion")?;
    }
    app.update();

    assert!(
        app.world()
            .resource::<PendingStageChanges>()
            .batch()
            .is_some_and(|batch| batch.has_resync()),
        "prototype removal must be delivered through the real stage-change sink"
    );

    let world = app.world();
    let paths = world.resource::<PathStore>();
    let map = world.resource::<PrimEntities>();
    assert!(map.entity(paths, "/World/Window_A/Mullion").is_none());
    assert!(map.entity(paths, "/World/Window_B/Mullion").is_none());
    assert!(
        world
            .resource::<NativeInstanceDependencyIndex>()
            .dependents_for_path(paths, "/World/WindowPrototype/Mullion")
            .is_empty()
    );
    Ok(())
}

#[test]
fn native_instance_removal_cleans_proxy_entities_and_dependency_records() -> Result<()> {
    let mut app = characterization_app();
    let window_b = projected_entity(&app, "/World/Window_B");
    let frame_b = projected_entity(&app, "/World/Window_B/Frame");
    let frame_b_mesh = app
        .world()
        .get::<Mesh3d>(frame_b)
        .expect("frame B mesh")
        .0
        .clone();

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        live.stage.remove_prim("/World/Window_A")?;
    }
    app.update();

    let world = app.world();
    let map = world.resource::<PrimEntities>();
    let paths = world.resource::<PathStore>();
    assert!(map.entity(paths, "/World/Window_A").is_none());
    assert!(map.entity(paths, "/World/Window_A/Frame").is_none());
    assert!(map.entity(paths, "/World/Window_A/Glass").is_none());
    assert_eq!(map.entity(paths, "/World/Window_B"), Some(window_b));
    assert_eq!(map.entity(paths, "/World/Window_B/Frame"), Some(frame_b));
    assert_eq!(
        app.world()
            .get::<Mesh3d>(frame_b)
            .expect("frame B remains")
            .0,
        frame_b_mesh
    );
    let index = world.resource::<NativeInstanceDependencyIndex>();
    assert_eq!(index.len(), 2, "only Window_B proxy meshes remain indexed");
    assert!(
        index
            .dependents_for_path(paths, "/World/WindowPrototype/Frame")
            .iter()
            .all(|path| paths
                .path(*path)
                .is_some_and(|path| path.starts_with("/World/Window_B/")))
    );
    Ok(())
}

#[test]
fn native_instance_transform_edit_preserves_shared_mesh_and_other_instance_transform() -> Result<()>
{
    let mut app = characterization_app();
    let window_a = projected_entity(&app, "/World/Window_A");
    let window_b = projected_entity(&app, "/World/Window_B");
    let frame_a = projected_entity(&app, "/World/Window_A/Frame");
    let frame_b = projected_entity(&app, "/World/Window_B/Frame");
    let frame_mesh = app
        .world()
        .get::<Mesh3d>(frame_a)
        .expect("frame A mesh")
        .0
        .clone();
    let before_b = *app
        .world()
        .get::<Transform>(window_b)
        .expect("window B transform");

    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        author_transform(
            &live.stage,
            "/World/Window_A",
            &Transform::from_translation(Vec3::new(-8.0, 0.0, 0.0)),
        )?;
    }
    app.update();

    assert!(
        app.world()
            .resource::<PendingStageChanges>()
            .batch()
            .is_some_and(|batch| {
                batch
                    .changes
                    .iter()
                    .flat_map(|change| change.changed_info.iter())
                    .any(|path| path.starts_with("/World/Window_A"))
            })
    );

    assert_eq!(
        app.world()
            .get::<Transform>(window_a)
            .expect("window A transform")
            .translation,
        Vec3::new(-8.0, 0.0, 0.0)
    );
    assert_eq!(
        app.world()
            .get::<Transform>(window_b)
            .expect("window B transform"),
        &before_b,
        "editing Window_A must not patch Window_B's local transform"
    );
    assert_eq!(
        app.world().get::<Mesh3d>(frame_a).expect("frame A mesh").0,
        frame_mesh
    );
    assert_eq!(
        app.world().get::<Mesh3d>(frame_b).expect("frame B mesh").0,
        frame_mesh
    );
    Ok(())
}

#[test]
fn nested_prototype_resync_reconciles_nested_consumers_and_excludes_unrelated_instances()
-> Result<()> {
    let mut app = projected_app(nested_consumers_stage());
    {
        let live = app.world().get_non_send::<LiveStage>().expect("live stage");
        add_triangle_mesh(&live.stage, "/InnerPrototype/Added")?;
    }
    app.update();

    assert!(
        app.world()
            .resource::<PendingStageChanges>()
            .batch()
            .is_some_and(|batch| batch.has_resync()),
        "nested prototype add must be delivered through the real stage-change sink"
    );

    for path in ["/Outer_A/Nested/Added", "/Outer_B/Nested/Added"] {
        let entity = projected_entity(&app, path);
        assert!(app.world().get::<Mesh3d>(entity).is_some(), "{path} mesh");
    }
    assert!(
        app.world()
            .resource::<PrimEntities>()
            .entity(app.world().resource::<PathStore>(), "/Other_C/Added",)
            .is_none()
    );
    assert!(
        app.world()
            .resource::<PrimEntities>()
            .entity(app.world().resource::<PathStore>(), "/Other_C/OtherLeaf",)
            .is_some()
    );
    Ok(())
}
