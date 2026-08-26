use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use bevy_frost::prelude::AccentColor;
use bevy_glacial::prelude::{
    AxisGizmoPlugin, GizmoAutoScale, GroundGrid, GroundGridPlugin, TransformGizmoPlugin,
    auto_scale_gizmo_to_target,
};
use bevy_mod_outline::OutlinePlugin;
use usd_bevy::{LiveStagePlugin, LiveStageSet, UsdPlugin};

use super::{cadence, headless, offscreen_resize, scene, sync};
use crate::project::semantic_store::sync::TursoClientSyncRuntime;
use crate::viewport::animation::{UsdStageTime, tick_stage_time};
use crate::viewport::api::{RenderServerInterface, ViewportBridgePlugin, ViewportBridgeSet};
use crate::viewport::camera::{
    ArcballCameraPlugin, CameraBookmarks, CameraMount, FlyTo, apply_fly_to, fit_camera_once,
    follow_mounted_camera, sync_chase_camera,
};
use crate::viewport::input::{ViewportNavigationInput, keyboard::ViewerKeyboardPlugin};
use crate::viewport::physics::{PhysicsActive, RapierPhysicsPlugin};
use crate::viewport::rendering::sampling::{
    DlssProviderPlugin, SamplingCoordinatorPlugin, configure_dlss,
};
use crate::viewport::scene::visualization::{DisplayToggles, OverlaysPlugin};
use crate::viewport::scene::{
    HideMeshesFlag, SectionClipMaterial, SelectedPrim, SelectedTargets, SelectionOutlineState,
    ShowJointGizmosFlag, SkeletonGizmos, SolariCapabilityPlugin, hide_meshes_on_startup,
    setup_skeleton_gizmos_on_top, sync_selected_instance_identity, sync_selection_outlines,
};
use crate::viewport::semantic::synchronize_live_stage;
use crate::viewport::session::{
    LoadRequest, LoaderTuning, ReloadRequest, RequestedAsset, Spawned, StageInfo,
    apply_load_request, handle_usd_hot_reload, load_stage, spawn_when_ready,
};
use crate::viewport::transport::{ViewportTransport, parse_launch_options};
use crate::viewport::ui_frost::ViewerUiPlugin;

use scene::{open_default_panel, resolve_requested_asset, spawn_camera_and_ground};
use sync::{SemanticSyncRuntimeResource, process_semantic_sync_requests};

pub(crate) fn run() {
    let launch_options = match parse_launch_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("usdview: {error}");
            std::process::exit(2);
        }
    };
    let (asset_path, asset_root) = resolve_requested_asset(launch_options.asset_argument.clone());

    let mut app = App::new();
    configure_dlss(&mut app);

    if launch_options.headless {
        app.add_plugins(
            DefaultPlugins
                .build()
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .set(bevy::asset::AssetPlugin {
                    file_path: asset_root.to_string_lossy().into_owned(),
                    ..Default::default()
                })
                .set(bevy::log::LogPlugin {
                    custom_layer:
                        crate::viewport::diagnostics::log_capture::loader_log_custom_layer,
                    ..Default::default()
                }),
        );
    } else {
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("usdview — {asset_path}"),
                        resolution: (1400u32, 900u32).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::asset::AssetPlugin {
                    file_path: asset_root.to_string_lossy().into_owned(),
                    ..Default::default()
                })
                .set(bevy::log::LogPlugin {
                    custom_layer:
                        crate::viewport::diagnostics::log_capture::loader_log_custom_layer,
                    ..Default::default()
                }),
        );
    }

    app.add_plugins(EguiPlugin::default());
    if launch_options.headless {
        headless::configure_headless_egui(&mut app);
    }
    app.add_plugins(bevy::pbr::wireframe::WireframePlugin::default())
        .add_plugins(UsdPlugin)
        .add_plugins(OutlinePlugin::JUMP_FLOOD);

    #[cfg(feature = "solari")]
    app.add_plugins(bevy::solari::prelude::SolariPlugins);

    if launch_options.benchmark_mesh_profile {
        let mut profile = app.world_mut().resource_mut::<usd_bevy::GeometryProfile>();
        profile.enabled = true;
        profile.top_n = 128;
    }

    app.add_plugins(LiveStagePlugin)
        .add_plugins(RapierPhysicsPlugin)
        .add_plugins(ArcballCameraPlugin)
        .add_plugins(GroundGridPlugin)
        .add_plugins(AxisGizmoPlugin)
        .add_plugins(TransformGizmoPlugin)
        .add_plugins(MaterialPlugin::<SectionClipMaterial>::default())
        .init_resource::<GizmoAutoScale>()
        .add_systems(Update, auto_scale_gizmo_to_target)
        .insert_resource(ClearColor(Color::srgb(0.06, 0.08, 0.12)))
        .insert_resource(GroundGrid {
            visible: true,
            color: Color::srgba(0.30, 0.38, 0.50, 0.42),
            ground_y: None,
            coverage_radius: bevy_glacial::prelude::LEVEL_HALF
                .last()
                .copied()
                .unwrap_or(640.0),
        })
        .insert_resource(AccentColor(bevy_egui::egui::Color32::from_rgb(
            0x4A, 0x90, 0xE2,
        )))
        .insert_resource(cadence::RendererCadence::new(Some(launch_options.fps)));

    if launch_options.headless {
        app.add_plugins(headless::HeadlessRenderPlugin {
            width: launch_options.width,
            height: launch_options.height,
        })
        .add_systems(
            Update,
            offscreen_resize::apply_stream_configuration.before(ViewportBridgeSet::ApplyCommands),
        )
        .insert_resource(ViewportNavigationInput::with_viewport_size(
            launch_options.width,
            launch_options.height,
        ));
    }

    app.add_plugins(ViewportBridgePlugin)
        .add_plugins(SamplingCoordinatorPlugin)
        .add_plugins(DlssProviderPlugin)
        .add_plugins(SolariCapabilityPlugin)
        .add_plugins(OverlaysPlugin);
    app.world_mut()
        .resource_mut::<DisplayToggles>()
        .renderer
        .preferred_fps = Some(launch_options.fps);

    if !launch_options.headless {
        app.add_plugins(ViewerKeyboardPlugin)
            .add_plugins(ViewerUiPlugin)
            .add_systems(Startup, open_default_panel);
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
        let stage_display_name = std::path::Path::new(&asset_path)
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

    // Keep the public play/pause state independent from the transport. The
    // viewport-owned adapter consumes current `usd_bevy` markers and builds
    // the Rapier world through the pure `usd_rapier` crate.
    let physics_initially_active = std::env::var("BEVY_OPENUSD_PHYSICS")
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "on"))
        .unwrap_or(false);
    app.insert_resource(PhysicsActive(physics_initially_active));

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

    let is_s8 = benchmark_scenario
        == Some(
            crate::viewport::diagnostics::performance::BenchmarkScenarioId::S8NativeNoLiveStage,
        );

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

    app.init_resource::<Spawned>()
        .init_resource::<ReloadRequest>()
        .init_resource::<LoadRequest>()
        .init_resource::<SelectedPrim>()
        .init_resource::<SelectedTargets>()
        .init_resource::<SelectionOutlineState>()
        .init_resource::<FlyTo>()
        .init_resource::<CameraMount>()
        .init_resource::<LoaderTuning>()
        .init_resource::<UsdStageTime>()
        .init_resource::<CameraBookmarks>()
        .insert_resource(StageInfo {
            path: if is_s8 {
                String::new()
            } else {
                asset_path.clone()
            },
            ..default()
        })
        .insert_resource(RequestedAsset {
            name: if is_s8 { String::new() } else { asset_path },
            root: asset_root.clone(),
        })
        .add_systems(Startup, spawn_camera_and_ground);

    if !is_s8 {
        app.add_systems(Startup, load_stage);
    }

    app.add_systems(
        Update,
        (
            spawn_when_ready,
            fit_camera_once,
            handle_usd_hot_reload,
            apply_load_request,
            apply_fly_to,
            sync_selected_instance_identity.before(LiveStageSet::Reconcile),
            follow_mounted_camera,
            tick_stage_time,
            hide_meshes_on_startup,
        ),
    )
    .add_systems(
        Update,
        sync_chase_camera.before(bevy_glacial::prelude::build_grid_meshes),
    )
    .add_systems(
        Update,
        sync_selection_outlines.after(ViewportBridgeSet::ApplyCommands),
    );
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

    // Skeleton bones render as part of their own gizmo group with
    // `depth_bias = -1.0` so they always draw on top of geometry —
    // otherwise the rig is hidden inside the skin and the user has
    // no way to verify the joint hierarchy is alive.
    app.init_gizmo_group::<SkeletonGizmos>()
        .add_systems(Startup, setup_skeleton_gizmos_on_top);

    if launch_options.headless {
        app.set_runner(cadence::run_headless);
    }

    app.run();
}
