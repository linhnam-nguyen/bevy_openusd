use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{App, Res, ResMut, Resource, Update, World};
use bevy::render::{Extract, ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems};
use usd_bevy::{LiveStage, ProgressiveProjectionState, ProjectionReadiness};
use usd_project::SceneId;

use crate::viewport::session::PendingCanonicalVisualHandoff;

#[derive(Clone)]
struct HandoffIdentity {
    activation_generation: u64,
    live_stage_identity: (u64, u64),
    scene_id: SceneId,
    scene_cache_generation: u64,
}

#[derive(Resource, Default)]
struct CanonicalVisualHandoffRenderWitness {
    identity: Option<HandoffIdentity>,
    canonical_ready: bool,
    frame_complete: bool,
}

pub(super) fn install(app: &mut App) {
    app.add_systems(
        Update,
        observe_canonical_visual_handoff.after(crate::viewport::session::spawn_when_ready),
    );
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<CanonicalVisualHandoffRenderWitness>()
        .add_systems(
            ExtractSchedule,
            (collect_canonical_frame_complete, snapshot_canonical_handoff).chain(),
        )
        .add_systems(
            Render,
            mark_canonical_frame_complete
                .after(RenderSystems::Render)
                .in_set(RenderSystems::Cleanup),
        );
}

pub(super) fn cancel(world: &mut World) {
    world.remove_resource::<PendingCanonicalVisualHandoff>();
}

pub(super) fn observe_canonical_visual_handoff(world: &mut World) {
    let Some(pending) = world
        .get_resource::<PendingCanonicalVisualHandoff>()
        .cloned()
    else {
        return;
    };
    let Some(live_stage_identity) = world
        .get_non_send::<LiveStage>()
        .map(LiveStage::stage_identity)
    else {
        cancel(world);
        return;
    };
    let Some(stage_info) = world.get_resource::<crate::viewport::session::StageInfo>() else {
        cancel(world);
        return;
    };
    let Some(presentation) =
        world.get_resource::<crate::viewport::session::SceneCachePresentation>()
    else {
        cancel(world);
        return;
    };
    if !pending.matches_identity(
        stage_info.activation_generation,
        live_stage_identity,
        &presentation.scene_id,
        presentation.generation,
    ) {
        cancel(world);
        return;
    }
    let activation_generation = stage_info.activation_generation;
    let scene_id = presentation.scene_id.clone();
    let scene_cache_generation = presentation.generation;

    let projection_ready = world
        .get_resource::<ProgressiveProjectionState>()
        .is_some_and(|state| state.readiness() == ProjectionReadiness::Ready);
    if !projection_ready {
        return;
    }
    if !pending.canonical_ready {
        world
            .resource_mut::<PendingCanonicalVisualHandoff>()
            .mark_canonical_ready();
        bevy::log::info!(
            "[cache-handoff] scene={} cache_generation={} activation_generation={} live_session={} state=canonical_ready",
            scene_id,
            scene_cache_generation,
            activation_generation,
            live_stage_identity.0,
        );
        return;
    }
    if !pending.canonical_frame_rendered {
        return;
    }

    crate::viewport::session::discard_scene_cache_bootstrap(
        world,
        scene_id.clone(),
        scene_cache_generation,
    );
    bevy::log::info!(
        "[cache-handoff] scene={} cache_generation={} activation_generation={} live_session={} state=cache_retired",
        scene_id,
        scene_cache_generation,
        activation_generation,
        live_stage_identity.0,
    );
}

fn collect_canonical_frame_complete(
    mut main_world: ResMut<MainWorld>,
    witness: Res<CanonicalVisualHandoffRenderWitness>,
) {
    if !witness.frame_complete {
        return;
    }
    let Some(identity) = witness.identity.as_ref() else {
        return;
    };
    mark_main_world_frame_rendered(&mut main_world, identity);
}

fn mark_main_world_frame_rendered(world: &mut World, identity: &HandoffIdentity) {
    let Some(mut pending) = world.get_resource_mut::<PendingCanonicalVisualHandoff>() else {
        return;
    };
    mark_rendered_if_matching(&mut pending, identity);
}

fn mark_rendered_if_matching(
    pending: &mut PendingCanonicalVisualHandoff,
    identity: &HandoffIdentity,
) {
    if pending.matches_identity(
        identity.activation_generation,
        identity.live_stage_identity,
        &identity.scene_id,
        identity.scene_cache_generation,
    ) {
        pending.mark_canonical_frame_rendered();
    }
}

fn snapshot_canonical_handoff(
    pending: Option<Extract<Res<PendingCanonicalVisualHandoff>>>,
    mut witness: ResMut<CanonicalVisualHandoffRenderWitness>,
) {
    witness.frame_complete = false;
    let Some(pending) = pending else {
        witness.identity = None;
        witness.canonical_ready = false;
        return;
    };
    if !pending.canonical_ready {
        witness.identity = None;
        witness.canonical_ready = false;
        return;
    }
    witness.identity = Some(HandoffIdentity {
        activation_generation: pending.activation_generation,
        live_stage_identity: (pending.live_stage_session_id, pending.live_stage_generation),
        scene_id: pending.scene_id,
        scene_cache_generation: pending.scene_cache_generation,
    });
    witness.canonical_ready = true;
}

fn mark_canonical_frame_complete(mut witness: ResMut<CanonicalVisualHandoffRenderWitness>) {
    mark_witness(&mut witness);
}

fn mark_witness(witness: &mut CanonicalVisualHandoffRenderWitness) {
    if witness.canonical_ready && witness.identity.is_some() {
        witness.frame_complete = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::{Assets, RenderAssetUsages};
    use bevy::ecs::system::Commands;
    use bevy::ecs::world::CommandQueue;
    use bevy::mesh::{Mesh, PrimitiveTopology};
    use std::path::PathBuf;
    use usd_model::{Bounds3, HashDigest};
    use usd_project::SceneMemberId;

    use crate::project::cache_contract::{
        SceneCacheAddress, SceneCacheOccurrence, SceneCacheState,
    };
    use crate::viewport::residency::{
        ScenePayloadKey, SceneResidencyOccurrence, SceneResidencyProjection, SceneSpatialPayload,
    };
    use crate::viewport::session::{SceneCacheOwnershipContext, SceneCachePresentation};
    use usd_project::SceneId;

    #[test]
    fn render_witness_does_not_mark_ready_until_a_render_boundary() {
        let scene_id = SceneId::new_v4();
        let pending = PendingCanonicalVisualHandoff::new(4, (8, 1), scene_id, 2);
        let mut witness = CanonicalVisualHandoffRenderWitness::default();
        snapshot_canonical_handoff_value(&pending, &mut witness);
        assert!(!witness.frame_complete);
        mark_witness(&mut witness);
        assert!(witness.frame_complete);
    }

    #[test]
    fn stale_render_witness_preserves_newer_world_state() {
        let old_scene = SceneId::new_v4();
        let new_scene = SceneId::new_v4();
        let payload = SceneSpatialPayload {
            address: SceneCacheAddress {
                scene_id: new_scene,
                occurrence: SceneCacheOccurrence::Member(SceneMemberId::new_v4()),
            },
            payload_key: ScenePayloadKey {
                scene_id: new_scene,
                blob_hash: HashDigest::new([7; HashDigest::BYTE_LEN]),
            },
            transform: usd_project::ScenePlacementTransform::IDENTITY,
            bounds: Bounds3 {
                min: [-1.0; 3],
                max: [1.0; 3],
            },
            cpu_bytes: 8,
            gpu_bytes: 8,
        };
        let mut projection = SceneResidencyProjection::default();
        projection.install_scene(std::slice::from_ref(&payload));
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        let handle = world.resource_mut::<Assets<Mesh>>().add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        ));
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, &world);
            projection.attach_payload(payload.payload_key, handle, &mut commands);
        }
        queue.apply(&mut world);
        world.insert_resource(projection);
        world.insert_resource(PendingCanonicalVisualHandoff::new(8, (12, 1), new_scene, 4));
        world.insert_resource(SceneCachePresentation {
            scene_id: new_scene,
            generation: 4,
            state: SceneCacheState::Partial,
            entries: Vec::new(),
        });
        world.insert_resource(SceneCacheOwnershipContext {
            project_root: PathBuf::from("/cache/new"),
            scene_id: new_scene,
            config_hash: HashDigest::new([4; HashDigest::BYTE_LEN]),
        });
        let active_before = world
            .resource::<SceneResidencyProjection>()
            .active_entity_count_for_test();
        let occurrence_count_before = world
            .query::<&SceneResidencyOccurrence>()
            .iter(&world)
            .count();
        let stale = HandoffIdentity {
            activation_generation: 7,
            live_stage_identity: (11, 1),
            scene_id: old_scene,
            scene_cache_generation: 3,
        };

        mark_main_world_frame_rendered(&mut world, &stale);

        assert!(
            !world
                .resource::<PendingCanonicalVisualHandoff>()
                .canonical_frame_rendered
        );
        assert_eq!(world.resource::<SceneCachePresentation>().generation, 4);
        assert_eq!(
            world.resource::<SceneCacheOwnershipContext>().scene_id,
            new_scene
        );
        assert_eq!(
            world
                .resource::<SceneResidencyProjection>()
                .active_entity_count_for_test(),
            active_before
        );
        assert_eq!(
            world
                .query::<&SceneResidencyOccurrence>()
                .iter(&world)
                .count(),
            occurrence_count_before
        );
    }

    fn snapshot_canonical_handoff_value(
        pending: &PendingCanonicalVisualHandoff,
        witness: &mut CanonicalVisualHandoffRenderWitness,
    ) {
        witness.identity = Some(HandoffIdentity {
            activation_generation: pending.activation_generation,
            live_stage_identity: (pending.live_stage_session_id, pending.live_stage_generation),
            scene_id: pending.scene_id,
            scene_cache_generation: pending.scene_cache_generation,
        });
        witness.canonical_ready = true;
        witness.frame_complete = false;
    }
}
