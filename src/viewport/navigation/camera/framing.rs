//! Frustum-aware camera framing policy shared by startup framing, panel
//! actions, and selected-prim focus commands.
//!
//! Computes candidate poses in `f64` from all 8 AABB corners against the
//! active camera basis and `Projection`, then validates every corner against
//! the actual represented `f32` `Transform` and effective clipping/area planes
//! without mutating live camera state.

use bevy::camera::{
    Camera, CameraProjection, OrthographicProjection, PerspectiveProjection, Projection,
    ScalingMode,
};
use bevy::camera_controller::pan_orbit_camera::controller::component::PanOrbitCamera;
use bevy::math::DVec3;
use bevy::prelude::*;

use super::navigation::{MIN_NEAR_PLANE, adaptive_far, adaptive_near};
use super::{
    ArcballCamera, CameraNavigationConfig, FramingIntent, MAX_CAMERA_DISTANCE, MIN_CAMERA_DISTANCE,
    apply_rig, distance_as_f32, finite_positive_distance, sanitize_camera_distance,
    sync_pan_orbit_from_arcball,
};
use crate::viewport::rendering::scene::visualization::SceneExtent;

/// Dimensionless framing margin (`>= 1.0`) leaving 10% breathing room around
/// fitted bounding corners inside the active viewing frustum.
const DEFAULT_FRAMING_MARGIN: f64 = 1.1;

/// Lower bound on representable orthographic world extent (`area.width()` /
/// `area.height()`) keeping `2.0 / extent` comfortably finite in `f32`.
const MIN_ORTHO_WORLD_EXTENT: f64 = 1.0e-30;

/// Upper bound on representable orthographic world extent keeping `area`
/// coordinates comfortably finite in `f32`.
const MAX_ORTHO_WORLD_EXTENT: f64 = 1.0e30;

/// Complete validated orthographic projection state produced by the framing
/// policy and committed atomically by [`apply_framing_fit`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OrthographicFit {
    pub scale: f32,
    pub area: Rect,
    pub near: f32,
    pub far: f32,
}

/// Validated camera pose produced by the shared frustum-aware framing policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FramingFit {
    pub focus: Vec3,
    pub distance: f32,
    pub orthographic: Option<OrthographicFit>,
}

/// Outcome of the shared frustum-aware framing policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FramingOutcome {
    /// Framing succeeded and all 8 corners were verified inside the actual
    /// represented `f32` camera pose, lateral frustum bounds, and ordered
    /// clipping planes.
    Fit(FramingFit),
    /// Active projection or viewport dimensions are not yet valid (for example
    /// a zero-sized viewport or unsupported custom projection); callers must
    /// defer without mutating camera state or mount mode.
    Deferred,
    /// Bounds cannot be contained within the qualified engineering envelope or
    /// actual `f32` rig representation; callers must leave live state intact.
    Unrepresentable,
}

#[derive(Debug, Clone, Copy)]
struct CameraBasis {
    right: DVec3,
    up: DVec3,
    forward: DVec3,
}

fn camera_basis_from_arcball(cam: &ArcballCamera) -> Option<CameraBasis> {
    let yaw = if cam.yaw.is_finite() {
        f64::from(cam.yaw)
    } else {
        0.0
    };
    let elevation = if cam.elevation.is_finite() {
        f64::from(cam.elevation)
    } else {
        0.0
    };
    let cos_e = elevation.cos();
    let sin_e = elevation.sin();
    let dir_from_focus = DVec3::new(cos_e * yaw.sin(), sin_e, cos_e * yaw.cos());
    let forward = (-dir_from_focus).try_normalize()?;
    let world_up = if cos_e.abs() <= 1.0e-5 {
        if sin_e >= 0.0 { DVec3::NEG_Z } else { DVec3::Z }
    } else {
        DVec3::Y
    };
    let right = forward.cross(world_up).try_normalize()?;
    let up = right.cross(forward).try_normalize()?;
    (right.is_finite() && up.is_finite() && forward.is_finite()).then_some(CameraBasis {
        right,
        up,
        forward,
    })
}

fn eight_aabb_corners(min: DVec3, max: DVec3) -> [DVec3; 8] {
    [
        DVec3::new(min.x, min.y, min.z),
        DVec3::new(max.x, min.y, min.z),
        DVec3::new(min.x, max.y, min.z),
        DVec3::new(max.x, max.y, min.z),
        DVec3::new(min.x, min.y, max.z),
        DVec3::new(max.x, min.y, max.z),
        DVec3::new(min.x, max.y, max.z),
        DVec3::new(max.x, max.y, max.z),
    ]
}

pub(crate) fn effective_viewport_size(camera: Option<&Camera>) -> Option<Vec2> {
    camera.and_then(|cam| {
        cam.logical_viewport_size()
            .or_else(|| cam.physical_viewport_size().map(|size| size.as_vec2()))
    })
}

fn validated_viewport_dims(viewport_size: Option<Vec2>) -> Result<Option<(f64, f64)>, ()> {
    match viewport_size {
        None => Ok(None),
        Some(size) if size.is_finite() && size.x > 0.0 && size.y > 0.0 => {
            Ok(Some((f64::from(size.x), f64::from(size.y))))
        }
        Some(_) => Err(()),
    }
}

fn ortho_base_dimensions(
    ortho: &OrthographicProjection,
    viewport_dims: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    match ortho.scaling_mode {
        ScalingMode::Fixed { width, height } => {
            let w = f64::from(width);
            let h = f64::from(height);
            (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some((w, h))
        }
        mode => {
            if let Some((width, height)) = viewport_dims {
                let (w, h) = match mode {
                    ScalingMode::WindowSize => (width, height),
                    ScalingMode::AutoMin {
                        min_width,
                        min_height,
                    } => {
                        let mw = f64::from(min_width);
                        let mh = f64::from(min_height);
                        if !mw.is_finite() || !mh.is_finite() || mw <= 0.0 || mh <= 0.0 {
                            return None;
                        }
                        if width * mh > mw * height {
                            (width * mh / height, mh)
                        } else {
                            (mw, height * mw / width)
                        }
                    }
                    ScalingMode::AutoMax {
                        max_width,
                        max_height,
                    } => {
                        let mw = f64::from(max_width);
                        let mh = f64::from(max_height);
                        if !mw.is_finite() || !mh.is_finite() || mw <= 0.0 || mh <= 0.0 {
                            return None;
                        }
                        if width * mh < mw * height {
                            (width * mh / height, mh)
                        } else {
                            (mw, height * mw / width)
                        }
                    }
                    ScalingMode::FixedVertical { viewport_height } => {
                        let vh = f64::from(viewport_height);
                        if !vh.is_finite() || vh <= 0.0 {
                            return None;
                        }
                        (width * vh / height, vh)
                    }
                    ScalingMode::FixedHorizontal { viewport_width } => {
                        let vw = f64::from(viewport_width);
                        if !vw.is_finite() || vw <= 0.0 {
                            return None;
                        }
                        (vw, height * vw / width)
                    }
                    ScalingMode::Fixed { .. } => unreachable!(),
                };
                (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some((w, h))
            } else {
                let scale = f64::from(ortho.scale);
                let area_w = f64::from(ortho.area.width());
                let area_h = f64::from(ortho.area.height());
                if scale.is_finite()
                    && scale > 0.0
                    && area_w.is_finite()
                    && area_w > 0.0
                    && area_h.is_finite()
                    && area_h > 0.0
                {
                    Some((area_w / scale, area_h / scale))
                } else {
                    None
                }
            }
        }
    }
}

/// Validates a temporary `f32` perspective rig candidate against all 8 AABB
/// corners using the actual represented `f32` translation, rotation, and
/// effective local near/far planes.
fn validate_actual_perspective_candidate(
    corners: &[DVec3; 8],
    max_radius: f64,
    cam: &ArcballCamera,
    focus: Vec3,
    distance: f64,
    tx: f64,
    ty: f64,
    config: &CameraNavigationConfig,
) -> Option<FramingFit> {
    if !focus.is_finite() || !distance.is_finite() || distance > MAX_CAMERA_DISTANCE {
        return None;
    }
    let distance_f32 = distance_as_f32(distance);
    if !distance_f32.is_finite() || distance_f32 <= 0.0 {
        return None;
    }

    let mut candidate_cam = cam.clone();
    candidate_cam.focus = focus;
    candidate_cam.distance = distance_f32;
    let mut candidate_tr = Transform::default();
    apply_rig(&candidate_cam, &mut candidate_tr);
    if !candidate_tr.translation.is_finite() || !candidate_tr.rotation.is_finite() {
        return None;
    }

    let near = adaptive_near(f64::from(distance_f32), config);
    let far = adaptive_far(
        f64::from(distance_f32),
        f64::from(distance_f32),
        max_radius,
        near,
        config,
    );
    if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
        return None;
    }

    let actual_pos = candidate_tr.translation.as_dvec3();
    let actual_right = (candidate_tr.rotation * Vec3::X).as_dvec3();
    let actual_up = (candidate_tr.rotation * Vec3::Y).as_dvec3();
    let actual_forward = (candidate_tr.rotation * Vec3::NEG_Z).as_dvec3();
    let near_f64 = f64::from(near);
    let far_f64 = f64::from(far);

    for corner in corners {
        let rel = *corner - actual_pos;
        let x = rel.dot(actual_right);
        let y = rel.dot(actual_up);
        let depth = rel.dot(actual_forward);
        if !x.is_finite()
            || !y.is_finite()
            || !depth.is_finite()
            || depth <= near_f64
            || depth >= far_f64
            || x.abs() > depth * tx * (1.0 + 1.0e-5)
            || y.abs() > depth * ty * (1.0 + 1.0e-5)
        {
            return None;
        }
    }

    Some(FramingFit {
        focus,
        distance: distance_f32,
        orthographic: None,
    })
}

/// Computes the usable `(min_scale, max_scale)` envelope for an orthographic
/// projection directly from its active base dimensions and finite `f32`
/// area/matrix representability (decoupled from world-space camera distance).
///
/// Endpoints are aligned to representable `f32` values inside the valid range
/// so committed endpoint scales always round-trip through validation.
pub(crate) fn orthographic_scale_bounds(
    ortho: &OrthographicProjection,
    viewport_size: Option<Vec2>,
) -> Option<(f64, f64)> {
    let viewport_dims = validated_viewport_dims(viewport_size).ok()?;
    let (proj_w, proj_h) = ortho_base_dimensions(ortho, viewport_dims)?;
    let min_dim = proj_w.min(proj_h);
    let max_dim = proj_w.max(proj_h);
    if !min_dim.is_finite() || min_dim <= 0.0 || !max_dim.is_finite() || max_dim <= 0.0 {
        return None;
    }
    let raw_min = (MIN_ORTHO_WORLD_EXTENT / min_dim).max(f64::from(f32::MIN_POSITIVE) * 16.0);
    let raw_max = (MAX_ORTHO_WORLD_EXTENT / max_dim).min(f64::from(f32::MAX) * 1.0e-6);
    if !raw_min.is_finite() || !raw_max.is_finite() || raw_min <= 0.0 || raw_max <= raw_min {
        return None;
    }
    let min_f32 = raw_min as f32;
    let min_scale = if f64::from(min_f32) < raw_min {
        f64::from(min_f32.next_up())
    } else {
        f64::from(min_f32)
    };
    let max_f32 = raw_max as f32;
    let max_scale = if f64::from(max_f32) > raw_max {
        f64::from(max_f32.next_down())
    } else {
        f64::from(max_f32)
    };
    (min_scale.is_finite() && max_scale.is_finite() && min_scale > 0.0 && max_scale > min_scale)
        .then_some((min_scale, max_scale))
}

/// Lateral-only orthographic scale and area validator (does not validate or
/// alter `near`/`far` depth planes). Returns the represented `f32` `(scale, area)`
/// only when both are finite, non-degenerate, and produce finite lateral
/// reciprocals (`2.0 / width`, `2.0 / height`).
pub(crate) fn validated_orthographic_scale_area(
    scale: f64,
    ortho: &OrthographicProjection,
    viewport_size: Option<Vec2>,
) -> Option<(f32, Rect)> {
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    let viewport_dims = validated_viewport_dims(viewport_size).ok()?;
    let (proj_w, proj_h) = ortho_base_dimensions(ortho, viewport_dims)?;
    let (min_scale, max_scale) = orthographic_scale_bounds(ortho, viewport_size)?;
    let scale_f32 = scale as f32;
    let represented = f64::from(scale_f32);
    if !scale_f32.is_finite()
        || scale_f32 <= 0.0
        || represented < min_scale
        || represented > max_scale
    {
        return None;
    }

    let mut candidate = ortho.clone();
    candidate.scale = scale_f32;
    if let Some((w, h)) = viewport_dims {
        candidate.update(w as f32, h as f32);
    } else {
        let ox = proj_w * f64::from(ortho.viewport_origin.x);
        let oy = proj_h * f64::from(ortho.viewport_origin.y);
        candidate.area = Rect::new(
            (represented * -ox) as f32,
            (represented * -oy) as f32,
            (represented * (proj_w - ox)) as f32,
            (represented * (proj_h - oy)) as f32,
        );
    }

    let area = candidate.area;
    let width = area.width();
    let height = area.height();
    if !area.min.is_finite()
        || !area.max.is_finite()
        || !width.is_finite()
        || width <= 0.0
        || !height.is_finite()
        || height <= 0.0
        || !(2.0_f32 / width).is_finite()
        || !(2.0_f32 / height).is_finite()
    {
        return None;
    }

    Some((scale_f32, area))
}

/// Complete orthographic projection validator: validates lateral `(scale, area)`,
/// requires finite ordered depth planes (`near < far`) on the candidate itself,
/// and verifies that `candidate.get_clip_from_view()` is completely finite.
/// Never repairs invalid depth planes on a scratch copy.
pub(crate) fn validated_orthographic_projection(
    scale: f64,
    ortho: &OrthographicProjection,
    viewport_size: Option<Vec2>,
) -> Option<OrthographicProjection> {
    if !ortho.near.is_finite() || !ortho.far.is_finite() || ortho.far <= ortho.near {
        return None;
    }
    let (scale_f32, area) = validated_orthographic_scale_area(scale, ortho, viewport_size)?;
    let mut candidate = ortho.clone();
    candidate.scale = scale_f32;
    candidate.area = area;
    let clip = candidate.get_clip_from_view();
    if !clip.to_cols_array().iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(candidate)
}

/// Computes the next orthographic `scale` for a scroll delta using
/// frame-partition-invariant, overflow-safe log-room arithmetic saturated
/// strictly against `[min_scale, max_scale]`.
pub(crate) fn exponential_orthographic_scale(
    current_scale: f64,
    scroll_delta: f64,
    zoom_speed: f64,
    min_scale: f64,
    max_scale: f64,
) -> Option<f64> {
    if !current_scale.is_finite()
        || current_scale <= 0.0
        || !min_scale.is_finite()
        || !max_scale.is_finite()
        || min_scale <= 0.0
        || max_scale <= min_scale
    {
        return None;
    }
    let base = current_scale.clamp(min_scale, max_scale);
    if !scroll_delta.is_finite()
        || scroll_delta == 0.0
        || !zoom_speed.is_finite()
        || zoom_speed <= 0.0
    {
        return Some(base);
    }
    let lo = min_scale.ln();
    let hi = max_scale.ln();
    let b = base.ln().clamp(lo, hi);
    let target = if scroll_delta > 0.0 {
        let room = (b - lo).max(0.0);
        let step = if room == 0.0 || scroll_delta >= room / zoom_speed {
            room
        } else {
            scroll_delta * zoom_speed
        };
        b - step
    } else {
        let room = (hi - b).max(0.0);
        let magnitude = -scroll_delta;
        let step = if room == 0.0 || magnitude >= room / zoom_speed {
            room
        } else {
            magnitude * zoom_speed
        };
        b + step
    };
    let clamped = target.clamp(lo, hi);
    if !clamped.is_finite() {
        return Some(base);
    }
    let next = clamped.exp();
    (next.is_finite() && next > 0.0).then_some(next.clamp(min_scale, max_scale))
}

/// Validates a temporary `f32` orthographic rig candidate and effective
/// `OrthographicProjection` area/matrix/clipping against all 8 AABB corners.
#[allow(clippy::too_many_arguments)]
fn validate_actual_orthographic_candidate(
    corners: &[DVec3; 8],
    max_radius: f64,
    cam: &ArcballCamera,
    focus: Vec3,
    distance: f64,
    scale: f64,
    ortho: &OrthographicProjection,
    viewport_size: Option<Vec2>,
    config: &CameraNavigationConfig,
) -> Option<FramingFit> {
    if !focus.is_finite()
        || !distance.is_finite()
        || distance <= 0.0
        || distance > MAX_CAMERA_DISTANCE
    {
        return None;
    }
    let distance_f32 = distance_as_f32(distance);
    if !distance_f32.is_finite() || distance_f32 <= 0.0 {
        return None;
    }

    let near = 0.0_f32;
    let far = adaptive_far(
        f64::from(distance_f32),
        f64::from(distance_f32),
        max_radius,
        near,
        config,
    );
    if !far.is_finite() || far <= near {
        return None;
    }

    let mut prospective = ortho.clone();
    prospective.near = near;
    prospective.far = far;
    let candidate_ortho = validated_orthographic_projection(scale, &prospective, viewport_size)?;

    let mut candidate_cam = cam.clone();
    candidate_cam.focus = focus;
    candidate_cam.distance = distance_f32;
    let mut candidate_tr = Transform::default();
    apply_rig(&candidate_cam, &mut candidate_tr);
    if !candidate_tr.translation.is_finite() || !candidate_tr.rotation.is_finite() {
        return None;
    }

    let area = candidate_ortho.area;
    let tol_x = f64::from(area.width().abs()) * 1.0e-5 + 1.0e-12;
    let tol_y = f64::from(area.height().abs()) * 1.0e-5 + 1.0e-12;
    let actual_pos = candidate_tr.translation.as_dvec3();
    let actual_right = (candidate_tr.rotation * Vec3::X).as_dvec3();
    let actual_up = (candidate_tr.rotation * Vec3::Y).as_dvec3();
    let actual_forward = (candidate_tr.rotation * Vec3::NEG_Z).as_dvec3();

    for corner in corners {
        let rel = *corner - actual_pos;
        let x = rel.dot(actual_right);
        let y = rel.dot(actual_up);
        let depth = rel.dot(actual_forward);
        if !x.is_finite()
            || !y.is_finite()
            || !depth.is_finite()
            || depth <= f64::from(candidate_ortho.near)
            || depth >= f64::from(candidate_ortho.far)
            || x < f64::from(area.min.x) - tol_x
            || x > f64::from(area.max.x) + tol_x
            || y < f64::from(area.min.y) - tol_y
            || y > f64::from(area.max.y) + tol_y
        {
            return None;
        }
    }

    Some(FramingFit {
        focus,
        distance: distance_f32,
        orthographic: Some(OrthographicFit {
            scale: candidate_ortho.scale,
            area: candidate_ortho.area,
            near: candidate_ortho.near,
            far: candidate_ortho.far,
        }),
    })
}

fn f32_ulp_separation_hint(focus: Vec3) -> f64 {
    let max_coord = f64::from(focus.abs().max_element());
    if max_coord.is_finite() && max_coord > 0.0 {
        max_coord * f64::from(f32::EPSILON) * 4.0
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
fn fit_perspective_corners(
    corners: &[DVec3; 8],
    center: DVec3,
    cam: &ArcballCamera,
    basis: CameraBasis,
    perspective: &PerspectiveProjection,
    viewport_dims: Option<(f64, f64)>,
    margin: f64,
    config: &CameraNavigationConfig,
) -> FramingOutcome {
    let vfov = f64::from(perspective.fov);
    if !vfov.is_finite() || vfov <= 0.0 || vfov >= std::f64::consts::PI {
        return FramingOutcome::Deferred;
    }
    let aspect = match viewport_dims {
        Some((w, h)) => w / h,
        None => f64::from(perspective.aspect_ratio),
    };
    if !aspect.is_finite() || aspect <= 0.0 {
        return FramingOutcome::Deferred;
    }
    let ty = (vfov * 0.5).tan();
    let tx = ty * aspect;
    if !ty.is_finite() || ty <= 0.0 || !tx.is_finite() || tx <= 0.0 {
        return FramingOutcome::Deferred;
    }

    let mut required_distance = MIN_CAMERA_DISTANCE;
    let mut min_z = f64::INFINITY;
    let mut max_radius = 0.0_f64;

    for corner in corners {
        let delta = *corner - center;
        let x = delta.dot(basis.right);
        let y = delta.dot(basis.up);
        let z = delta.dot(basis.forward);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return FramingOutcome::Unrepresentable;
        }
        min_z = min_z.min(z);
        max_radius = max_radius.max(delta.length());

        let dist_x = margin * x.abs() / tx - z;
        let dist_y = margin * y.abs() / ty - z;
        let dist_depth = MIN_NEAR_PLANE * 2.0 - z;
        required_distance = required_distance.max(dist_x).max(dist_y).max(dist_depth);
    }

    // Ensure the closest corner stays strictly in front of the active local
    // near clipping plane under `adaptive_near`.
    let near_ratio =
        if config.near_focus_distance_ratio.is_finite() && config.near_focus_distance_ratio > 0.0 {
            config.near_focus_distance_ratio.min(0.25)
        } else {
            1.0e-3
        };
    let near_denom = 1.0 - near_ratio * 1.1;
    if min_z < 0.0 && near_denom > 0.0 {
        required_distance = required_distance
            .max((-min_z) / near_denom)
            .max(MIN_NEAR_PLANE * 1.1 - min_z);
    }

    if !required_distance.is_finite() || required_distance > MAX_CAMERA_DISTANCE {
        return FramingOutcome::Unrepresentable;
    }

    let focus = center.as_vec3();
    let candidate_distance = required_distance.clamp(MIN_CAMERA_DISTANCE, MAX_CAMERA_DISTANCE);
    if let Some(fit) = validate_actual_perspective_candidate(
        corners,
        max_radius,
        cam,
        focus,
        candidate_distance,
        tx,
        ty,
        config,
    ) {
        return FramingOutcome::Fit(fit);
    }

    // If f32 translation quantization at a large world offset collapsed or
    // shifted the candidate pose, retry with a representable f32 separation
    // strictly inside the engineering envelope and verify all 8 corners again.
    let quantized_retry = candidate_distance.max(f32_ulp_separation_hint(focus) * 2.0);
    if quantized_retry > candidate_distance
        && quantized_retry <= MAX_CAMERA_DISTANCE
        && let Some(fit) = validate_actual_perspective_candidate(
            corners,
            max_radius,
            cam,
            focus,
            quantized_retry,
            tx,
            ty,
            config,
        )
    {
        return FramingOutcome::Fit(fit);
    }

    FramingOutcome::Unrepresentable
}

#[allow(clippy::too_many_arguments)]
fn fit_orthographic_corners(
    corners: &[DVec3; 8],
    center: DVec3,
    cam: &ArcballCamera,
    basis: CameraBasis,
    ortho: &OrthographicProjection,
    viewport_size: Option<Vec2>,
    viewport_dims: Option<(f64, f64)>,
    margin: f64,
    config: &CameraNavigationConfig,
) -> FramingOutcome {
    let Some((proj_w, proj_h)) = ortho_base_dimensions(ortho, viewport_dims) else {
        return FramingOutcome::Deferred;
    };
    let origin_x = f64::from(ortho.viewport_origin.x);
    let origin_y = f64::from(ortho.viewport_origin.y);
    if !origin_x.is_finite() || !origin_y.is_finite() {
        return FramingOutcome::Deferred;
    }
    let half_w = proj_w * origin_x.min(1.0 - origin_x);
    let half_h = proj_h * origin_y.min(1.0 - origin_y);
    if !half_w.is_finite() || half_w <= 0.0 || !half_h.is_finite() || half_h <= 0.0 {
        return FramingOutcome::Deferred;
    }

    let mut max_abs_x = 0.0_f64;
    let mut max_abs_y = 0.0_f64;
    let mut min_z = f64::INFINITY;
    let mut max_z = f64::NEG_INFINITY;
    let mut max_radius = 0.0_f64;
    for corner in corners {
        let delta = *corner - center;
        let x = delta.dot(basis.right);
        let y = delta.dot(basis.up);
        let z = delta.dot(basis.forward);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return FramingOutcome::Unrepresentable;
        }
        max_abs_x = max_abs_x.max(x.abs());
        max_abs_y = max_abs_y.max(y.abs());
        min_z = min_z.min(z);
        max_z = max_z.max(z);
        max_radius = max_radius.max(delta.length());
    }

    let Some((min_scale, max_scale)) = orthographic_scale_bounds(ortho, viewport_size) else {
        return FramingOutcome::Deferred;
    };
    let required_scale = (margin * max_abs_x / half_w)
        .max(margin * max_abs_y / half_h)
        .max(min_scale);
    let half_depth = (max_z - min_z).abs() * 0.5;
    let required_distance = ((-min_z) * 1.1 + MIN_NEAR_PLANE * 2.0)
        .max(half_depth * 2.0)
        .max(max_abs_x.max(max_abs_y) * 2.0)
        .max(MIN_CAMERA_DISTANCE);

    // Reject impossible scale (against true scale/area representability) or
    // over-envelope depth distance BEFORE clamping so out-of-range boxes never
    // produce a false Fit.
    if !required_scale.is_finite()
        || required_scale <= 0.0
        || required_scale > max_scale
        || !required_distance.is_finite()
        || required_distance > MAX_CAMERA_DISTANCE
    {
        return FramingOutcome::Unrepresentable;
    }

    let focus = center.as_vec3();
    if let Some(fit) = validate_actual_orthographic_candidate(
        corners,
        max_radius,
        cam,
        focus,
        required_distance,
        required_scale,
        ortho,
        viewport_size,
        config,
    ) {
        return FramingOutcome::Fit(fit);
    }

    let quantized_retry = required_distance.max(f32_ulp_separation_hint(focus) * 2.0);
    if quantized_retry > required_distance
        && quantized_retry <= MAX_CAMERA_DISTANCE
        && let Some(fit) = validate_actual_orthographic_candidate(
            corners,
            max_radius,
            cam,
            focus,
            quantized_retry,
            required_scale,
            ortho,
            viewport_size,
            config,
        )
    {
        return FramingOutcome::Fit(fit);
    }

    FramingOutcome::Unrepresentable
}

/// Single viewport-owned frustum-aware framing policy shared by startup
/// framing, panel actions, and selected-prim focus commands.
///
/// Fits all 8 corners of `bounds` against the active camera basis and
/// `Projection` using stable `f64` arithmetic and validates the resulting
/// actual `f32` `Transform` and effective clipping/area planes before
/// returning [`FramingOutcome::Fit`]. Empty, degenerate zero-extent, or
/// non-finite bounds fall back safely to the configured empty-scene extent;
/// zero-sized viewports or unsupported projections return
/// [`FramingOutcome::Deferred`].
pub(crate) fn frame_bounds_for_camera(
    bounds: Option<(Vec3, Vec3)>,
    cam: &ArcballCamera,
    projection: &Projection,
    viewport_size: Option<Vec2>,
    config: &CameraNavigationConfig,
) -> FramingOutcome {
    let Ok(viewport_dims) = validated_viewport_dims(viewport_size) else {
        return FramingOutcome::Deferred;
    };
    let Some(basis) = camera_basis_from_arcball(cam) else {
        return FramingOutcome::Deferred;
    };
    let fallback_radius = finite_positive_distance(config.empty_scene_radius, 1.0);
    let (center, min_f64, max_f64) = match bounds {
        Some((min, max)) if min.is_finite() && max.is_finite() && min.cmple(max).all() => {
            let min_d = min.as_dvec3();
            let max_d = max.as_dvec3();
            let center_d = min_d * 0.5 + max_d * 0.5;
            if !center_d.is_finite() {
                return FramingOutcome::Unrepresentable;
            }
            if (max_d - min_d).length_squared() <= 0.0 {
                // Degenerate zero-extent point: keep its exact center and frame
                // a fallback region around it.
                let half = DVec3::splat(fallback_radius);
                (center_d, center_d - half, center_d + half)
            } else {
                (center_d, min_d, max_d)
            }
        }
        _ => {
            let half = DVec3::splat(fallback_radius);
            (DVec3::ZERO, -half, half)
        }
    };
    let corners = eight_aabb_corners(min_f64, max_f64);
    match projection {
        Projection::Perspective(perspective) => fit_perspective_corners(
            &corners,
            center,
            cam,
            basis,
            perspective,
            viewport_dims,
            DEFAULT_FRAMING_MARGIN,
            config,
        ),
        Projection::Orthographic(ortho) => fit_orthographic_corners(
            &corners,
            center,
            cam,
            basis,
            ortho,
            viewport_size,
            viewport_dims,
            DEFAULT_FRAMING_MARGIN,
            config,
        ),
        Projection::Custom(_) => FramingOutcome::Deferred,
    }
}

/// Validates a point-only focus target (such as a transform-only prim with no
/// mesh bounds) against the active projection, viewport, and actual `f32` rig.
///
/// Preserves the camera's current distance (and current orthographic scale)
/// whenever the resulting actual `f32` pose has valid positive finite depth
/// inside ordered clipping planes; if `f32` quantization at a large world
/// offset collapses depth onto the point, retries with a bounded representable
/// separation or returns [`FramingOutcome::Unrepresentable`].
pub(crate) fn frame_point_for_camera(
    origin: Vec3,
    cam: &ArcballCamera,
    projection: &Projection,
    viewport_size: Option<Vec2>,
    config: &CameraNavigationConfig,
) -> FramingOutcome {
    let Ok(viewport_dims) = validated_viewport_dims(viewport_size) else {
        return FramingOutcome::Deferred;
    };
    if camera_basis_from_arcball(cam).is_none() {
        return FramingOutcome::Deferred;
    }
    if !origin.is_finite() {
        return FramingOutcome::Unrepresentable;
    }
    let current_distance =
        sanitize_camera_distance(f64::from(cam.distance), config.empty_scene_distance);
    let point_corners = [origin.as_dvec3(); 8];

    match projection {
        Projection::Perspective(perspective) => {
            let vfov = f64::from(perspective.fov);
            if !vfov.is_finite() || vfov <= 0.0 || vfov >= std::f64::consts::PI {
                return FramingOutcome::Deferred;
            }
            let aspect = match viewport_dims {
                Some((w, h)) => w / h,
                None => f64::from(perspective.aspect_ratio),
            };
            if !aspect.is_finite() || aspect <= 0.0 {
                return FramingOutcome::Deferred;
            }
            let ty = (vfov * 0.5).tan();
            let tx = ty * aspect;
            if !ty.is_finite() || ty <= 0.0 || !tx.is_finite() || tx <= 0.0 {
                return FramingOutcome::Deferred;
            }
            if let Some(fit) = validate_actual_perspective_candidate(
                &point_corners,
                0.0,
                cam,
                origin,
                current_distance,
                tx,
                ty,
                config,
            ) {
                return FramingOutcome::Fit(fit);
            }
            let quantized_retry = current_distance.max(f32_ulp_separation_hint(origin) * 2.0);
            if quantized_retry > current_distance
                && quantized_retry <= MAX_CAMERA_DISTANCE
                && let Some(fit) = validate_actual_perspective_candidate(
                    &point_corners,
                    0.0,
                    cam,
                    origin,
                    quantized_retry,
                    tx,
                    ty,
                    config,
                )
            {
                return FramingOutcome::Fit(fit);
            }
            FramingOutcome::Unrepresentable
        }
        Projection::Orthographic(ortho) => {
            if ortho_base_dimensions(ortho, viewport_dims).is_none() {
                return FramingOutcome::Deferred;
            }
            let current_scale = f64::from(ortho.scale);
            if let Some(fit) = validate_actual_orthographic_candidate(
                &point_corners,
                0.0,
                cam,
                origin,
                current_distance,
                current_scale,
                ortho,
                viewport_size,
                config,
            ) {
                return FramingOutcome::Fit(fit);
            }
            let quantized_retry = current_distance.max(f32_ulp_separation_hint(origin) * 2.0);
            if quantized_retry > current_distance
                && quantized_retry <= MAX_CAMERA_DISTANCE
                && let Some(fit) = validate_actual_orthographic_candidate(
                    &point_corners,
                    0.0,
                    cam,
                    origin,
                    quantized_retry,
                    current_scale,
                    ortho,
                    viewport_size,
                    config,
                )
            {
                return FramingOutcome::Fit(fit);
            }
            FramingOutcome::Unrepresentable
        }
        Projection::Custom(_) => FramingOutcome::Deferred,
    }
}

/// Evaluates a retained [`FramingIntent`] (bounds or point) against the active
/// camera basis, projection, and viewport.
pub(crate) fn frame_intent_for_camera(
    intent: FramingIntent,
    cam: &ArcballCamera,
    projection: &Projection,
    viewport_size: Option<Vec2>,
    config: &CameraNavigationConfig,
) -> FramingOutcome {
    match intent {
        FramingIntent::Bounds { min, max } => {
            frame_bounds_for_camera(Some((min, max)), cam, projection, viewport_size, config)
        }
        FramingIntent::Point(origin) => {
            frame_point_for_camera(origin, cam, projection, viewport_size, config)
        }
    }
}

/// Applies a validated [`FramingFit`] onto the free camera rig, transform,
/// complete validated orthographic projection (`scale`, `area`, `near`, `far`),
/// and `PanOrbitCamera` anchor. Works identically whether viewport dimensions
/// were explicitly supplied or derived from an accepted fallback context.
pub(crate) fn apply_framing_fit(
    fit: FramingFit,
    cam: &mut ArcballCamera,
    transform: &mut Transform,
    pan_orbit: Option<&mut PanOrbitCamera>,
    projection: Option<&mut Projection>,
    _viewport_size: Option<Vec2>,
) {
    cam.focus = fit.focus;
    cam.distance = fit.distance;
    cam.zoom_target = f64::from(fit.distance);
    if let (Some(ortho_fit), Some(Projection::Orthographic(ortho))) = (fit.orthographic, projection)
    {
        ortho.scale = ortho_fit.scale;
        ortho.area = ortho_fit.area;
        ortho.near = ortho_fit.near;
        ortho.far = ortho_fit.far;
    }
    apply_rig(cam, transform);
    if let Some(pan_orbit) = pan_orbit {
        sync_pan_orbit_from_arcball(cam, pan_orbit);
    }
}

/// Computes frustum-aware framing for a [`SceneExtent`] using the single
/// shared camera framing policy.
pub(crate) fn framing_for_scene_extent(
    extent: &SceneExtent,
    cam: &ArcballCamera,
    projection: &Projection,
    viewport_size: Option<Vec2>,
    config: &CameraNavigationConfig,
) -> FramingOutcome {
    frame_bounds_for_camera(
        extent.valid_bounds(),
        cam,
        projection,
        viewport_size,
        config,
    )
}
