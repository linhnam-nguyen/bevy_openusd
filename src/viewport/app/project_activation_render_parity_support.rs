use std::sync::mpsc::sync_channel;

use bevy::{
    app::First,
    camera::Hdr,
    core_pipeline::tonemapping::Tonemapping,
    ecs::schedule::ScheduleLabel,
    prelude::*,
    render::{Render, RenderApp},
    window::{ExitCondition, WindowPlugin},
};
use project_protocol::ProjectActivationCommand;
use usd_bevy::{LiveStage, LiveStagePlugin, ProjectionBudget, ProjectionReadiness, UsdPlugin};
use usd_project::SceneId;
use viewport_streaming::{FrameTransportMetrics, ProjectActivationRequest};

use crate::viewport::{
    api::{
        ActiveHierarchyProvider, BimClassificationRecipeState, CurrentHierarchyProjection,
        RenderServerInterface, SceneAnchorIndex, refresh_active_hierarchy_projection,
        refresh_scene_anchor_index,
    },
    app::headless::HeadlessRenderPlugin,
    bim::BimClassificationFieldCatalogueState,
    camera::{ArcballCamera, apply_rig},
    diagnostics::performance::{RendererCounters, start_frame_timing_system},
    scene::{SceneExtent, SelectedPrim, SelectedTargets, extent::compute_extent},
    semantic::{SemanticSyncState, SemanticWorkingStore, synchronize_live_stage},
    session::{
        Spawned, StageInfo, StagePresentationContext, rehydrate_activation_presentation,
        spawn_when_ready,
    },
    transport::{
        FrameCapturePlugin,
        frame_signature::{FrameLumaSample, FrameSampleId, FrameSignatureDiagnostic},
    },
};

pub(super) struct RenderActivationWorld {
    app: App,
    _receiver: std::sync::mpsc::Receiver<viewport_streaming::VideoFrame>,
}

impl RenderActivationWorld {
    pub(super) fn new() -> Self {
        let (sender, receiver) = sync_channel::<viewport_streaming::VideoFrame>(4);
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                }),
        )
        .add_plugins(UsdPlugin)
        .add_plugins(usd_bevy::ExtendedSkinPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(HeadlessRenderPlugin {
            width: 640,
            height: 480,
        })
        .add_plugins(FrameCapturePlugin {
            sender,
            metrics: FrameTransportMetrics::default(),
            frame_signature: true,
        })
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
        .insert_resource(ProjectionBudget::bounded(
            32,
            std::time::Duration::from_millis(8),
        ))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.08, 0.12)))
        .insert_resource(ActiveHierarchyProvider::default())
        .init_resource::<BimClassificationRecipeState>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<BimClassificationFieldCatalogueState>()
        .init_resource::<crate::viewport::residency::SceneResidencyProjection>()
        .init_resource::<crate::viewport::animation::UsdStageTime>()
        .insert_resource(SemanticSyncState::with_config(
            usd_semantic::SemanticConfig::for_nvidia_revit_connector(),
        ))
        .init_resource::<SceneExtent>()
        .insert_resource(SemanticWorkingStore::default())
        .insert_resource(usd_bevy::PendingStageChanges::default())
        .insert_resource(usd_bevy::PrimEntities::default())
        .insert_resource(SelectedTargets::default())
        .insert_resource(SelectedPrim::default())
        .insert_resource(Spawned::default())
        .insert_resource(StageInfo::default())
        .insert_resource(StagePresentationContext::default())
        .insert_resource(RenderServerInterface::default())
        .insert_resource(super::super::ProjectActivationAuthorityRuntime::default())
        .init_resource::<RendererCounters>()
        .add_systems(First, start_frame_timing_system)
        .add_systems(
            Update,
            (
                refresh_scene_anchor_index,
                synchronize_live_stage,
                rehydrate_activation_presentation,
                refresh_active_hierarchy_projection,
            )
                .chain()
                .after(usd_bevy::LiveStageSet::Presentation),
        )
        .add_systems(
            Update,
            (
                crate::viewport::animation::tick_stage_time
                    .after(usd_bevy::LiveStageSet::Reconcile)
                    .before(usd_bevy::LiveStageSet::Animation),
                super::super::continue_deferred_stage_activation_for_test,
                spawn_when_ready,
            ),
        )
        .add_systems(
            Update,
            compute_extent.after(usd_bevy::LiveStageSet::Presentation),
        );
        super::super::canonical_visual_handoff::install(&mut app);
        app.world_mut().spawn((
            Camera3d::default(),
            Hdr,
            Tonemapping::AgX,
            Transform::from_xyz(3.0, 2.5, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
            ArcballCamera::default(),
        ));
        app.world_mut().spawn((
            DirectionalLight {
                illuminance: 5_000.0,
                ..default()
            },
            Transform::from_xyz(4.0, 6.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        app.world_mut()
            .insert_resource(bevy::light::GlobalAmbientLight {
                brightness: 200.0,
                ..default()
            });
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.update_schedule = Some(Render.intern());
        }
        app.finish();
        app.cleanup();
        Self {
            app,
            _receiver: receiver,
        }
    }

    pub(super) fn admit(&mut self, command: &ProjectActivationCommand, session: &str) -> bool {
        super::super::observe_project_activation_for_test(self.app.world_mut(), session, command)
    }

    pub(super) fn apply(
        &mut self,
        command: &ProjectActivationCommand,
        session: &str,
        target: Result<Option<crate::project::service::ProjectStageActivationTarget>, String>,
    ) -> Option<project_protocol::ProjectActivationReply> {
        let request = ProjectActivationRequest {
            session_id: viewport_protocol::SessionId::new(session),
            command: command.clone(),
        };
        super::super::apply_prepared_activation_for_test(self.app.world_mut(), &request, target)
    }

    pub(super) fn world(&self) -> &World {
        self.app.world()
    }

    pub(super) fn update(&mut self) {
        self.app.update();
    }

    pub(super) fn wait_ready(&mut self) {
        for _ in 0..20_000 {
            self.update();
            if self
                .world()
                .resource::<usd_bevy::ProgressiveProjectionState>()
                .readiness()
                == ProjectionReadiness::Ready
                && self.world().get_non_send::<LiveStage>().is_some()
            {
                self.frame_camera();
                return;
            }
        }
        panic!("render parity Project activation did not reach Ready");
    }

    pub(super) fn frame_camera(&mut self) {
        let extent = *self.world().resource::<SceneExtent>();
        let world = self.app.world_mut();
        let mesh_count = world.query::<&Mesh3d>().iter(world).count();
        assert!(mesh_count > 0, "render parity activation has no meshes");
        let mut query = world.query::<(&mut Transform, &mut ArcballCamera)>();
        let Ok((mut transform, mut camera)) = query.single_mut(world) else {
            panic!("render parity camera missing");
        };
        camera.focus = extent.centre();
        camera.distance = extent.diag().max(0.25) * 1.1;
        camera.zoom_target = camera.distance as f64;
        apply_rig(&camera, &mut transform);
    }

    pub(super) fn wait_for_cache_retirement(&mut self) {
        for _ in 0..20_000 {
            self.update();
            if self
                .world()
                .get_resource::<crate::viewport::session::SceneCachePresentation>()
                .is_none()
                && self
                    .world()
                    .resource::<crate::viewport::residency::SceneResidencyProjection>()
                    .active_entity_count_for_test()
                    == 0
            {
                assert_eq!(
                    self.world()
                        .resource::<usd_bevy::ProgressiveProjectionState>()
                        .readiness(),
                    ProjectionReadiness::Ready
                );
                assert!(self.world().get_non_send::<LiveStage>().is_some());
                assert!(
                    !self
                        .world()
                        .resource::<usd_bevy::AnimatedPrims>()
                        .0
                        .is_empty()
                );
                assert!(
                    self.world()
                        .get_resource::<crate::viewport::session::PendingCanonicalVisualHandoff>()
                        .is_none()
                );
                return;
            }
        }
        panic!("render parity cache handoff did not retire");
    }

    pub(super) fn sample_triplet(&mut self) -> [FrameLumaSample; 3] {
        let (start, end) = {
            let live = self
                .world()
                .get_non_send::<LiveStage>()
                .expect("live stage");
            (live.stage.start_time_code(), live.stage.end_time_code())
        };
        let t0 = start + (end - start) * 0.25;
        let t1 = start + (end - start) * 0.75;
        [
            self.sample(FrameSampleId::T0, t0),
            self.sample(FrameSampleId::T1, t1),
            self.sample(FrameSampleId::T0RoundTrip, t0),
        ]
    }

    fn sample(&mut self, id: FrameSampleId, time_code: f64) -> FrameLumaSample {
        let (start, fps) = {
            let clock = self
                .world()
                .resource::<crate::viewport::animation::UsdStageTime>();
            (clock.start_time_code, clock.time_codes_per_second)
        };
        {
            let clock = &mut self
                .app
                .world_mut()
                .resource_mut::<crate::viewport::animation::UsdStageTime>();
            clock.playing = false;
            clock.seconds = (time_code - start) / fps;
        }
        self.update();
        self.update();
        let minimum_sequence = self.world().resource::<RendererCounters>().frame_count + 1;
        self.app
            .world_mut()
            .resource_mut::<FrameSignatureDiagnostic>()
            .arm(id, minimum_sequence);
        for _ in 0..512 {
            self.update();
            if self
                .world()
                .resource::<FrameSignatureDiagnostic>()
                .capture(id)
                .is_some()
            {
                return self
                    .world()
                    .resource::<FrameSignatureDiagnostic>()
                    .capture(id)
                    .expect("captured frame")
                    .sample
                    .clone();
            }
        }
        panic!("render parity frame capture timed out for {id:?}");
    }
}

pub(super) fn install_visible_cache(world: &mut RenderActivationWorld, scene_id: SceneId) {
    let world = world.app.world_mut();
    let handle = world
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::from(Cuboid::default()));
    let mut projection = world
        .remove_resource::<crate::viewport::residency::SceneResidencyProjection>()
        .expect("cache projection resource");
    let payload = crate::viewport::residency::SceneSpatialPayload {
        address: crate::project::cache_contract::SceneCacheAddress {
            scene_id,
            occurrence: crate::project::cache_contract::SceneCacheOccurrence::Member(
                usd_project::SceneMemberId::new_v4(),
            ),
        },
        payload_key: crate::viewport::residency::ScenePayloadKey {
            scene_id,
            blob_hash: usd_model::HashDigest::new([7; usd_model::HashDigest::BYTE_LEN]),
        },
        transform: usd_project::ScenePlacementTransform::IDENTITY,
        bounds: usd_model::Bounds3 {
            min: [-1.0; 3],
            max: [1.0; 3],
        },
        cpu_bytes: 8,
        gpu_bytes: 8,
    };
    projection.install_scene(std::slice::from_ref(&payload));
    let mut queue = bevy::ecs::world::CommandQueue::default();
    {
        let mut commands = Commands::new(&mut queue, &*world);
        projection.attach_payload(payload.payload_key, handle, &mut commands);
    }
    queue.apply(world);
    world.insert_resource(projection);
}
