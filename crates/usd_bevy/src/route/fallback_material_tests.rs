use bevy::asset::Assets;
use bevy::pbr::StandardMaterial;
use bevy::prelude::{Color, World};

use super::{fallback_material, set_fallback_material_color};

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
