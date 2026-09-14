use bevy::prelude::*;
use usd_bevy::{LiveStage, ProjectionReadiness};
use usd_project::SceneId;

use super::{asset_path, open_stage, playback_app, settle_stage};
use crate::project::cache_contract::SceneCacheState;
use crate::viewport::session::SceneCachePresentation;

#[test]
fn cache_first_presentation_is_not_canonical_animation_readiness() {
    let mut app = playback_app();
    app.world_mut().insert_resource(SceneCachePresentation {
        scene_id: SceneId::new_v4(),
        generation: 1,
        state: SceneCacheState::Ready,
        entries: Vec::new(),
    });

    assert!(app.world().get_non_send::<LiveStage>().is_none());
    assert_ne!(
        app.world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Ready,
        "cache-first presentation must not announce canonical readiness"
    );

    app.world_mut()
        .insert_non_send(LiveStage::new(open_stage(&asset_path("hummingbird.usdz"))));
    let ready = settle_stage(&mut app, true);
    assert_eq!(
        ready.identity,
        app.world()
            .get_non_send::<LiveStage>()
            .unwrap()
            .stage_identity()
    );
    assert_eq!(
        app.world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        ProjectionReadiness::Ready
    );
}
