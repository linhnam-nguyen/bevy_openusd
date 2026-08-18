//! `bevy_openusd` viewer — primary dogfood binary.
//!
//! Loads a USD file, projects it into a Bevy Scene, and shows the result in
//! a VS-Code-style UI (left activity bar + floating panels). Used
//! throughout plugin development: each milestone gets dropped into this
//! viewer so we can eyeball the projection.
//!
//!   cargo run -- --headless --webrtc path/to/robot.usda
//!
//! Mouse: L+R drag orbit · Middle drag pan · Scroll zoom.
//! Keyboard: T I O ? toggle panels · G X P toggle overlays.

pub(crate) mod headless;
mod offscreen_resize;

use std::path::PathBuf;

use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use headless::HeadlessRenderPlugin;
use usd_bevy::{LiveStagePlugin, UsdPlugin};

use crate::viewport::animation::{UsdStageTime, tick_stage_time};
use crate::viewport::api::{RenderServerInterface, ViewportBridgePlugin};
use crate::viewport::camera::{
    ArcballCamera, ArcballCameraPlugin, CameraBookmarks, CameraMount, FlyTo, apply_fly_to,
    fit_camera_once, follow_mounted_camera, sync_chase_camera,
};
use crate::viewport::input::{ViewportNavigationInput, keyboard::ViewerKeyboardPlugin};
use crate::viewport::physics::{PhysicsActive, RapierPhysicsPlugin};
use crate::viewport::scene::visualization::OverlaysPlugin;
use crate::viewport::scene::{
    HideMeshesFlag, SelectedPrim, ShowJointGizmosFlag, SkeletonGizmos,
    draw_selected_prim_highlight, hide_meshes_on_startup, setup_skeleton_gizmos_on_top,
};
use crate::viewport::session::{
    LoadRequest, LoaderTuning, ReloadRequest, RequestedAsset, Spawned, StageInfo,
    apply_load_request, handle_usd_hot_reload, load_stage, spawn_when_ready,
};
use crate::viewport::transport::{ViewportTransport, parse_launch_options};
use crate::viewport::ui_frost::{RIB_TREE, RIBBON_LEFT, ViewerUiPlugin};
use bevy_glacial::prelude::{
    AxisGizmo, AxisGizmoPlugin, ChaseCamera, GroundGrid, GroundGridPlugin,
};
use viewport_protocol::{SemanticSyncPhase, SemanticSyncStatus};

use crate::project::semantic_store::sync::{
    TursoClientSyncProvisionRequest, TursoClientSyncRuntime, TursoClientSyncRuntimeCommand,
};
use crate::viewport::semantic::{SemanticSyncState, synchronize_live_stage};

/// Tag on the viewer's fallback `DirectionalLight`.
#[derive(Component)]
struct DefaultSun;

#[derive(Resource, Default)]
struct SemanticSyncRuntimeResource(Option<TursoClientSyncRuntime>);

pub(crate) fn run() {
    let launch_options = match parse_launch_options(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("usdview: {error}");
            std::process::exit(2);
        }
    };
    let (asset_path, asset_root) = resolve_requested_asset(launch_options.asset_argument);

    let mut app = App::new();

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
                })
                .add(bevy::app::ScheduleRunnerPlugin::run_loop(
                    std::time::Duration::from_secs_f64(1.0 / launch_options.fps as f64),
                )),
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

    app.add_plugins(EguiPlugin::default())
        .add_plugins(bevy::pbr::wireframe::WireframePlugin::default())
        .add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .add_plugins(RapierPhysicsPlugin)
        .add_plugins(ArcballCameraPlugin)
        .add_plugins(GroundGridPlugin)
        .add_plugins(AxisGizmoPlugin)
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
        .insert_resource(bevy_frost::prelude::AccentColor(
            bevy_egui::egui::Color32::from_rgb(0x4A, 0x90, 0xE2),
        ));

    if launch_options.headless {
        app.add_plugins(HeadlessRenderPlugin {
            width: launch_options.width,
            height: launch_options.height,
        })
        .add_systems(Update, offscreen_resize::apply_stream_configuration)
        .insert_resource(ViewportNavigationInput::with_viewport_size(
            launch_options.width,
            launch_options.height,
        ));
    }

    app.add_plugins(ViewportBridgePlugin)
        .add_plugins(OverlaysPlugin);

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
        if launch_options.headless {
            app.add_plugins(crate::viewport::transport::FrameCapturePlugin {
                sender: stream_frame_tx,
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

    app.init_resource::<Spawned>()
        .init_resource::<ReloadRequest>()
        .init_resource::<LoadRequest>()
        .init_resource::<SelectedPrim>()
        .init_resource::<FlyTo>()
        .init_resource::<CameraMount>()
        .init_resource::<LoaderTuning>()
        .init_resource::<UsdStageTime>()
        .init_resource::<CameraBookmarks>()
        .insert_resource(StageInfo {
            path: asset_path.clone(),
            ..default()
        })
        .insert_resource(RequestedAsset {
            name: asset_path,
            root: asset_root.clone(),
        })
        .add_systems(Startup, (load_stage, spawn_camera_and_ground))
        .add_systems(
            Update,
            (
                spawn_when_ready,
                fit_camera_once,
                handle_usd_hot_reload,
                apply_load_request,
                apply_fly_to,
                draw_selected_prim_highlight,
                follow_mounted_camera,
                tick_stage_time,
                hide_meshes_on_startup,
            ),
        )
        .add_systems(
            Update,
            sync_chase_camera.before(bevy_glacial::prelude::build_grid_meshes),
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

    app.run();
}

fn process_semantic_sync_requests(
    interface: Res<RenderServerInterface>,
    runtime: Res<SemanticSyncRuntimeResource>,
    semantic: Res<SemanticSyncState>,
) {
    let application_interface = interface.shared();
    while let Some(request) = application_interface.pop_semantic_sync_request() {
        let request_id = request.request_id;
        let session_id = request.session_id;
        let client_name = request.client_name;
        let authorization = request.authorization;

        let (command, is_close) = match request.kind {
            viewport_streaming::SemanticSyncRequestKind::Client(operation) => {
                let is_close = operation == viewport_protocol::SemanticSyncOperation::Close;

                let command = match operation {
                    viewport_protocol::SemanticSyncOperation::Provision => {
                        TursoClientSyncRuntimeCommand::Provision(TursoClientSyncProvisionRequest {
                            session_id: session_id.clone(),
                            client_name,
                            authorization,
                        })
                    }
                    viewport_protocol::SemanticSyncOperation::Connect => {
                        TursoClientSyncRuntimeCommand::Connect(session_id.clone())
                    }
                    viewport_protocol::SemanticSyncOperation::PushSnapshot => {
                        let Some(snapshot) = semantic.snapshot().cloned() else {
                            let _ = application_interface.publish_semantic_sync_status(
                                session_id.clone(),
                                SemanticSyncStatus::phase(
                                    SemanticSyncPhase::Failed,
                                    Some("snapshot_unavailable".to_owned()),
                                ),
                            );
                            continue;
                        };

                        TursoClientSyncRuntimeCommand::PushSnapshot {
                            session_id: session_id.clone(),
                            snapshot,
                        }
                    }
                    viewport_protocol::SemanticSyncOperation::PullProjection => {
                        TursoClientSyncRuntimeCommand::PullProjection(session_id.clone())
                    }
                    viewport_protocol::SemanticSyncOperation::Close => {
                        TursoClientSyncRuntimeCommand::Close(session_id.clone())
                    }
                };

                (command, is_close)
            }

            viewport_streaming::SemanticSyncRequestKind::AuthorizationChanged => (
                TursoClientSyncRuntimeCommand::UpdateAuthorization {
                    session_id: session_id.clone(),
                    authorization,
                },
                false,
            ),
        };

        let Some(runtime) = runtime.0.as_ref() else {
            let status = if is_close {
                SemanticSyncStatus::phase(SemanticSyncPhase::Closed, None)
            } else {
                SemanticSyncStatus::phase(
                    SemanticSyncPhase::Failed,
                    Some("runtime_unavailable".to_owned()),
                )
            };
            let _ = application_interface.publish_semantic_sync_status(session_id, status);
            continue;
        };
        if let Err(error) = runtime.submit(command) {
            bevy::log::warn!(
                "[semantic-sync] request {} could not reach worker: {error:#}",
                request_id
            );
            let _ = application_interface.publish_semantic_sync_status(
                session_id,
                SemanticSyncStatus::phase(
                    SemanticSyncPhase::Failed,
                    Some("worker_unavailable".to_owned()),
                ),
            );
        }
    }
}

/// Opens the prim tree on startup to give the viewer an immediate focal panel.
fn open_default_panel(mut ribbon: ResMut<bevy_frost::RibbonOpen>) {
    ribbon.toggle(RIBBON_LEFT, RIB_TREE);
}

/// Resolves the CLI stage argument into a stage path and its asset root.
///
/// - `cargo run` with no argument loads the self-contained spinner sample.
/// - `cargo run -- path/to/file.usda` roots relative USD references at the
///   file's parent directory.
fn resolve_requested_asset(arg: Option<String>) -> (String, PathBuf) {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    match arg {
        None => (
            "animated_spinner.usda".to_string(),
            workspace_root.join("assets"),
        ),
        Some(raw) => {
            let path = PathBuf::from(&raw);
            let abs = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| workspace_root.clone())
                    .join(path)
            };
            let file = abs
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| raw.clone());
            let dir = abs
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| workspace_root.clone());
            (file, dir)
        }
    }
}

/// Spawns the default arcball camera, fallback light, and ground-support entities.
fn spawn_camera_and_ground(mut commands: Commands) {
    use bevy::camera::{Hdr, PerspectiveProjection, Projection};
    use bevy::core_pipeline::tonemapping::Tonemapping;

    // Arcball camera targeting origin; focus/distance get tuned once the
    // stage lands (stretch goal: fit-to-bounds).
    //
    // **HDR + ACES tone mapping + bloom** are essential for PBR
    // materials to look right. In Bevy 0.18 HDR is a `Hdr` marker
    // component (was a `hdr: bool` field on `Camera` previously), and
    // bloom lives in `bevy::post_process`. Without HDR, emissive
    // textures and metallic specular highlights clamp to LDR and look
    // chalky; ACES tone mapping (the curve usdview / Quick Look apply)
    // restores the filmic falloff. Bloom adds the soft edge around
    // light sources and bright reflections.
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            // The default 10cm near plane made close inspection feel like
            // "zoom stopped early" on small robotics/USD details. Keep it
            // tight so the arcball can dolly into millimetre-scale features.
            near: 0.0001,
            ..default()
        }),
        Hdr,
        // AgX is the modern filmic curve Blender / Krita default to.
        // ACES is more contrasty + clips highlights harder; with a
        // single 50k-lux sun ACES turned the teapot into a pure-white
        // blob. AgX rolls highlights gently, reproduces albedo more
        // faithfully, and tolerates wider exposure ranges before
        // clipping.
        Tonemapping::AgX,
        // Bloom is OFF by default — turn it on with `Bloom::default()`
        // once the user wants it. With strong direct lighting + HDR +
        // ACES, the default bloom radius blew the highlights out into
        // halos that overlapped the entire silhouette.
        // (Re-enable: add `Bloom::default()` here.)
        Transform::from_xyz(3.0, 2.5, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
        ArcballCamera {
            focus: Vec3::new(0.0, 0.4, 0.0),
            distance: 4.0,
            ..default()
        },
        // bevy_glacial's GroundGridPlugin queries `&ChaseCamera` to
        // pick the LOD level. Tag our arcball-camera entity so the
        // grid follows our actual viewport without us having to
        // adopt their full ChaseCameraPlugin (we keep our orbit
        // controls). `sync_chase_camera` mirrors focus/distance/yaw
        // every frame.
        ChaseCamera::default(),
    ));
    // World-origin axis triad — replaces our hand-rolled
    // `draw_axes` overlay system.
    commands.spawn((
        Name::new("WorldAxes"),
        Transform::default(),
        AxisGizmo::default(),
    ));
    // Indoor-overcast lux. With HDR + AgX a single 5k-lux sun reads
    // closer to a quick-look studio render than 50k did — an order of
    // magnitude lower because HDR + AgX preserve dynamic range above
    // 1.0 instead of clipping; we don't need to push raw lux.
    // Ambient stays modest so PBR keeps its contrast.
    commands.spawn((
        DirectionalLight {
            illuminance: 5_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 6.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
        DefaultSun,
    ));
    commands.insert_resource(bevy::light::GlobalAmbientLight {
        brightness: 200.0,
        ..default()
    });
    // Ground plate intentionally gone — the WorldGrid overlay provides the
    // reference plane now, sized and faded to match the scene extent.
}
