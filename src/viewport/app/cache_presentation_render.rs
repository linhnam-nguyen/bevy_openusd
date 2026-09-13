use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{App, Res, ResMut, Resource};
use bevy::render::{Extract, ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems};
use usd_project::SceneId;

use crate::viewport::session::CachePresentationGate;

#[derive(Resource, Default)]
struct CachePresentationRenderWitness {
    scene_id: Option<SceneId>,
    generation: Option<u64>,
    presentation_ready: bool,
    frame_complete: bool,
}

pub(super) fn install(app: &mut App) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<CachePresentationRenderWitness>()
        .add_systems(
            ExtractSchedule,
            (collect_rendered_cache_frame, snapshot_cache_presentation).chain(),
        )
        .add_systems(
            Render,
            mark_cache_frame_complete
                .after(RenderSystems::Render)
                .in_set(RenderSystems::Cleanup),
        );
}

fn collect_rendered_cache_frame(
    mut main_world: ResMut<MainWorld>,
    witness: Res<CachePresentationRenderWitness>,
) {
    if !witness.frame_complete {
        return;
    }
    let Some(scene_id) = witness.scene_id.as_ref() else {
        return;
    };
    let Some(generation) = witness.generation else {
        return;
    };
    if let Some(mut gate) = main_world.get_resource_mut::<CachePresentationGate>() {
        gate.observe_rendered_frame(scene_id.clone(), generation);
    }
}

fn snapshot_cache_presentation(
    gate: Option<Extract<Res<CachePresentationGate>>>,
    mut witness: ResMut<CachePresentationRenderWitness>,
) {
    let Some(gate) = gate else {
        *witness = CachePresentationRenderWitness::default();
        return;
    };
    witness.scene_id = Some(gate.scene_id.clone());
    witness.generation = Some(gate.generation);
    witness.presentation_ready = gate.presentation_ready;
    witness.frame_complete = false;
}

fn mark_cache_frame_complete(mut witness: ResMut<CachePresentationRenderWitness>) {
    if witness.presentation_ready {
        witness.frame_complete = true;
    }
}
