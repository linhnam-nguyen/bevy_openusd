use super::*;

fn prim(x: f32) -> ExtentPrim {
    ExtentPrim {
        global: GlobalTransform::from(Transform::from_xyz(x, 0.0, 0.0)),
        local: Some(usd_bevy::UsdLocalExtent {
            min: [-1.0, -1.0, -1.0],
            max: [1.0, 1.0, 1.0],
        }),
        aabb: None,
        is_geometry: true,
    }
}

#[test]
fn transformed_prim_recompute_keeps_unchanged_global_bounds() {
    let extent = recompute_scene_extent([prim(-100.0), prim(0.0), prim(100.0)]);
    let moved = prim(25.0);
    let updated = recompute_scene_extent([prim(-100.0), moved, prim(100.0)]);

    assert_eq!(extent.count, 3);
    assert_eq!(updated.count, 3);
    assert_eq!(updated.min, Vec3::new(-101.0, -1.0, -1.0));
    assert_eq!(updated.max, Vec3::new(101.0, 1.0, 1.0));
}

#[test]
fn add_and_remove_recompute_over_all_current_prims() {
    let added = prim(300.0);
    let with_added = recompute_scene_extent([prim(-100.0), prim(100.0), added]);
    let after_remove = recompute_scene_extent([prim(-100.0), added]);

    assert_eq!(with_added.count, 3);
    assert_eq!(with_added.max.x, 301.0);
    assert_eq!(after_remove.count, 2);
    assert_eq!(after_remove.min.x, -101.0);
    assert_eq!(after_remove.max.x, 301.0);
}

#[test]
fn removing_last_prim_returns_empty_extent() {
    let empty = recompute_scene_extent(std::iter::empty());

    assert_eq!(empty, SceneExtent::default());
}

#[test]
fn reload_replaces_previous_stage_bounds() {
    let reloaded = recompute_scene_extent([prim(-7.0), prim(12.0)]);

    assert_eq!(reloaded.count, 2);
    assert_eq!(reloaded.min.x, -8.0);
    assert_eq!(reloaded.max.x, 13.0);
    assert_eq!(reloaded.geometry_count, 2);
}

#[test]
fn local_extent_uses_world_transform_for_root_or_z_up_scene() {
    let mut root = prim(0.0);
    root.global = GlobalTransform::from(
        Transform::from_xyz(10.0, 20.0, 30.0)
            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
    );
    let extent = recompute_scene_extent([root]);

    assert_eq!(extent.count, 1);
    assert_eq!(extent.min, Vec3::new(9.0, 19.0, 29.0));
    assert_eq!(extent.max, Vec3::new(11.0, 21.0, 31.0));
}
