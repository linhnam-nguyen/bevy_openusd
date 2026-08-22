use super::*;

use bevy_glacial::prelude::GroundGrid;
use viewport_protocol::{ColorRgb8, GroundGridOrigin, RenderMode, ViewerEnvironmentSettings};

#[test]
fn grid_projection_applies_color_visibility_and_world_origin_without_spawning() {
    let mut app = App::new();
    app.insert_resource(DisplayToggles {
        renderer: viewport_protocol::RendererConfiguration {
            grid: false,
            ..default()
        },
        ground_grid_origin: GroundGridOrigin::WorldOrigin,
        ..default()
    })
    .insert_resource(ViewerSettingsState::default())
    .insert_resource(SceneExtent {
        geometry_count: 1,
        geometry_min: Vec3::new(-2.0, 4.0, -2.0),
        geometry_max: Vec3::new(2.0, 8.0, 2.0),
        ..default()
    })
    .insert_resource(GroundGrid {
        visible: true,
        ground_y: Some(12.0),
        ..default()
    })
    .add_systems(Update, sync_ground_grid_to_scene);

    app.world_mut()
        .resource_mut::<ViewerSettingsState>()
        .environment_mut()
        .grid_color = ColorRgb8::new(0x12, 0x34, 0x56);

    app.update();

    let grid = app.world().resource::<GroundGrid>();
    assert!(!grid.visible);
    assert_eq!(grid.ground_y, Some(0.0));
    assert_eq!(
        grid.color,
        color_from_rgb8(ColorRgb8::new(0x12, 0x34, 0x56))
    );
}

#[test]
fn scene_origin_tracks_loaded_geometry_when_selected() {
    let mut app = App::new();
    app.insert_resource(DisplayToggles {
        renderer: viewport_protocol::RendererConfiguration {
            render_mode: RenderMode::Shaded,
            ..default()
        },
        ground_grid_origin: GroundGridOrigin::LoadedScene,
        ..default()
    })
    .insert_resource(ViewerSettingsState::default())
    .insert_resource(SceneExtent {
        geometry_count: 1,
        geometry_min: Vec3::new(-1.0, 6.0, -1.0),
        geometry_max: Vec3::new(1.0, 10.0, 1.0),
        ..default()
    })
    .insert_resource(GroundGrid {
        ground_y: Some(0.0),
        ..default()
    })
    .add_systems(Update, sync_ground_grid_to_scene);

    app.update();

    let expected = 6.0 + (Vec3::new(2.0, 4.0, 2.0).length() * 0.0005).clamp(0.0001, 0.05);
    let actual = app.world().resource::<GroundGrid>().ground_y.unwrap();
    assert!((actual - expected).abs() < f32::EPSILON);
}

#[test]
fn background_projection_updates_clear_color_only() {
    let mut app = App::new();
    app.insert_resource(ViewerSettingsState::default())
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Update, sync_background_color);
    app.world_mut()
        .resource_mut::<ViewerSettingsState>()
        .environment_mut()
        .background_color = ColorRgb8::new(0x11, 0x22, 0x33);

    app.update();

    assert_eq!(
        app.world().resource::<ClearColor>().0,
        color_from_rgb8(ColorRgb8::new(0x11, 0x22, 0x33))
    );
}

#[test]
fn fallback_surface_projection_updates_fallback_state_without_authored_materials() {
    let mut app = App::new();
    app.insert_resource(ViewerSettingsState::default())
        .add_systems(Update, sync_fallback_surface_color);
    app.world_mut()
        .resource_mut::<ViewerSettingsState>()
        .environment_mut()
        .default_surface_color = ViewerEnvironmentSettings::default().default_surface_color;

    app.update();

    let expected = color_from_rgb8(ViewerEnvironmentSettings::default().default_surface_color);
    assert_eq!(
        app.world().resource::<usd_bevy::FallbackMaterialColor>().0,
        expected
    );
}
