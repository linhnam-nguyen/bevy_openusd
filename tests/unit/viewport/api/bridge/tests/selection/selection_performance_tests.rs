use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use usd_bevy::{UsdLocalExtent, UsdPrimRef};
use viewport_protocol::{SceneAnchor, SelectionReadModel, ViewportCommand};

use super::support::command_test_app;
use crate::api::bridge::commands::apply_viewport_commands;
use crate::api::bridge::tree::apply_tree_commands;
use crate::api::{ViewportCommandInbox, ViewportEventOutbox, ViewportTreeCommandInbox};
use crate::viewport::navigation::camera::{ArcballCamera, FlyTo};
use crate::viewport::rendering::scene::SceneAnchorIndex;
use crate::viewport::rendering::scene::{
    SelectedRenderableProjection, SelectedTargets, sync_selected_renderable_projection,
};

const HEAVY_PATH: &str = "/World/HeavyGroup";
const LIGHT_PATH: &str = "/World/LightMesh";
const HEAVY_MESH_COUNT: usize = 10_000;

fn anchor(path: &str) -> SceneAnchor {
    SceneAnchor::active_session(path)
}

#[test]
fn focus_keeps_canonical_nested_bounds_after_visual_unload() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("root.usda");
    let source = br#"#usda 1.0
(
    upAxis = "Z"
)
def Xform "Assembly" {
    def Cube "Lower" {}
    def Xform "Nested" (payload = @./nested.usda@</Nested>) {}
}
"#;
    std::fs::write(&root, source)?;
    std::fs::write(
        dir.path().join("nested.usda"),
        br#"#usda 1.0
def Xform "Nested" {
    def Cube "Upper" {
        double3 xformOp:translate = (0, 0, 12)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }
}
"#,
    )?;
    let live = usd_bevy::LiveStage::open_document(&root)?;
    assert!(
        live.visual_loadable_paths_if_backed()?.is_some(),
        "file-backed source must expose a distinct visual ownership context"
    );
    live.apply_visual_load_batch(&[], &["/Assembly/Nested".to_owned()])?;
    assert!(
        !live
            .visual_loaded_paths()?
            .contains(&"/Assembly/Nested".to_owned())
    );

    let target = anchor("/Assembly");
    let mut app = focus_test_app(&target, Vec3::ZERO);
    app.world_mut().insert_non_send(live);
    let focus = request_focus(&mut app, target.clone());
    assert!((focus.0 - Vec3::Y * 6.0).length() < 0.001, "{focus:?}");
    assert!(
        focus.1 > 15.0 && focus.1 < 25.0,
        "canonical nested bounds must frame the unloaded tall assembly, got {focus:?}"
    );
    assert_eq!(std::fs::read(root)?, source);

    // Introduce a genuine canonical composition error after the file-backed
    // context has been published. Its visual snapshot remains incomplete.
    let live = app.world().get_non_send::<usd_bevy::LiveStage>().unwrap();
    live.stage.prim("/Assembly")?.set_metadata(
        "references",
        openusd::vt::Value::ReferenceListOp(openusd::sdf::ReferenceListOp::prepended([
            openusd::sdf::Reference {
                asset_path: "./missing.usda".to_owned(),
                prim_path: openusd::sdf::path("/Missing")?,
                ..Default::default()
            },
        ])),
    )?;
    assert!(live.visual_loadable_paths_if_backed().is_err());
    let request =
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::FocusTarget {
                target,
                mode: viewport_protocol::FocusMode::FrameTarget,
            });
    app.update();
    let mut outbox = app.world_mut().resource_mut::<ViewportEventOutbox>();
    let events: Vec<_> = std::iter::from_fn(|| outbox.pop())
        .filter(|event| event.request_id.as_deref() == Some(request.as_str()))
        .collect();
    assert!(matches!(
        events.as_slice(),
        [viewport_protocol::ViewportEventEnvelope {
            event: viewport_protocol::ViewportEvent::CommandRejected { reason, .. },
            ..
        }] if reason == "cannot verify visual payload owners for focus"
    ));
    let fly_to = app.world().resource::<FlyTo>();
    assert_eq!((fly_to.target_focus, fly_to.target_distance), focus);
    Ok(())
}

#[test]
fn focus_uses_resident_geometry_for_occurrences_and_disconnected_targets() {
    let mut occurrence = anchor("/Assembly");
    occurrence.instance_context = Some("occurrence".to_owned());
    let mut app = focus_test_app(&occurrence, Vec3::X * 20.0);
    let stage = openusd::usd::Stage::builder()
        .in_memory("focus.usda")
        .unwrap();
    let live = usd_bevy::LiveStage::new(stage);
    assert!(
        live.visual_loadable_paths_if_backed().unwrap().is_none(),
        "documentless scene must retain the existing resident-entity fit"
    );
    app.world_mut().insert_non_send(live);
    let (occ_focus, occ_dist) = request_focus(&mut app, occurrence);
    assert_eq!(occ_focus, Vec3::X * 20.0);
    assert!(occ_dist.is_finite() && occ_dist > 3.5);

    let target = anchor("/Unconnected");
    let mut app = focus_test_app(&target, Vec3::Z * 5.0);
    let (target_focus, target_dist) = request_focus(&mut app, target);
    assert_eq!(target_focus, Vec3::Z * 5.0);
    assert!((target_dist - occ_dist).abs() < 1.0e-5);
}

#[test]
fn api_focus_frames_vertical_box_at_narrow_fov_and_fly_to_contains_all_corners() {
    use bevy::camera::{CameraProjection, PerspectiveProjection, Projection, RenderTargetInfo};

    let target = anchor("/TallColumn");
    let mut app = command_test_app();
    app.init_resource::<Time>().add_systems(
        Update,
        (
            apply_tree_commands.after(apply_viewport_commands),
            crate::viewport::navigation::camera::apply_fly_to,
        ),
    );

    let perspective = PerspectiveProjection {
        fov: 30.0_f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        ..default()
    };
    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(1600, 900),
        scale_factor: 1.0,
    });
    let cam_entity = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            camera,
            Projection::Perspective(perspective.clone()),
            Transform::default(),
            ArcballCamera {
                yaw: 0.0,
                elevation: 0.0,
                distance: 10.0,
                zoom_target: 10.0,
                ..Default::default()
            },
        ))
        .id();
    let entity = app
        .world_mut()
        .spawn((
            Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
            UsdLocalExtent {
                min: [-0.05, -1.0, -0.05],
                max: [0.05, 1.0, 0.05],
            },
            Visibility::Visible,
        ))
        .id();
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(target.clone(), entity);

    let (focus, distance) = request_focus(&mut app, target);
    assert_eq!(focus, Vec3::new(0.0, 2.0, 0.0));
    // Legacy `max_dim * 1.6 = 3.2` clipped a ±1 vertical box at FOV 30°;
    // frustum-aware framing must pull back beyond 1.1 / tan(15°) ≈ 4.105.
    assert!(
        distance > 4.1,
        "API focus must respect 30-deg FOV, got distance {distance}"
    );

    // Complete the fly-to tween and verify all 8 corners project inside NDC [-1, 1].
    app.world_mut().resource_mut::<FlyTo>().duration = 0.0;
    app.update();
    let tr = *app
        .world()
        .get::<Transform>(cam_entity)
        .expect("camera transform exists");
    let view_from_world = tr.compute_affine().inverse();
    let clip_from_view = perspective.get_clip_from_view();
    let min = Vec3::new(-0.05, 1.0, -0.05);
    let max = Vec3::new(0.05, 3.0, 0.05);
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        let view = view_from_world.transform_point3(corner);
        let clip = clip_from_view * view.extend(1.0);
        assert!(clip.w > 0.0);
        let ndc = clip.truncate() / clip.w;
        assert!(
            ndc.x.abs() <= 1.0 + 1.0e-4 && ndc.y.abs() <= 1.0 + 1.0e-4,
            "focused corner {corner:?} outside NDC [-1, 1]: {ndc:?}"
        );
    }
}

#[test]
fn api_focus_rejects_missing_projection_without_mutating_mount_or_fly_to() {
    let target = anchor("/Part");
    let mut app = command_test_app();
    app.add_systems(Update, apply_tree_commands.after(apply_viewport_commands));
    // Spawn camera WITHOUT a Projection component.
    app.world_mut().spawn(ArcballCamera::default());
    let entity = app
        .world_mut()
        .spawn((
            Transform::default(),
            UsdLocalExtent {
                min: [-1.0; 3],
                max: [1.0; 3],
            },
            Visibility::Visible,
        ))
        .id();
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(target.clone(), entity);
    *app.world_mut()
        .resource_mut::<crate::viewport::navigation::camera::CameraMount>() =
        crate::viewport::navigation::camera::CameraMount::Mounted {
            prim_path: "/World/ShotCam".to_owned(),
        };

    let request_id =
        app.world_mut()
            .resource_mut::<ViewportCommandInbox>()
            .send(ViewportCommand::FocusTarget {
                target,
                mode: viewport_protocol::FocusMode::FrameTarget,
            });
    app.update();

    let events: Vec<_> =
        std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
            .filter(|event| event.request_id.as_deref() == Some(request_id.as_str()))
            .collect();
    assert!(matches!(
        events.as_slice(),
        [viewport_protocol::ViewportEventEnvelope {
            event: viewport_protocol::ViewportEvent::CommandRejected { .. },
            ..
        }]
    ));
    assert!(matches!(
        app.world()
            .resource::<crate::viewport::navigation::camera::CameraMount>(),
        crate::viewport::navigation::camera::CameraMount::Mounted { prim_path }
            if prim_path == "/World/ShotCam"
    ));
    assert_eq!(app.world().resource::<FlyTo>().remaining, 0.0);
}

#[test]
fn api_focus_point_validates_context_and_never_mutates_mounted_camera_on_zero_viewport() {
    use bevy::camera::{PerspectiveProjection, Projection, RenderTargetInfo};

    let target = anchor("/TransformOnly");
    let mut app = command_test_app();
    app.add_systems(Update, apply_tree_commands.after(apply_viewport_commands));

    let mut zero_cam = Camera::default();
    zero_cam.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::ZERO,
        scale_factor: 1.0,
    });
    let cam_entity = app
        .world_mut()
        .spawn((
            ArcballCamera {
                distance: 8.5,
                zoom_target: 8.5,
                ..Default::default()
            },
            Projection::Perspective(PerspectiveProjection::default()),
            zero_cam,
        ))
        .id();
    let entity = app
        .world_mut()
        .spawn((Transform::from_xyz(3.0, 4.0, 5.0), Visibility::Visible))
        .id();
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(target.clone(), entity);
    *app.world_mut()
        .resource_mut::<crate::viewport::navigation::camera::CameraMount>() =
        crate::viewport::navigation::camera::CameraMount::Mounted {
            prim_path: "/World/ShotCam".to_owned(),
        };

    // With zero viewport, point focus must defer without mutating mount,
    // selection, FlyTo, or emitting CameraTransitionStarted.
    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::FocusTarget {
            target: target.clone(),
            mode: viewport_protocol::FocusMode::FrameTarget,
        });
    app.update();

    assert!(matches!(
        app.world()
            .resource::<crate::viewport::navigation::camera::CameraMount>(),
        crate::viewport::navigation::camera::CameraMount::Mounted { prim_path }
            if prim_path == "/World/ShotCam"
    ));
    assert_eq!(app.world().resource::<FlyTo>().remaining, 0.0);
    assert_eq!(app.world().resource::<SelectedTargets>().0.primary, None);
    let emitted: Vec<_> =
        std::iter::from_fn(|| app.world_mut().resource_mut::<ViewportEventOutbox>().pop())
            .collect();
    assert!(
        !emitted.iter().any(|e| matches!(
            e.event,
            viewport_protocol::ViewportEvent::CameraTransitionStarted { .. }
        )),
        "deferred point focus must not emit CameraTransitionStarted"
    );

    // Once viewport becomes valid, deferred point focus succeeds and preserves distance.
    app.world_mut()
        .get_mut::<Camera>(cam_entity)
        .expect("cam")
        .computed
        .target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(1280, 720),
        scale_factor: 1.0,
    });
    app.update();

    assert!(matches!(
        app.world()
            .resource::<crate::viewport::navigation::camera::CameraMount>(),
        crate::viewport::navigation::camera::CameraMount::Arcball
    ));
    let fly = app.world().resource::<FlyTo>();
    assert_eq!(fly.target_focus, Vec3::new(3.0, 4.0, 5.0));
    assert!((fly.target_distance - 8.5).abs() < 1.0e-5);
    assert!(fly.framing_intent.is_some());
}

fn focus_test_app(target: &SceneAnchor, position: Vec3) -> App {
    let mut app = command_test_app();
    app.add_systems(Update, apply_tree_commands.after(apply_viewport_commands));
    app.world_mut()
        .spawn((ArcballCamera::default(), Projection::default()));
    let entity = app
        .world_mut()
        .spawn((
            Transform::from_translation(position),
            UsdLocalExtent {
                min: [-1.0; 3],
                max: [1.0; 3],
            },
            Visibility::Visible,
        ))
        .id();
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entity(target.clone(), entity);
    app
}

fn request_focus(app: &mut App, target: SceneAnchor) -> (Vec3, f32) {
    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::FocusTarget {
            target: target.clone(),
            mode: viewport_protocol::FocusMode::FrameTarget,
        });
    app.update();
    assert_eq!(
        app.world().resource::<SelectedTargets>().0.primary,
        Some(target)
    );
    let fly_to = app.world().resource::<FlyTo>();
    (fly_to.target_focus, fly_to.target_distance)
}

#[test]
fn heavy_focus_is_canceled_by_light_selection_and_control_lane_progresses() {
    let mut app = command_test_app();
    app.add_systems(Update, apply_tree_commands.after(apply_viewport_commands));

    let heavy = app.world_mut().spawn(Visibility::Visible).id();
    let light = app.world_mut().spawn(Visibility::Visible).id();
    let heavy_anchor = anchor(HEAVY_PATH);
    let light_anchor = anchor(LIGHT_PATH);
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entities(vec![
            (heavy_anchor.clone(), heavy),
            (light_anchor.clone(), light),
        ]);

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::ReplaceSelection {
            targets: vec![heavy_anchor.clone()],
            primary: Some(heavy_anchor.clone()),
        },
    );
    app.update();
    assert_eq!(
        app.world().resource::<SelectedTargets>().0.primary,
        Some(heavy_anchor.clone())
    );

    app.world_mut()
        .resource_mut::<ViewportCommandInbox>()
        .send(ViewportCommand::FocusTarget {
            target: heavy_anchor.clone(),
            mode: viewport_protocol::FocusMode::FrameTarget,
        });
    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::ReplaceSelection {
            targets: vec![light_anchor.clone()],
            primary: Some(light_anchor.clone()),
        },
    );
    app.update();

    assert_eq!(
        app.world().resource::<SelectedTargets>().0.primary,
        Some(light_anchor.clone())
    );
    assert_eq!(
        app.world()
            .resource::<ViewportTreeCommandInbox>()
            .pending_focus_count(),
        0,
        "a focus captured against Heavy must not survive its replacement"
    );

    app.world_mut().resource_mut::<ViewportCommandInbox>().send(
        ViewportCommand::SetSubtreeVisibility {
            target: heavy_anchor,
            visible: false,
        },
    );
    app.update();
    assert_eq!(
        *app.world().get::<Visibility>(heavy).unwrap(),
        Visibility::Hidden
    );
}

#[test]
fn heavy_selection_releases_renderables_and_bounds_are_opt_in() {
    let mut app = projection_app();
    let heavy_anchor = anchor(HEAVY_PATH);
    let light_anchor = anchor(LIGHT_PATH);

    set_selection(&mut app, heavy_anchor.clone());
    app.update();
    let heavy_renderables = app
        .world()
        .resource::<SelectedRenderableProjection>()
        .renderables()
        .len();
    assert!(
        heavy_renderables < HEAVY_MESH_COUNT,
        "large-parent selection must yield bounded projection work per update"
    );
    let mut updates = 0;
    while app
        .world()
        .resource::<SelectedRenderableProjection>()
        .is_pending()
    {
        app.update();
        updates += 1;
        assert!(
            updates <= 160,
            "selection projection did not converge within the bounded cursor budget"
        );
    }
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .renderables()
            .len(),
        HEAVY_MESH_COUNT
    );
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .aggregate_bounds(),
        None,
        "plain selection must not derive geometry bounds"
    );

    app.world_mut()
        .resource_mut::<crate::api::ViewerSettingsState>()
        .set_section_box_enabled(true);
    app.update();
    while app
        .world()
        .resource::<SelectedRenderableProjection>()
        .is_pending()
    {
        app.update();
    }
    assert!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .aggregate_bounds()
            .is_some()
    );

    set_selection(&mut app, light_anchor);
    app.update();
    assert!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .renderables()
            .len()
            < HEAVY_MESH_COUNT
    );
    let mut release_updates = 0;
    while app
        .world()
        .resource::<SelectedRenderableProjection>()
        .is_pending()
    {
        app.update();
        release_updates += 1;
        assert!(
            release_updates <= 64,
            "selection release did not converge within the bounded removal budget"
        );
    }
    let projection = app.world().resource::<SelectedRenderableProjection>();
    assert_eq!(projection.renderables().len(), 1);
    assert!(projection.aggregate_bounds().is_some());

    app.world_mut()
        .resource_mut::<SelectedTargets>()
        .clear()
        .expect("clearing the synthetic selection must succeed");
    app.update();
    let mut clear_updates = 0;
    while app
        .world()
        .resource::<SelectedRenderableProjection>()
        .is_pending()
    {
        app.update();
        clear_updates += 1;
        assert!(
            clear_updates <= 16,
            "selection clear did not converge promptly"
        );
    }
    assert!(
        app.world()
            .resource::<SelectedTargets>()
            .0
            .targets
            .is_empty()
    );
    assert!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .renderables()
            .is_empty()
    );
    app.world_mut()
        .resource_mut::<crate::api::ViewerSettingsState>()
        .set_section_box_enabled(false);
    app.update();
    assert_eq!(
        app.world()
            .resource::<SelectedRenderableProjection>()
            .aggregate_bounds(),
        None
    );
}

fn set_selection(app: &mut App, target: SceneAnchor) {
    app.world_mut()
        .resource_mut::<SelectedTargets>()
        .replace(SelectionReadModel {
            targets: vec![target.clone()],
            primary: Some(target),
        })
        .expect("synthetic selection must satisfy the protocol invariant");
}

fn projection_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<SelectedTargets>()
        .init_resource::<crate::api::ViewerSettingsState>()
        .init_resource::<SelectedRenderableProjection>();

    let heavy = app.world_mut().spawn(UsdPrimRef::new(HEAVY_PATH)).id();
    let mut entries = vec![(anchor(HEAVY_PATH), heavy)];
    for index in 0..HEAVY_MESH_COUNT {
        app.world_mut().spawn((
            UsdPrimRef::new(format!("{HEAVY_PATH}/Mesh{index:04}")),
            Mesh3d(Handle::<Mesh>::default()),
            GlobalTransform::from(Transform::from_xyz(index as f32, 0.0, 0.0)),
            UsdLocalExtent {
                min: [-0.5, -0.5, -0.5],
                max: [0.5, 0.5, 0.5],
            },
            ChildOf(heavy),
        ));
    }
    let light = app
        .world_mut()
        .spawn((
            UsdPrimRef::new(LIGHT_PATH),
            Mesh3d(Handle::<Mesh>::default()),
            GlobalTransform::from(Transform::from_xyz(1.0, 0.0, 0.0)),
            UsdLocalExtent {
                min: [-0.5, -0.5, -0.5],
                max: [0.5, 0.5, 0.5],
            },
        ))
        .id();
    entries.push((anchor(LIGHT_PATH), light));
    *app.world_mut().resource_mut::<SceneAnchorIndex>() =
        SceneAnchorIndex::from_test_entities(entries);
    app.add_systems(Update, sync_selected_renderable_projection);
    app.update();
    app
}
