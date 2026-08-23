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
    app.insert_resource(DisplayToggles::default())
        .insert_resource(ShadowProjectionState::default())
        .insert_resource(ShadowProjectionStats::default())
        .add_systems(
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
    assert_eq!(
        app.world()
            .resource::<ShadowProjectionStats>()
            .full_light_visits,
        0
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
    assert_eq!(
        app.world()
            .resource::<ShadowProjectionStats>()
            .full_light_visits,
        2
    );

    let newly_added = app
        .world_mut()
        .spawn(DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        })
        .id();
    app.update();
    assert!(
        !app.world()
            .get::<DirectionalLight>(newly_added)
            .unwrap()
            .shadow_maps_enabled
    );
    let full_visits_after_new_light = app
        .world()
        .resource::<ShadowProjectionStats>()
        .full_light_visits;
    assert_eq!(full_visits_after_new_light, 2);
    app.update();
    assert_eq!(
        app.world()
            .resource::<ShadowProjectionStats>()
            .full_light_visits,
        full_visits_after_new_light
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
