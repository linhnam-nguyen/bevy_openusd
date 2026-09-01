use bevy::prelude::App;

pub(super) fn enable_skinning_profile(app: &mut App) {
    if std::env::var_os("USDHUB_SKIN_PROFILE").is_some()
        && let Some(mut profile) = app
            .world_mut()
            .get_resource_mut::<usd_bevy::SkinningProfile>()
    {
        profile.enabled = true;
    }
}

pub(super) fn enable_c12_counters(app: &mut App) {
    if std::env::var_os("USDHUB_C12_DIAGNOSTIC").is_some() {
        app.world_mut()
            .resource_mut::<usd_bevy::PerformanceCounters>()
            .enabled = true;
    }
}

pub(super) fn enable_mesh_profile(app: &mut App, enabled: bool) {
    if enabled {
        let mut profile = app.world_mut().resource_mut::<usd_bevy::GeometryProfile>();
        profile.enabled = true;
        profile.top_n = 128;
    }
}
