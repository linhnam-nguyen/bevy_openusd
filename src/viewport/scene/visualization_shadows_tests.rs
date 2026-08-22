use super::*;

#[test]
fn shadows_disable_globally_and_restore_each_authored_setting() {
    let mut app = App::new();
    let authored_on = app
        .world_mut()
        .spawn(DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        })
        .id();
    let authored_off = app
        .world_mut()
        .spawn(DirectionalLight {
            shadow_maps_enabled: false,
            ..default()
        })
        .id();
    app.insert_resource(DisplayToggles::default()).add_systems(
        Update,
        (capture_original_shadow_settings, apply_shadow_toggle).chain(),
    );

    app.update();
    app.update();
    assert!(
        app.world()
            .get::<DirectionalLight>(authored_on)
            .unwrap()
            .shadow_maps_enabled
    );
    assert!(
        !app.world()
            .get::<DirectionalLight>(authored_off)
            .unwrap()
            .shadow_maps_enabled
    );

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .shadows = false;
    app.update();
    assert!(
        !app.world()
            .get::<DirectionalLight>(authored_on)
            .unwrap()
            .shadow_maps_enabled
    );
    assert!(
        !app.world()
            .get::<DirectionalLight>(authored_off)
            .unwrap()
            .shadow_maps_enabled
    );

    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .shadows = true;
    app.update();
    assert!(
        app.world()
            .get::<DirectionalLight>(authored_on)
            .unwrap()
            .shadow_maps_enabled
    );
    assert!(
        !app.world()
            .get::<DirectionalLight>(authored_off)
            .unwrap()
            .shadow_maps_enabled
    );
}
