use super::*;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use usd_bevy::UsdPrimRef;

fn test_app() -> (
    App,
    Entity,
    Handle<StandardMaterial>,
    Handle<StandardMaterial>,
) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<StandardMaterial>()
        .insert_resource(DisplayToggles::default())
        .insert_resource(UniformRenderMaterial(Handle::default()))
        .add_systems(Update, apply_render_mode);

    let authored = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let uniform = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: UNIFORM_COLOR,
            perceptual_roughness: 1.0,
            ..default()
        });
    app.world_mut()
        .insert_resource(UniformRenderMaterial(uniform.clone()));
    let entity = app
        .world_mut()
        .spawn((
            UsdPrimRef::new("/Triangle"),
            MeshMaterial3d(authored.clone()),
        ))
        .id();

    (app, entity, authored, uniform)
}

#[test]
fn uniform_color_rebinds_and_shaded_restores_each_authored_material() {
    let (mut app, entity, authored, uniform) = test_app();

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .render_mode = RenderMode::UniformColor;
    app.update();

    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .unwrap()
            .0,
        uniform
    );
    assert!(app.world().get::<OriginalRenderMaterial>(entity).is_some());

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .render_mode = RenderMode::Shaded;
    app.update();

    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .unwrap()
            .0,
        authored
    );
    assert!(app.world().get::<OriginalRenderMaterial>(entity).is_none());
}

#[test]
fn uniform_color_tracks_material_route_changes_before_restoring() {
    let (mut app, entity, _, uniform) = test_app();
    let replacement = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .render_mode = RenderMode::UniformColor;
    app.update();
    app.world_mut()
        .get_mut::<MeshMaterial3d<StandardMaterial>>(entity)
        .unwrap()
        .0 = replacement.clone();
    app.update();

    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .unwrap()
            .0,
        uniform
    );

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .render_mode = RenderMode::Shaded;
    app.update();
    assert_eq!(
        app.world()
            .get::<MeshMaterial3d<StandardMaterial>>(entity)
            .unwrap()
            .0,
        replacement
    );
}
