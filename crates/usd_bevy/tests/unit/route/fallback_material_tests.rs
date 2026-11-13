use bevy::asset::Assets;
use bevy::pbr::StandardMaterial;
use bevy::prelude::{App, Color, MinimalPlugins, Update, World};

use super::{
    FallbackMaterialColor, fallback_material, set_fallback_material_color,
    sync_fallback_material_color,
};

#[test]
fn fallback_color_updates_one_shared_material_without_allocating_another() {
    let mut world = World::new();
    world.init_resource::<Assets<StandardMaterial>>();
    let first = fallback_material(&mut world);
    let before = world.resource::<Assets<StandardMaterial>>().len();

    let green = Color::srgb(0.1, 0.8, 0.2);
    set_fallback_material_color(&mut world, green);

    let second = fallback_material(&mut world);
    let material = world
        .resource::<Assets<StandardMaterial>>()
        .get(&second)
        .expect("fallback material remains alive");
    assert_eq!(first, second);
    assert_eq!(before, 1);
    assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), before);
    assert_eq!(material.base_color, green);
}

#[test]
fn changed_fallback_color_updates_shared_asset_without_idle_work() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<FallbackMaterialColor>()
        .add_systems(Update, sync_fallback_material_color);
    let first = fallback_material(app.world_mut());
    let before = app.world().resource::<Assets<StandardMaterial>>().len();
    let green = Color::srgb(0.1, 0.8, 0.2);

    app.world_mut().resource_mut::<FallbackMaterialColor>().0 = green;
    app.update();

    let material = app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(&first)
        .expect("fallback material remains alive");
    assert_eq!(
        app.world().resource::<Assets<StandardMaterial>>().len(),
        before
    );
    assert_eq!(material.base_color, green);

    app.update();
    assert_eq!(
        app.world().resource::<Assets<StandardMaterial>>().len(),
        before
    );
}
