use super::edge::{EdgeOverlay, EdgeOverlayMaterial};
use super::*;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy_glacial::prelude::{GizmoOptions, GroundGrid};
use usd_bevy::UsdPrimRef;
use viewport_protocol::RenderMode;

#[test]
fn gizmo_size_level_uses_log_scale_for_both_gizmo_paths() {
    let mut app = App::new();
    app.init_resource::<ViewerSettingsState>()
        .init_resource::<GizmoOptions>()
        .add_systems(Update, sync_gizmo_size);

    app.world_mut()
        .resource_mut::<ViewerSettingsState>()
        .set_gizmo_size_level(2);
    app.update();
    assert_eq!(app.world().resource::<GizmoOptions>().gizmo_size_scale, 1.0);

    app.world_mut()
        .resource_mut::<ViewerSettingsState>()
        .set_gizmo_size_level(10);
    app.update();
    assert!((app.world().resource::<GizmoOptions>().gizmo_size_scale - 10.0).abs() < 1e-5);

    app.world_mut()
        .resource_mut::<ViewerSettingsState>()
        .set_gizmo_size_level(6);
    app.update();
    assert!(
        (app.world().resource::<GizmoOptions>().gizmo_size_scale - 10.0_f32.sqrt()).abs() < 1e-5
    );
}

fn triangle_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
    mesh
}

fn edge_child(world: &World, source: Entity) -> Option<Entity> {
    world
        .get::<Children>(source)?
        .iter()
        .find(|child| world.get::<EdgeOverlay>(*child).is_some())
}

#[test]
fn grid_visibility_reads_the_authoritative_renderer_configuration() {
    let mut app = App::new();
    app.insert_resource(DisplayToggles::default())
        .insert_resource(GroundGrid {
            visible: true,
            ..default()
        })
        .add_systems(Update, sync_ground_grid_visibility);

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .grid = false;
    app.update();

    assert!(!app.world().resource::<GroundGrid>().visible);
}

#[test]
fn render_mode_round_trip_updates_bevy_wireframe_without_touching_edges() {
    let mut app = App::new();
    app.insert_resource(DisplayToggles {
        renderer: RendererConfiguration {
            edges: true,
            ..default()
        },
        ..default()
    })
    .insert_resource(bevy::pbr::wireframe::WireframeConfig {
        global: false,
        ..default()
    })
    .add_systems(Update, apply_wireframe_toggle);

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .render_mode = RenderMode::Wireframe;
    app.update();
    assert!(
        app.world()
            .resource::<bevy::pbr::wireframe::WireframeConfig>()
            .global
    );
    assert!(app.world().resource::<DisplayToggles>().renderer.edges);

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .render_mode = RenderMode::Shaded;
    app.update();
    assert!(
        !app.world()
            .resource::<bevy::pbr::wireframe::WireframeConfig>()
            .global
    );
    assert!(app.world().resource::<DisplayToggles>().renderer.edges);
}

#[test]
fn full_renderer_configuration_matrix_preserves_each_independent_option() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(DisplayToggles::default())
        .insert_resource(GroundGrid {
            visible: true,
            ..default()
        })
        .init_resource::<EdgeOverlayCache>()
        .init_resource::<EdgeOverlayStats>()
        .init_resource::<ShadowProjectionState>()
        .insert_resource(bevy::pbr::wireframe::WireframeConfig::default())
        .add_systems(
            Update,
            (
                sync_ground_grid_visibility,
                capture_original_shadow_settings,
                apply_shadow_toggle,
                apply_wireframe_toggle,
                sync_edge_overlays,
            )
                .chain(),
        );

    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.insert_resource(EdgeOverlayMaterial(material));
    let source_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(triangle_mesh());
    let source = app
        .world_mut()
        .spawn((UsdPrimRef::new("/Triangle"), Mesh3d(source_mesh)))
        .id();
    let light = app
        .world_mut()
        .spawn(DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        })
        .id();

    app.update();

    for grid in [false, true] {
        for shadows in [false, true] {
            for edges in [false, true] {
                for render_mode in [RenderMode::Shaded, RenderMode::Wireframe] {
                    app.world_mut().resource_mut::<DisplayToggles>().renderer =
                        RendererConfiguration {
                            grid,
                            shadows,
                            edges,
                            render_mode,
                            preferred_fps: Some(60),
                        };
                    app.update();

                    assert_eq!(app.world().resource::<GroundGrid>().visible, grid);
                    assert_eq!(
                        app.world()
                            .get::<DirectionalLight>(light)
                            .unwrap()
                            .shadow_maps_enabled,
                        shadows
                    );
                    assert_eq!(
                        app.world()
                            .resource::<bevy::pbr::wireframe::WireframeConfig>()
                            .global,
                        render_mode == RenderMode::Wireframe
                    );
                    assert_eq!(app.world().resource::<EdgeOverlayStats>().enabled, edges);

                    if edges {
                        let child = edge_child(app.world(), source)
                            .expect("enabled edges must retain a cached overlay child");
                        assert_eq!(
                            app.world().get::<Visibility>(child),
                            Some(&Visibility::Inherited)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn edge_overlay_is_independent_from_wireframe_for_all_four_combinations() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(DisplayToggles::default())
        .init_resource::<EdgeOverlayCache>()
        .init_resource::<EdgeOverlayStats>()
        .insert_resource(bevy::pbr::wireframe::WireframeConfig::default())
        .add_systems(Update, (apply_wireframe_toggle, sync_edge_overlays).chain());
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.insert_resource(EdgeOverlayMaterial(material));
    let source_mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(triangle_mesh());
    let source = app
        .world_mut()
        .spawn((UsdPrimRef::new("/Triangle"), Mesh3d(source_mesh)))
        .id();

    app.update();
    assert!(!app.world().resource::<EdgeOverlayStats>().enabled);
    assert_eq!(edge_child(app.world(), source), None);
    assert!(
        !app.world()
            .resource::<bevy::pbr::wireframe::WireframeConfig>()
            .global
    );

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .edges = true;
    app.update();
    let child = edge_child(app.world(), source).expect("edge pass should create a child mesh");
    assert_eq!(
        app.world().get::<Visibility>(child),
        Some(&Visibility::Inherited)
    );
    assert!(
        app.world()
            .get::<bevy::pbr::wireframe::NoWireframe>(child)
            .is_some()
    );
    assert!(
        !app.world()
            .resource::<bevy::pbr::wireframe::WireframeConfig>()
            .global
    );
    let edge_mesh = app
        .world()
        .resource::<Assets<Mesh>>()
        .get(app.world().get::<Mesh3d>(child).unwrap().0.id())
        .expect("edge child must reference cached line geometry");
    assert_eq!(edge_mesh.primitive_topology(), PrimitiveTopology::LineList);
    assert_eq!(edge_mesh.indices().unwrap().len(), 6);
    assert_eq!(app.world().resource::<EdgeOverlayStats>().mesh_builds, 1);

    app.world_mut().resource_mut::<DisplayToggles>().renderer = RendererConfiguration {
        edges: false,
        render_mode: RenderMode::Wireframe,
        ..Default::default()
    };
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(child),
        Some(&Visibility::Hidden)
    );
    assert!(
        app.world()
            .resource::<bevy::pbr::wireframe::WireframeConfig>()
            .global
    );

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .edges = true;
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(child),
        Some(&Visibility::Inherited)
    );
    assert!(
        app.world()
            .resource::<bevy::pbr::wireframe::WireframeConfig>()
            .global
    );
    assert_eq!(app.world().resource::<EdgeOverlayStats>().mesh_builds, 1);
}

#[test]
fn edge_mesh_deduplicates_shared_triangle_edges_and_rejects_points() {
    let mut quad = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD,
    );
    quad.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    );
    quad.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    let edges = edge_mesh::build_edge_mesh(&quad).expect("triangle topology should produce edges");
    assert_eq!(edges.indices().unwrap().len(), 10);

    let points = Mesh::new(PrimitiveTopology::PointList, RenderAssetUsages::MAIN_WORLD);
    assert!(edge_mesh::build_edge_mesh(&points).is_none());
}
