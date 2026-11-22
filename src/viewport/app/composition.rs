//! Shared Bevy application composition boundary for native and headless hosts.
//!
//! Splits host-independent USD/viewport plugin wiring from host-specific
//! windowing, GPU device creation, and transport runners.

use std::path::PathBuf;

use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy_glacial::prelude::{
    AxisGizmoPlugin, GizmoAutoScale, GroundGrid, GroundGridPlugin, TransformGizmoPlugin,
    auto_scale_gizmo_to_target,
};
use bevy_mod_outline::OutlinePlugin;
use usd_bevy::{LiveStagePlugin, LiveStageSet, UsdPlugin};
use usd_semantic::SemanticConfig;

use super::cadence::RendererCadence;
use super::launch_options::LaunchOptions;
use super::runner_projection;
use super::scene::{DefaultSun, resolve_requested_asset, spawn_camera_and_ground};
use crate::project::semantic_store::sync::TursoClientSyncRuntime;
use crate::viewport::animation::{self, UsdStageTime};
use crate::viewport::api::{
    RenderServerInterface, ViewerSettingsState, ViewportBridgePlugin, ViewportBridgeSet,
    ViewportCommandInbox, ViewportEventOutbox, ViewportReadModelState, ViewportTreeCommandInbox,
};
use crate::viewport::camera::{
    ArcballCameraPlugin, CameraBookmarks, CameraMount, FlyTo, apply_fly_to, fit_camera_once,
    follow_mounted_camera, sync_chase_camera,
};
use crate::viewport::physics::{PhysicsActive, RapierPhysicsPlugin};
use crate::viewport::rendering::presentation::{DisplayToggles, OverlaysPlugin};
use crate::viewport::rendering::sampling::{DlssProviderPlugin, SamplingCoordinatorPlugin};
use crate::viewport::scene::{
    HideMeshesFlag, SceneAnchorIndex, SectionClipMaterial, SelectedPrim, SelectedTargets,
    SelectionOutlineState, ShowJointGizmosFlag, SkeletonGizmos, SolariCapabilityPlugin,
    hide_meshes_on_startup, setup_skeleton_gizmos_on_top, sync_selected_instance_identity,
    sync_selected_renderable_projection, sync_selection_outlines,
};
use crate::viewport::semantic::{
    SemanticSyncRuntimeResource, SemanticSyncState, process_semantic_sync_requests,
    synchronize_live_stage,
};
use crate::viewport::stage::{
    LoadRequest, LoaderTuning, ReloadRequest, RequestedAsset, Spawned, StageInfo,
    apply_load_request, handle_usd_hot_reload, load_stage, poll_scene_cache_revalidation,
    spawn_when_ready,
};
use crate::viewport::transport::ViewportTransport;

/// Host-independent configuration for the generic USD viewport plugin graph.
#[derive(Debug, Clone)]
pub(crate) struct GenericViewportConfig {
    pub(crate) asset_path: String,
    pub(crate) asset_root: PathBuf,
    pub(crate) fps: u32,
    pub(crate) benchmark_mesh_profile: bool,
    pub(crate) skip_initial_stage_load: bool,
}

impl GenericViewportConfig {
    pub(crate) fn from_launch_options(options: &LaunchOptions, is_s8: bool) -> Self {
        let (asset_path, asset_root) = resolve_requested_asset(options.asset_argument.clone());
        Self {
            asset_path: if is_s8 { String::new() } else { asset_path },
            asset_root,
            fps: options.fps,
            benchmark_mesh_profile: options.benchmark_mesh_profile,
            skip_initial_stage_load: is_s8,
        }
    }
}

/// Idempotency guards so repeated composition calls cannot double-register systems.
#[derive(Resource, Default)]
struct GenericViewportCoreConfigured;

#[derive(Resource, Default)]
struct GenericViewportRenderConfigured;

/// Installs the core generic USD viewport plugins, resources, and systems
/// without requiring Project, Git, Turso, cache, or RenderPlugin-only overlays.
pub(crate) fn configure_generic_viewport_core(app: &mut App, config: &GenericViewportConfig) {
    if app.world().contains_resource::<GenericViewportCoreConfigured>() {
        return;
    }
    app.init_resource::<GenericViewportCoreConfigured>();

    if !app.is_plugin_added::<UsdPlugin>() {
        app.add_plugins(UsdPlugin);
    }
    if config.benchmark_mesh_profile {
        let mut profile = app.world_mut().resource_mut::<usd_bevy::GeometryProfile>();
        profile.enabled = true;
        profile.top_n = 128;
    }

    if !app.is_plugin_added::<LiveStagePlugin>() {
        app.add_plugins(LiveStagePlugin);
    }
    runner_projection::configure(app);

    if !app.is_plugin_added::<RapierPhysicsPlugin>() {
        app.add_plugins(RapierPhysicsPlugin);
    }
    if !app.is_plugin_added::<ArcballCameraPlugin>() {
        app.add_plugins(ArcballCameraPlugin);
    }

    app.insert_resource(ClearColor(Color::srgb(0.06, 0.08, 0.12)))
        .insert_resource(GroundGrid {
            visible: true,
            color: Color::srgba(0.30, 0.38, 0.50, 0.42),
            ground_y: None,
            coverage_radius: bevy_glacial::prelude::LEVEL_HALF
                .last()
                .copied()
                .unwrap_or(640.0),
        })
        .insert_resource(RendererCadence::new(Some(config.fps)));

    // Minimal protocol/settings/scene-index resources required by generic
    // viewport adapters (`SamplingCoordinatorPlugin`, `DlssProviderPlugin`,
    // `camera`, etc.) even when product bridge/project services are absent.
    app.init_resource::<ViewportCommandInbox>()
        .init_resource::<ViewportTreeCommandInbox>()
        .init_resource::<ViewportEventOutbox>()
        .init_resource::<ViewportReadModelState>()
        .init_resource::<ViewerSettingsState>()
        .init_resource::<SceneAnchorIndex>();

    if !app.is_plugin_added::<SamplingCoordinatorPlugin>() {
        app.add_plugins(SamplingCoordinatorPlugin);
    }
    if !app.is_plugin_added::<DlssProviderPlugin>() {
        app.add_plugins(DlssProviderPlugin);
    }
    if !app.is_plugin_added::<SolariCapabilityPlugin>() {
        app.add_plugins(SolariCapabilityPlugin);
    }
    if !app.world().contains_resource::<DisplayToggles>() {
        app.init_resource::<DisplayToggles>();
    }
    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .preferred_fps = Some(config.fps);

    let physics_initially_active = std::env::var("BEVY_OPENUSD_PHYSICS")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "on"))
        .unwrap_or(false);
    app.insert_resource(PhysicsActive(physics_initially_active));

    app.init_resource::<Spawned>()
        .init_resource::<ReloadRequest>()
        .init_resource::<LoadRequest>()
        .init_resource::<SelectedPrim>()
        .init_resource::<SelectedTargets>()
        .init_resource::<SelectionOutlineState>()
        .init_resource::<crate::viewport::scene::visualization::SceneExtent>()
        .init_resource::<FlyTo>()
        .init_resource::<CameraMount>()
        .init_resource::<LoaderTuning>()
        .init_resource::<UsdStageTime>()
        .init_resource::<CameraBookmarks>()
        .insert_resource(StageInfo {
            path: config.asset_path.clone(),
            ..default()
        })
        .insert_resource(RequestedAsset {
            name: config.asset_path.clone(),
            root: config.asset_root.clone(),
        })
        .add_systems(Startup, spawn_camera_and_ground);

    if !config.skip_initial_stage_load {
        app.add_systems(Startup, load_stage);
    }

    app.add_systems(
        Update,
        (
            poll_scene_cache_revalidation
                .after(LiveStageSet::Presentation)
                .before(spawn_when_ready),
            spawn_when_ready
                .after(LiveStageSet::Presentation)
                .before(ViewportBridgeSet::ApplyCommands),
            fit_camera_once,
            handle_usd_hot_reload,
            apply_load_request,
            apply_fly_to,
            sync_selected_instance_identity.before(LiveStageSet::Reconcile),
            follow_mounted_camera,
            hide_meshes_on_startup,
        ),
    );
    animation::configure(app);

    let hide_meshes = std::env::var("BEVY_OPENUSD_HIDE_MESHES")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "on"))
        .unwrap_or(false);
    app.insert_resource(HideMeshesFlag(hide_meshes));
    let show_joint_gizmos = std::env::var("BEVY_OPENUSD_JOINT_GIZMOS")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "on"))
        .unwrap_or(false);
    app.insert_resource(ShowJointGizmosFlag(show_joint_gizmos));
}

/// Installs the render-world-dependent generic viewport plugins (outlines,
/// wireframe, extended skinning, clipping material, grid/axis/transform gizmos,
/// overlays, and Solari when enabled).
pub(crate) fn configure_generic_viewport_render(app: &mut App) {
    if app.world().contains_resource::<GenericViewportRenderConfigured>() {
        return;
    }
    app.init_resource::<GenericViewportRenderConfigured>();

    if !app.is_plugin_added::<bevy::pbr::wireframe::WireframePlugin>() {
        app.add_plugins(bevy::pbr::wireframe::WireframePlugin::default());
    }
    if !app.is_plugin_added::<usd_bevy::ExtendedSkinPlugin>() {
        app.add_plugins(usd_bevy::ExtendedSkinPlugin);
    }
    if !cfg!(feature = "solari")
        && std::env::var("USDHUB_GPU_POINT_INSTANCING").is_ok_and(|value| value == "1")
    {
        use usd_bevy::route::gpu_instancing::{UsdGpuInstancing, UsdGpuInstancingPlugin};
        app.insert_resource(UsdGpuInstancing {
            enabled: true,
            min_pixels: 0.0,
            shadows: true,
            max_chunk_instances: std::env::var("USDHUB_GPU_POINT_INSTANCING_CHUNK_CAP")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(UsdGpuInstancing::default().max_chunk_instances),
            ..default()
        });
        app.add_plugins(UsdGpuInstancingPlugin);
    }
    if !app.is_plugin_added::<OutlinePlugin>() {
        app.add_plugins(OutlinePlugin::JUMP_FLOOD);
    }

    #[cfg(feature = "solari")]
    if !app.is_plugin_added::<bevy::solari::realtime::SolariLightingPlugin>() {
        app.add_plugins(bevy::solari::prelude::SolariPlugins);
    }

    if !app.is_plugin_added::<GroundGridPlugin>() {
        app.add_plugins(GroundGridPlugin);
    }
    if !app.is_plugin_added::<AxisGizmoPlugin>() {
        app.add_plugins(AxisGizmoPlugin);
    }
    if !app.is_plugin_added::<TransformGizmoPlugin>() {
        app.add_plugins(TransformGizmoPlugin);
    }
    if !app.is_plugin_added::<MaterialPlugin<SectionClipMaterial>>() {
        app.add_plugins(MaterialPlugin::<SectionClipMaterial>::default());
    }
    if !app.is_plugin_added::<OverlaysPlugin>() {
        app.add_plugins(OverlaysPlugin);
    }

    app.init_resource::<GizmoAutoScale>()
        .add_systems(Update, auto_scale_gizmo_to_target)
        .add_systems(
            Update,
            sync_chase_camera.before(bevy_glacial::prelude::build_grid_meshes),
        )
        .add_systems(
            Update,
            sync_selection_outlines
                .after(ViewportBridgeSet::ApplyCommands)
                .after(sync_selected_renderable_projection),
        )
        .init_gizmo_group::<SkeletonGizmos>()
        .add_systems(Startup, setup_skeleton_gizmos_on_top);
}

/// Installs the full generic USD viewport (render + core) without Project,
/// Git, Turso, cache, or transport services.
pub(crate) fn configure_generic_viewport(app: &mut App, config: &GenericViewportConfig) {
    configure_generic_viewport_render(app);
    configure_generic_viewport_core(app, config);
}

/// Injects concrete product services (Project runtime, Semantic sync,
/// `ViewportBridgePlugin`, diagnostics, benchmarks, and optional WebRTC
/// transport) on top of the generic viewport.
pub(crate) fn configure_product_services(
    app: &mut App,
    launch_options: &LaunchOptions,
    asset_path: &str,
    is_s8: bool,
) {
    crate::viewport::stage::project::install(app);
    app.insert_resource(SemanticSyncState::with_config(
        SemanticConfig::for_nvidia_revit_connector(),
    ));
    if !app.is_plugin_added::<ViewportBridgePlugin>() {
        app.add_plugins(ViewportBridgePlugin);
    }

    if launch_options.transport == Some(ViewportTransport::WebRtc) {
        app.add_plugins(crate::viewport::transport::webrtc::WebRtcTransportPlugin);
        let semantic_sync_runtime = match TursoClientSyncRuntime::from_environment(
            app.world().resource::<RenderServerInterface>().shared(),
        ) {
            Ok(runtime) => runtime,
            Err(error) => {
                bevy::log::error!("[semantic-sync] runtime configuration failed: {error:#}");
                None
            }
        };
        app.insert_resource(SemanticSyncRuntimeResource(semantic_sync_runtime))
            .add_systems(
                PostUpdate,
                process_semantic_sync_requests.after(synchronize_live_stage),
            );
        let application_interface = app.world().resource::<RenderServerInterface>().shared();
        let (stream_frame_tx, stream_frame_rx) =
            std::sync::mpsc::sync_channel::<viewport_streaming::VideoFrame>(4);
        let frame_metrics = viewport_streaming::FrameTransportMetrics::default();
        if launch_options.headless {
            app.add_plugins(crate::viewport::transport::FrameCapturePlugin {
                sender: stream_frame_tx,
                metrics: frame_metrics.clone(),
            });
        }
        let stage_display_name = std::path::Path::new(asset_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("remote-stage")
            .to_owned();
        let config = viewport_streaming::StreamingConfig {
            stage_display_name,
            width: launch_options.width,
            height: launch_options.height,
            fps: launch_options.fps,
            codec: launch_options.codec,
            auto_codec: launch_options.auto_codec,
            ..Default::default()
        };
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let (session_tx, session_rx) = tokio::sync::mpsc::channel(32);
                let session = viewport_streaming::WebRtcSessionManager::new(
                    config.clone(),
                    stream_frame_rx,
                    application_interface,
                    frame_metrics,
                );

                tokio::spawn(async move {
                    if let Err(error) = session.run(session_rx).await {
                        bevy::log::error!("[viewport-session] session manager failed: {error:?}");
                    }
                });

                let _ = viewport_streaming::run_signaling_server(config, session_tx).await;
            });
        });
    }

    // Performance and incident diagnostics
    app.init_resource::<crate::viewport::diagnostics::performance::RendererCounters>()
        .add_systems(
            First,
            crate::viewport::diagnostics::performance::start_frame_timing_system,
        )
        .add_systems(
            Last,
            crate::viewport::diagnostics::performance::collect_renderer_counters_system,
        );

    let benchmark_scenario = launch_options
        .benchmark_scenario
        .as_deref()
        .and_then(crate::viewport::diagnostics::performance::BenchmarkScenarioId::from_code);

    if launch_options.benchmark {
        app.add_plugins(
            crate::viewport::diagnostics::performance::ScenarioDriverPlugin {
                scenario_id: benchmark_scenario,
            },
        )
        .add_plugins(
            crate::viewport::diagnostics::performance::BenchmarkRunnerPlugin {
                config: crate::viewport::diagnostics::performance::BenchmarkLaunchConfig {
                    scenario: benchmark_scenario,
                    renderer_matrix: launch_options.benchmark_renderer_matrix,
                    mesh_profile: launch_options.benchmark_mesh_profile,
                    warmup_frames: launch_options.benchmark_warmup_frames,
                    target_frames: launch_options.benchmark_frames,
                    output_path: launch_options
                        .benchmark_output
                        .clone()
                        .map(std::path::PathBuf::from)
                        .or_else(|| {
                            launch_options
                                .benchmark_renderer_matrix
                                .then(|| "target/m3-c6-renderer-matrix.json".into())
                        })
                        .or_else(|| {
                            launch_options
                                .benchmark_mesh_profile
                                .then(|| "target/m4-geometry-profile.json".into())
                        }),
                    label: launch_options.benchmark_label.clone(),
                    width: launch_options.width,
                    height: launch_options.height,
                    requested_fps: launch_options.fps as f64,
                    asset_path: if is_s8 {
                        None
                    } else {
                        launch_options.asset_argument.clone()
                    },
                    client_ready_file: launch_options
                        .benchmark_client_ready_file
                        .clone()
                        .map(std::path::PathBuf::from),
                    measurement_start_file: launch_options
                        .benchmark_measurement_start_file
                        .clone()
                        .map(std::path::PathBuf::from),
                    measurement_idle_file: launch_options
                        .benchmark_measurement_idle_file
                        .clone()
                        .map(std::path::PathBuf::from),
                    measurement_complete_file: launch_options
                        .benchmark_measurement_complete_file
                        .clone()
                        .map(std::path::PathBuf::from),
                },
            },
        );
    }

    let _ = DefaultSun;
}

/// Installs the complete shared application plugin graph (generic viewport +
/// product services) into `app`.
pub(crate) fn configure_shared_application(
    app: &mut App,
    launch_options: &LaunchOptions,
    asset_path: &str,
    asset_root: PathBuf,
    is_s8: bool,
) {
    let generic_config = GenericViewportConfig {
        asset_path: if is_s8 {
            String::new()
        } else {
            asset_path.to_owned()
        },
        asset_root,
        fps: launch_options.fps,
        benchmark_mesh_profile: launch_options.benchmark_mesh_profile,
        skip_initial_stage_load: is_s8,
    };
    configure_generic_viewport(app, &generic_config);
    configure_product_services(app, launch_options, asset_path, is_s8);
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/app/composition_tests.rs"]
mod tests;
