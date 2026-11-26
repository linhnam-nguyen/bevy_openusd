use bevy::camera::{
    CameraProjection, OrthographicProjection, PerspectiveProjection, Projection, ScalingMode,
};

use super::super::framing::{
    FramingOutcome, apply_framing_fit, frame_bounds_for_camera, frame_point_for_camera,
    framing_for_scene_extent, orthographic_scale_bounds, validated_orthographic_projection,
};
use super::*;

fn assert_all_corners_in_perspective_ndc(
    min: Vec3,
    max: Vec3,
    cam: &ArcballCamera,
    projection: &PerspectiveProjection,
    config: &CameraNavigationConfig,
) {
    let mut tr = Transform::default();
    apply_rig(cam, &mut tr);
    let mut proj = projection.clone();
    proj.near = adaptive_near(f64::from(cam.distance), config);
    let radius = f64::from((max - min).length()) * 0.5;
    proj.far = adaptive_far(
        f64::from(cam.distance),
        f64::from(cam.distance),
        radius,
        proj.near,
        config,
    );
    assert!(proj.near.is_finite() && proj.near > 0.0);
    assert!(proj.far.is_finite() && proj.far > proj.near);

    let view_from_world = tr.compute_affine().inverse();
    let clip_from_view = proj.get_clip_from_view();
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        );
        let view = view_from_world.transform_point3(corner);
        let depth = -view.z;
        assert!(
            depth > proj.near && depth < proj.far,
            "corner {corner:?} depth {depth} outside ({}, {})",
            proj.near,
            proj.far
        );
        let clip = clip_from_view * view.extend(1.0);
        assert!(
            clip.w > 0.0,
            "corner {corner:?} behind camera: w={}",
            clip.w
        );
        let ndc = clip.truncate() / clip.w;
        assert!(
            ndc.x.abs() <= 1.0 + 1.0e-4 && ndc.y.abs() <= 1.0 + 1.0e-4,
            "corner {corner:?} clipped outside NDC [-1, 1]: ndc={ndc:?}"
        );
    }
}

#[test]
fn tiny_scene_gets_a_near_plane_below_the_legacy_floor() {
    let config = CameraNavigationConfig::default();
    let near = adaptive_near(0.01, &config);

    assert!(near.is_finite());
    assert!(near > 0.0);
    assert!(near < 0.001);
}

#[test]
fn large_scene_gets_a_far_plane_beyond_the_legacy_limit() {
    let config = CameraNavigationConfig::default();
    let near = adaptive_near(100_000.0, &config);
    let far = adaptive_far(100_000.0, 100_000.0, 50_000.0, near, &config);

    assert!(near.is_finite());
    assert!(far.is_finite());
    assert!(far > near);
    assert!(far > 10_000.0);
    assert!(f64::from(far) >= 100_000.0 + 50_000.0);
}

#[test]
fn empty_scene_uses_finite_ordered_clipping_fallbacks() {
    let config = CameraNavigationConfig::default();
    let near = adaptive_near(config.empty_scene_distance, &config);
    let far = adaptive_far(
        config.empty_scene_distance,
        config.empty_scene_distance,
        config.empty_scene_radius,
        near,
        &config,
    );

    assert!(near.is_finite() && near > 0.0);
    assert!(far.is_finite() && far > near);
}

#[test]
fn perspective_framing_contains_horizontal_box_at_narrow_portrait_aspect() {
    let config = CameraNavigationConfig::default();
    let min = Vec3::new(-1.0, -0.05, -0.05);
    let max = Vec3::new(1.0, 0.05, 0.05);
    let mut cam = ArcballCamera {
        yaw: 0.0,
        elevation: 0.0,
        ..Default::default()
    };
    let perspective = PerspectiveProjection {
        fov: 45.0_f32.to_radians(),
        aspect_ratio: 0.5,
        ..default()
    };
    let projection = Projection::Perspective(perspective.clone());
    let FramingOutcome::Fit(fit) = frame_bounds_for_camera(
        Some((min, max)),
        &cam,
        &projection,
        Some(Vec2::new(500.0, 1000.0)),
        &config,
    ) else {
        panic!("expected valid perspective fit");
    };
    cam.focus = fit.focus;
    cam.distance = fit.distance;
    assert_all_corners_in_perspective_ndc(min, max, &cam, &perspective, &config);
}

#[test]
fn perspective_framing_contains_vertical_box_at_narrow_fov() {
    let config = CameraNavigationConfig::default();
    let min = Vec3::new(-0.05, -1.0, -0.05);
    let max = Vec3::new(0.05, 1.0, 0.05);
    let mut cam = ArcballCamera {
        yaw: 0.0,
        elevation: 0.0,
        ..Default::default()
    };
    let perspective = PerspectiveProjection {
        fov: 30.0_f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        ..default()
    };
    let projection = Projection::Perspective(perspective.clone());
    let FramingOutcome::Fit(fit) = frame_bounds_for_camera(
        Some((min, max)),
        &cam,
        &projection,
        Some(Vec2::new(1600.0, 900.0)),
        &config,
    ) else {
        panic!("expected valid perspective fit");
    };
    cam.focus = fit.focus;
    cam.distance = fit.distance;
    assert_all_corners_in_perspective_ndc(min, max, &cam, &perspective, &config);
}

#[test]
fn perspective_framing_contains_rotated_asymmetric_box_and_adapts_to_aspect_resize() {
    let config = CameraNavigationConfig::default();
    let min = Vec3::new(-4.0, -0.5, -1.5);
    let max = Vec3::new(4.0, 2.5, 1.5);
    let cam_base = ArcballCamera {
        yaw: 37.0_f32.to_radians(),
        elevation: 28.0_f32.to_radians(),
        ..Default::default()
    };

    let landscape_persp = PerspectiveProjection {
        fov: 45.0_f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        ..default()
    };
    let FramingOutcome::Fit(landscape_fit) = frame_bounds_for_camera(
        Some((min, max)),
        &cam_base,
        &Projection::Perspective(landscape_persp.clone()),
        Some(Vec2::new(1600.0, 900.0)),
        &config,
    ) else {
        panic!("expected landscape fit");
    };
    let mut landscape_cam = cam_base.clone();
    landscape_cam.focus = landscape_fit.focus;
    landscape_cam.distance = landscape_fit.distance;
    assert_all_corners_in_perspective_ndc(min, max, &landscape_cam, &landscape_persp, &config);

    let portrait_persp = PerspectiveProjection {
        fov: 45.0_f32.to_radians(),
        aspect_ratio: 0.5,
        ..default()
    };
    let FramingOutcome::Fit(portrait_fit) = frame_bounds_for_camera(
        Some((min, max)),
        &cam_base,
        &Projection::Perspective(portrait_persp.clone()),
        Some(Vec2::new(500.0, 1000.0)),
        &config,
    ) else {
        panic!("expected portrait fit");
    };
    assert!(
        portrait_fit.distance > landscape_fit.distance,
        "narrow portrait aspect must pull camera farther back for wide box"
    );
    let mut portrait_cam = cam_base;
    portrait_cam.focus = portrait_fit.focus;
    portrait_cam.distance = portrait_fit.distance;
    assert_all_corners_in_perspective_ndc(min, max, &portrait_cam, &portrait_persp, &config);
}

#[test]
fn orthographic_framing_fits_all_corners_across_varied_aspects() {
    let config = CameraNavigationConfig::default();
    let min = Vec3::new(-3.0, -1.0, -0.5);
    let max = Vec3::new(3.0, 1.0, 0.5);
    let cam_base = ArcballCamera {
        yaw: 20.0_f32.to_radians(),
        elevation: 15.0_f32.to_radians(),
        ..Default::default()
    };

    for (viewport, mode) in [
        (Vec2::new(1280.0, 720.0), ScalingMode::WindowSize),
        (Vec2::new(400.0, 1000.0), ScalingMode::WindowSize),
        (
            Vec2::new(400.0, 1000.0),
            ScalingMode::FixedVertical {
                viewport_height: 2.0,
            },
        ),
    ] {
        let mut ortho = OrthographicProjection {
            scaling_mode: mode,
            ..OrthographicProjection::default_3d()
        };
        ortho.update(viewport.x, viewport.y);
        let FramingOutcome::Fit(fit) = frame_bounds_for_camera(
            Some((min, max)),
            &cam_base,
            &Projection::Orthographic(ortho.clone()),
            Some(viewport),
            &config,
        ) else {
            panic!("expected orthographic fit for viewport {viewport:?} mode {mode:?}");
        };
        let scale = fit
            .orthographic
            .expect("orthographic fit must produce projection state")
            .scale;
        ortho.scale = scale;
        ortho.update(viewport.x, viewport.y);

        let mut cam = cam_base.clone();
        cam.focus = fit.focus;
        cam.distance = fit.distance;
        let mut tr = Transform::default();
        apply_rig(&cam, &mut tr);
        let view_from_world = tr.compute_affine().inverse();
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            );
            let view = view_from_world.transform_point3(corner);
            assert!(
                view.x >= ortho.area.min.x - 1.0e-4
                    && view.x <= ortho.area.max.x + 1.0e-4
                    && view.y >= ortho.area.min.y - 1.0e-4
                    && view.y <= ortho.area.max.y + 1.0e-4,
                "orthographic corner {corner:?} view={view:?} outside area {:?}",
                ortho.area
            );
        }
    }
}

#[test]
fn framing_for_tiny_valid_extent_has_no_two_metre_floor() {
    let config = CameraNavigationConfig::default();
    let tiny = SceneExtent {
        min: Vec3::splat(-5.0e-5),
        max: Vec3::splat(5.0e-5),
        count: 1,
        geometry_min: Vec3::splat(-5.0e-5),
        geometry_max: Vec3::splat(5.0e-5),
        geometry_count: 1,
    };
    let mut cam = ArcballCamera::default();
    let perspective = PerspectiveProjection {
        fov: 45.0_f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        ..default()
    };
    let FramingOutcome::Fit(fit) = framing_for_scene_extent(
        &tiny,
        &cam,
        &Projection::Perspective(perspective.clone()),
        Some(Vec2::new(1600.0, 900.0)),
        &config,
    ) else {
        panic!("expected tiny extent fit");
    };
    assert!(fit.focus.length() < 1.0e-6);
    assert!(
        fit.distance > 1.0e-4 && fit.distance < 1.0e-3,
        "0.1 mm cube should frame near sub-millimetre distance, got {}",
        fit.distance
    );
    cam.focus = fit.focus;
    cam.distance = fit.distance;
    assert_all_corners_in_perspective_ndc(tiny.min, tiny.max, &cam, &perspective, &config);
}

#[test]
fn framing_for_degenerate_nonfinite_inverted_and_zero_viewport_is_safe() {
    let config = CameraNavigationConfig::default();
    let cam = ArcballCamera::default();
    let projection = Projection::Perspective(PerspectiveProjection {
        aspect_ratio: 16.0 / 9.0,
        ..default()
    });

    // Zero-sized viewport must defer rather than guessing.
    assert_eq!(
        frame_bounds_for_camera(
            Some((Vec3::NEG_ONE, Vec3::ONE)),
            &cam,
            &projection,
            Some(Vec2::ZERO),
            &config,
        ),
        FramingOutcome::Deferred
    );

    // Zero-extent point keeps its exact center and frames a finite fallback region.
    let zero_extent = SceneExtent {
        min: Vec3::new(3.0, 4.0, 5.0),
        max: Vec3::new(3.0, 4.0, 5.0),
        count: 1,
        ..Default::default()
    };
    let FramingOutcome::Fit(point_fit) = framing_for_scene_extent(
        &zero_extent,
        &cam,
        &projection,
        Some(Vec2::new(1280.0, 720.0)),
        &config,
    ) else {
        panic!("expected safe point fallback fit");
    };
    assert_eq!(point_fit.focus, Vec3::new(3.0, 4.0, 5.0));
    assert!(point_fit.distance.is_finite() && point_fit.distance > 1.0);

    // NaN and inverted bounds fall back safely to origin without poisoning.
    for bad in [
        SceneExtent {
            min: Vec3::new(f32::NAN, 0.0, 0.0),
            max: Vec3::new(1.0, 1.0, 1.0),
            count: 1,
            ..Default::default()
        },
        SceneExtent {
            min: Vec3::new(5.0, 5.0, 5.0),
            max: Vec3::new(-5.0, -5.0, -5.0),
            count: 1,
            ..Default::default()
        },
    ] {
        let FramingOutcome::Fit(fallback) = framing_for_scene_extent(
            &bad,
            &cam,
            &projection,
            Some(Vec2::new(1280.0, 720.0)),
            &config,
        ) else {
            panic!("expected finite fallback for invalid extent");
        };
        assert_eq!(fallback.focus, Vec3::ZERO);
        assert!(fallback.distance.is_finite() && fallback.distance > 0.0);
    }
}

#[test]
fn apply_fly_to_rejects_nonfinite_targets_without_poisoning_camera() {
    let mut app = App::new();
    app.init_resource::<Time>()
        .insert_resource(FlyTo {
            target_focus: Vec3::new(f32::NAN, 0.0, 0.0),
            target_distance: f32::INFINITY,
            remaining: 0.4,
            duration: 0.4,
            start_focus: Vec3::ZERO,
            start_distance: 4.0,
            ..Default::default()
        })
        .add_systems(Update, apply_fly_to);

    let cam_entity = app
        .world_mut()
        .spawn((Transform::default(), ArcballCamera::default()))
        .id();
    app.update();

    let cam = app
        .world()
        .get::<ArcballCamera>(cam_entity)
        .expect("camera exists");
    assert!(cam.focus.is_finite());
    assert!(cam.distance.is_finite() && cam.distance > 0.0);
    assert_eq!(app.world().resource::<FlyTo>().remaining, 0.0);
}

#[test]
fn over_envelope_deep_orthographic_box_returns_unrepresentable() {
    let config = CameraNavigationConfig::default();
    let cam = ArcballCamera {
        yaw: 0.0,
        elevation: 0.0,
        ..Default::default()
    };
    let mut ortho = OrthographicProjection {
        scaling_mode: ScalingMode::Fixed {
            width: 2.0,
            height: 2.0,
        },
        ..OrthographicProjection::default_3d()
    };
    ortho.update(1000.0, 1000.0);
    let outcome = frame_bounds_for_camera(
        Some((Vec3::new(-1.0, -1.0, -2.0e12), Vec3::new(1.0, 1.0, 2.0e12))),
        &cam,
        &Projection::Orthographic(ortho),
        Some(Vec2::new(1000.0, 1000.0)),
        &config,
    );
    assert_eq!(
        outcome,
        FramingOutcome::Unrepresentable,
        "over-envelope orthographic depth must never clamp and return a false Fit"
    );
}

#[test]
fn quantized_perspective_at_large_offset_never_returns_false_fit() {
    let config = CameraNavigationConfig::default();
    let min = Vec3::new(1.0e8, -0.1, -0.1);
    let max = Vec3::new(1.0e8, 0.1, 0.1);
    let mut cam = ArcballCamera {
        yaw: core::f32::consts::FRAC_PI_2,
        elevation: 0.0,
        ..Default::default()
    };
    let perspective = PerspectiveProjection {
        fov: 45.0_f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        ..default()
    };
    let outcome = frame_bounds_for_camera(
        Some((min, max)),
        &cam,
        &Projection::Perspective(perspective.clone()),
        Some(Vec2::new(1600.0, 900.0)),
        &config,
    );
    match outcome {
        FramingOutcome::Fit(fit) => {
            cam.focus = fit.focus;
            cam.distance = fit.distance;
            assert_all_corners_in_perspective_ndc(min, max, &cam, &perspective, &config);
        }
        FramingOutcome::Unrepresentable => {}
        FramingOutcome::Deferred => panic!("valid viewport must not defer"),
    }
}

#[test]
fn tiny_orthographic_object_frames_without_physical_floor_across_modes() {
    let config = CameraNavigationConfig::default();
    let min = Vec3::splat(-5.0e-5);
    let max = Vec3::splat(5.0e-5);
    let cam_base = ArcballCamera {
        yaw: 15.0_f32.to_radians(),
        elevation: 20.0_f32.to_radians(),
        ..Default::default()
    };
    for (viewport, mode) in [
        (Vec2::new(1280.0, 720.0), ScalingMode::WindowSize),
        (Vec2::new(500.0, 1000.0), ScalingMode::WindowSize),
        (
            Vec2::new(1280.0, 720.0),
            ScalingMode::FixedVertical {
                viewport_height: 2.0,
            },
        ),
    ] {
        let mut ortho = OrthographicProjection {
            scaling_mode: mode,
            ..OrthographicProjection::default_3d()
        };
        ortho.update(viewport.x, viewport.y);
        let FramingOutcome::Fit(fit) = frame_bounds_for_camera(
            Some((min, max)),
            &cam_base,
            &Projection::Orthographic(ortho.clone()),
            Some(viewport),
            &config,
        ) else {
            panic!("expected tiny orthographic fit for {viewport:?} {mode:?}");
        };
        assert!(
            fit.distance < 1.0e-3,
            "0.1 mm ortho framing must not impose a physical floor: {}",
            fit.distance
        );
        let scale = fit.orthographic.expect("ortho state required").scale;
        ortho.scale = scale;
        ortho.update(viewport.x, viewport.y);
        let mut cam = cam_base.clone();
        cam.focus = fit.focus;
        cam.distance = fit.distance;
        let mut tr = Transform::default();
        apply_rig(&cam, &mut tr);
        let far = adaptive_far(
            f64::from(cam.distance),
            f64::from(cam.distance),
            f64::from((max - min).length()) * 0.5,
            0.0,
            &config,
        );
        let view_from_world = tr.compute_affine().inverse();
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            );
            let view = view_from_world.transform_point3(corner);
            let depth = -view.z;
            assert!(
                depth > 0.0 && depth < far,
                "tiny ortho corner {corner:?} depth {depth} outside (0, {far})"
            );
            assert!(
                view.x >= ortho.area.min.x - 1.0e-6
                    && view.x <= ortho.area.max.x + 1.0e-6
                    && view.y >= ortho.area.min.y - 1.0e-6
                    && view.y <= ortho.area.max.y + 1.0e-6
            );
        }
    }
}

#[test]
fn high_res_tiny_orthographic_square_attains_sub_1e7_scale_and_tight_world_height() {
    let config = CameraNavigationConfig::default();
    let min = Vec3::new(-5.0e-5, -5.0e-5, -5.0e-5);
    let max = Vec3::new(5.0e-5, 5.0e-5, 5.0e-5);
    let viewport = Vec2::new(3840.0, 2160.0);
    let cam = ArcballCamera {
        yaw: 0.0,
        elevation: 0.0,
        ..Default::default()
    };
    let mut ortho = OrthographicProjection {
        scaling_mode: ScalingMode::WindowSize,
        ..OrthographicProjection::default_3d()
    };
    ortho.update(viewport.x, viewport.y);

    let FramingOutcome::Fit(fit) = frame_bounds_for_camera(
        Some((min, max)),
        &cam,
        &Projection::Orthographic(ortho.clone()),
        Some(viewport),
        &config,
    ) else {
        panic!("expected tight orthographic fit for 0.1 mm square on 3840x2160");
    };
    let ortho_fit = fit.orthographic.expect("ortho state required");
    let scale = ortho_fit.scale;
    let expected_scale = 1.1e-4_f32 / 2160.0;
    assert!(
        scale < 1.0e-7 && (scale - expected_scale).abs() / expected_scale < 1.0e-3,
        "expected scale ~{expected_scale}, got {scale}"
    );
    let updated = validated_orthographic_projection(f64::from(scale), &ortho, Some(viewport))
        .expect("fitted ortho projection must be valid and finite");
    assert_eq!(updated.area, ortho_fit.area);
    assert!(
        (updated.area.height() - 1.1e-4).abs() / 1.1e-4 < 1.0e-3,
        "world height must match ~1.1e-4 m, got {}",
        updated.area.height()
    );
    assert!(
        updated
            .get_clip_from_view()
            .to_cols_array()
            .iter()
            .all(|v| v.is_finite())
    );
}

#[test]
fn point_framing_preserves_depth_defers_invalid_view_and_handles_extreme_offset() {
    let config = CameraNavigationConfig::default();
    let cam = ArcballCamera {
        focus: Vec3::ZERO,
        distance: 7.5,
        yaw: 0.0,
        elevation: 0.0,
        ..Default::default()
    };
    let persp = Projection::Perspective(PerspectiveProjection {
        fov: 45.0_f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        ..default()
    });

    // Zero viewport must defer rather than returning an unvalidated point fit.
    assert_eq!(
        frame_point_for_camera(
            Vec3::new(1.0, 2.0, 3.0),
            &cam,
            &persp,
            Some(Vec2::ZERO),
            &config
        ),
        FramingOutcome::Deferred
    );

    // Valid point preserves current camera distance.
    let FramingOutcome::Fit(fit) = frame_point_for_camera(
        Vec3::new(1.0, 2.0, 3.0),
        &cam,
        &persp,
        Some(Vec2::new(1600.0, 900.0)),
        &config,
    ) else {
        panic!("expected valid point fit");
    };
    assert_eq!(fit.focus, Vec3::new(1.0, 2.0, 3.0));
    assert!((fit.distance - 7.5).abs() < 1.0e-5);

    // Extreme-offset point where tiny current distance would round to focus
    // must either adjust to a non-collapsed representable depth or reject.
    let tiny_dist_cam = ArcballCamera {
        distance: 0.01,
        yaw: core::f32::consts::FRAC_PI_2,
        elevation: 0.0,
        ..Default::default()
    };
    let extreme_origin = Vec3::new(1.0e8, 0.0, 0.0);
    match frame_point_for_camera(
        extreme_origin,
        &tiny_dist_cam,
        &persp,
        Some(Vec2::new(1600.0, 900.0)),
        &config,
    ) {
        FramingOutcome::Fit(adj) => {
            let mut c = tiny_dist_cam;
            c.focus = adj.focus;
            c.distance = adj.distance;
            let mut tr = Transform::default();
            apply_rig(&c, &mut tr);
            assert!(
                (tr.translation - extreme_origin).length() > 0.0,
                "fitted point pose must not collapse onto origin"
            );
        }
        FramingOutcome::Unrepresentable => {}
        FramingOutcome::Deferred => panic!("valid viewport must not defer"),
    }
}

#[test]
fn apply_framing_fit_commits_validated_ortho_area_for_none_viewport_fallbacks() {
    let config = CameraNavigationConfig::default();

    // 1. Fixed 2x2 with stale initial scale=0.1, area=±0.1 and viewport=None.
    let mut fixed_ortho = OrthographicProjection {
        scaling_mode: ScalingMode::Fixed {
            width: 2.0,
            height: 2.0,
        },
        scale: 0.1,
        area: Rect::new(-0.1, -0.1, 0.1, 0.1),
        ..OrthographicProjection::default_3d()
    };
    let mut cam = ArcballCamera {
        yaw: 0.0,
        elevation: 0.0,
        ..Default::default()
    };
    let min = Vec3::new(-1.0, -1.0, -0.05);
    let max = Vec3::new(1.0, 1.0, 0.05);
    let FramingOutcome::Fit(fit) = frame_bounds_for_camera(
        Some((min, max)),
        &cam,
        &Projection::Orthographic(fixed_ortho.clone()),
        None,
        &config,
    ) else {
        panic!("expected valid Fixed 2x2 ortho fit with viewport=None");
    };
    let mut tr = Transform::default();
    let mut proj = Projection::Orthographic(fixed_ortho.clone());
    apply_framing_fit(fit, &mut cam, &mut tr, None, Some(&mut proj), None);

    let Projection::Orthographic(committed) = proj else {
        panic!("expected ortho projection");
    };
    assert!((committed.scale - 1.1).abs() < 1.0e-3);
    assert!(
        (committed.area.min.x + 1.1).abs() < 1.0e-3
            && (committed.area.max.x - 1.1).abs() < 1.0e-3
            && (committed.area.min.y + 1.1).abs() < 1.0e-3
            && (committed.area.max.y - 1.1).abs() < 1.0e-3,
        "apply_framing_fit must commit updated area even with viewport=None: {:?}",
        committed.area
    );
    assert!(
        committed
            .get_clip_from_view()
            .to_cols_array()
            .iter()
            .all(|v| v.is_finite())
    );

    // 2. WindowSize fallback with viewport=None derives consistent base dimensions
    // across a changed fit scale (no stale area poisoning subsequent fits).
    let mut win_ortho = OrthographicProjection {
        scaling_mode: ScalingMode::WindowSize,
        scale: 0.01,
        ..OrthographicProjection::default_3d()
    };
    win_ortho.update(1000.0, 500.0);
    let mut win_proj = Projection::Orthographic(win_ortho);
    let FramingOutcome::Fit(fit1) =
        frame_bounds_for_camera(Some((min, max)), &cam, &win_proj, None, &config)
    else {
        panic!("expected WindowSize fallback fit");
    };
    apply_framing_fit(fit1, &mut cam, &mut tr, None, Some(&mut win_proj), None);
    let FramingOutcome::Fit(fit2) =
        frame_bounds_for_camera(Some((min, max)), &cam, &win_proj, None, &config)
    else {
        panic!("expected consistent second WindowSize fallback fit");
    };
    let s1 = fit1.orthographic.expect("ortho").scale;
    let s2 = fit2.orthographic.expect("ortho").scale;
    assert!(
        (s1 - s2).abs() / s1 < 1.0e-4,
        "subsequent WindowSize None-viewport fit must derive identical scale: s1={s1}, s2={s2}"
    );
    fixed_ortho.area = committed.area;
    assert!(fixed_ortho.area.width() > 2.0);
}

#[test]
fn validated_orthographic_projection_rejects_invalid_depth_and_round_trips_endpoints() {
    let viewport = Vec2::new(1920.0, 1080.0);
    let mut base = OrthographicProjection {
        scaling_mode: ScalingMode::WindowSize,
        scale: 1.0,
        ..OrthographicProjection::default_3d()
    };
    base.update(viewport.x, viewport.y);

    // NaN near or reversed far must return None — never repair on a scratch copy.
    let mut nan_near = base.clone();
    nan_near.near = f32::NAN;
    assert!(validated_orthographic_projection(1.0, &nan_near, Some(viewport)).is_none());

    let mut reversed_depth = base.clone();
    reversed_depth.near = 10.0;
    reversed_depth.far = 5.0;
    assert!(validated_orthographic_projection(1.0, &reversed_depth, Some(viewport)).is_none());

    // Qualified scale endpoints must round-trip cleanly through f32 representation.
    let (min_scale, max_scale) =
        orthographic_scale_bounds(&base, Some(viewport)).expect("valid scale bounds");
    for endpoint in [min_scale, max_scale] {
        let committed = validated_orthographic_projection(endpoint, &base, Some(viewport))
            .expect("endpoint scale must validate");
        let revalidated = validated_orthographic_projection(
            f64::from(committed.scale),
            &committed,
            Some(viewport),
        )
        .expect("committed f32 endpoint scale must revalidate cleanly");
        assert_eq!(revalidated.scale, committed.scale);
        assert!(
            revalidated
                .get_clip_from_view()
                .to_cols_array()
                .iter()
                .all(|v| v.is_finite())
        );
    }
}
