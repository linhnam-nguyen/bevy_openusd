use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use bevy::asset::Assets;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::App;
use openusd::usd::{PrimPredicate, Stage};

use crate::UsdPlugin;
use crate::live::{
    LiveStage, NativeInstanceDependencyIndex, PrimEntities, ProjectionPlan, project_stage,
};
use crate::snippet::UsdSnippet;

const REVIT_ASSET: &str = "../../../external_assets/Omniverse/V2/Projet1.usdc";

fn generated_instance_stage(instance_count: usize) -> Result<Stage> {
    let mut usda = String::from(
        r##"#usda 1.0
(
    defaultPrim = "World"
)

def Xform "World"
{
    def Xform "Prototype"
    {
        def Mesh "Leaf"
        {
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
            point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
        }
    }

"##,
    );
    for index in 0..instance_count {
        writeln!(
            usda,
            "    def Xform \"Instance{index:04}\" (\n\
                instanceable = true\n\
                references = </World/Prototype>\n\
            )\n    {{\n    }}\n",
        )?;
    }
    usda.push_str("}\n");
    UsdSnippet::new(usda).open_stage()
}

fn projected_app(stage: Stage) -> (App, PrimEntities, f64) {
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    let live = LiveStage::new(stage);
    let mut map = PrimEntities::default();
    let started = Instant::now();
    project_stage(app.world_mut(), &live, &mut map);
    (app, map, started.elapsed().as_secs_f64() * 1000.0)
}

#[test]
fn generated_thousand_native_instances_keep_projection_allocation_bounded() -> Result<()> {
    let stage = generated_instance_stage(1_000)?;
    let plan = ProjectionPlan::from_stage(&stage)?;
    let (mut app, map, projection_ms) = projected_app(stage.clone());

    let leaf_paths = map
        .iter()
        .filter(|(path, _)| path.starts_with("/World/Instance") && path.ends_with("/Leaf"))
        .map(|(path, _)| path.to_owned())
        .collect::<Vec<_>>();
    assert_eq!(leaf_paths.len(), 1_000);
    assert_eq!(
        app.world()
            .resource::<NativeInstanceDependencyIndex>()
            .len(),
        1_000,
        "one dependency edge per scene proxy leaf"
    );

    let mut mesh_handles = HashSet::new();
    let mut mesh_query = app
        .world_mut()
        .query::<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>();
    for (mesh, _material) in mesh_query.iter(app.world()) {
        mesh_handles.insert(mesh.0.id());
    }
    assert_eq!(
        mesh_handles.len(),
        1,
        "all instance leaves share one mesh asset"
    );
    assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 1);
    println!(
        "OR1-C8 generated audit: instances={} plan_entries={} mesh_assets={} projection_ms={projection_ms:.2}",
        leaf_paths.len(),
        plan.len(),
        mesh_handles.len(),
    );
    Ok(())
}

#[test]
#[ignore = "requires the local external Omniverse Revit asset"]
fn projet1_usdc_windows_project_to_scene_proxy_meshes() -> Result<()> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REVIT_ASSET);
    let stage = Stage::open(path.to_str().expect("Revit asset path is valid"))?;
    let mut window_roots = Vec::new();
    stage.traverse(PrimPredicate::DEFAULT_PROXIES, |path| {
        let prim = stage.prim(path.clone());
        let is_window = prim
            .edit_target_for_arc(openusd::usd::EditTargetArc::Reference)
            .ok()
            .and_then(|target| target.map_to_spec_path(path))
            .is_some_and(|source| source.as_str().contains("Fentres"));
        if is_window && prim.is_instance().unwrap_or(false) {
            window_roots.push(path.as_str().to_owned());
        }
    })?;
    assert!(
        window_roots.len() >= 4,
        "expected the Revit window instances, found {}",
        window_roots.len()
    );

    let mut proxy_mesh_paths = Vec::new();
    stage.traverse(PrimPredicate::DEFAULT_PROXIES, |path| {
        let prim = stage.prim(path.clone());
        let is_window_descendant = window_roots.iter().any(|root| {
            path.as_str()
                .strip_prefix(root)
                .is_some_and(|suffix| suffix.starts_with('/'))
        });
        if is_window_descendant
            && prim.is_instance_proxy().unwrap_or(false)
            && matches!(prim.type_name().ok().flatten().as_deref(), Some("Mesh"))
        {
            proxy_mesh_paths.push(path.as_str().to_owned());
        }
    })?;
    assert!(
        proxy_mesh_paths.len() >= window_roots.len(),
        "window instances have projectable proxy meshes"
    );

    let (app, map, projection_ms) = projected_app(stage);
    let mut projected_proxy_meshes = 0;
    for path in &proxy_mesh_paths {
        let Some(entity) = map.entity(path) else {
            continue;
        };
        if app.world().get::<Mesh3d>(entity).is_some() {
            projected_proxy_meshes += 1;
        }
    }
    assert_eq!(projected_proxy_meshes, proxy_mesh_paths.len());
    println!(
        "OR1-C8 Revit audit: window_roots={} proxy_meshes={} projected_proxy_meshes={} projection_ms={projection_ms:.2}",
        window_roots.len(),
        proxy_mesh_paths.len(),
        projected_proxy_meshes,
    );
    Ok(())
}
