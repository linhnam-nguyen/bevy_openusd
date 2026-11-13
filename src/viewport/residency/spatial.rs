//! R-tree-backed Scene payload candidates for camera residency.

use crate::project::cache_contract::{CachedTransform, SceneCacheAddress, SceneCacheEntry};
use bevy::camera::primitives::Aabb;
use bevy::math::{Mat4, Quat, Vec3};
use rstar::{AABB, RTree, RTreeObject};
use usd_model::{Bounds3, TransformSignature};
use usd_project::ScenePlacementTransform;

use super::authority::{ResidencyAuthority, ScenePayloadKey};
use super::camera::CameraAdmission;
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SceneSpatialPayload {
    pub(crate) address: SceneCacheAddress,
    pub(crate) payload_key: ScenePayloadKey,
    /// Scene-space matrix shared by camera admission and disposable projection.
    pub(crate) transform: ScenePlacementTransform,
    pub(crate) bounds: Bounds3,
    pub(crate) cpu_bytes: u64,
    pub(crate) gpu_bytes: u64,
}
impl SceneSpatialPayload {
    fn envelope(&self) -> AABB<[f64; 3]> {
        let (min, max) = normalized_bounds(self.bounds);
        AABB::from_corners(min, max)
    }

    pub(crate) fn admitted_to(&self, admission: &CameraAdmission) -> bool {
        let (min, max) = normalized_bounds(self.bounds);
        let aabb = Aabb::from_min_max(
            Vec3::from_array(min.map(|value| value as f32)),
            Vec3::from_array(max.map(|value| value as f32)),
        );
        if !admission.frustum.intersects_obb_identity(&aabb) {
            return false;
        }
        let preload_distance =
            (admission.sample.search_radius + admission.sample.preload_margin).max(0.0);
        if squared_distance_to_aabb(admission.sample.position, min, max)
            > preload_distance * preload_distance
        {
            return false;
        }
        admission
            .section_box
            .is_none_or(|section_box| intersects_clip_planes(min, max, section_box.planes))
    }
}
/// Resolves cache-local transforms and bounds into one Scene-space contract.
/// Parent matrices are applied in the same order as the live Bevy projection:
/// `world = parent * local`. The resulting matrix and transformed AABB are the
/// only spatial values consumed by the residency authority.
pub(crate) fn scene_payloads(entries: &[SceneCacheEntry]) -> Vec<SceneSpatialPayload> {
    let mut resolved = vec![None; entries.len()];
    let mut visiting = vec![false; entries.len()];
    entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let payload_key = ResidencyAuthority::payload_key(entry)?;
            let geometry = entry.geometry.as_ref()?;
            let bounds = entry.bounds?;
            let matrix = resolve_scene_matrix(index, entries, &mut resolved, &mut visiting);
            let (cpu_bytes, gpu_bytes) =
                ResidencyAuthority::conservative_resident_footprint(geometry.byte_size);
            Some(SceneSpatialPayload {
                address: entry.address.clone(),
                payload_key,
                transform: scene_transform(matrix),
                bounds: transformed_bounds(bounds, matrix),
                cpu_bytes,
                gpu_bytes,
            })
        })
        .collect()
}
fn resolve_scene_matrix(
    index: usize,
    entries: &[SceneCacheEntry],
    resolved: &mut [Option<Mat4>],
    visiting: &mut [bool],
) -> Mat4 {
    if let Some(matrix) = resolved.get(index).and_then(Option::as_ref) {
        return *matrix;
    }
    if index >= entries.len() || visiting[index] {
        return Mat4::IDENTITY;
    }
    visiting[index] = true;
    let entry = &entries[index];
    let local = cached_transform_matrix(&entry.transform);
    let parent = entry
        .parent
        .map(|parent| parent as usize)
        .filter(|parent| *parent < entries.len())
        .map(|parent| resolve_scene_matrix(parent, entries, resolved, visiting))
        .unwrap_or(Mat4::IDENTITY);
    let matrix = parent * local;
    visiting[index] = false;
    resolved[index] = Some(matrix);
    matrix
}
fn cached_transform_matrix(transform: &CachedTransform) -> Mat4 {
    match transform {
        CachedTransform::Placement(transform) => {
            Mat4::from_cols_array(&transform.0.map(|value| value as f32))
        }
        CachedTransform::Prim(signature) => prim_transform_matrix(signature),
    }
}
fn scene_transform(matrix: Mat4) -> ScenePlacementTransform {
    ScenePlacementTransform(matrix.to_cols_array().map(f64::from))
}
fn prim_transform_matrix(signature: &TransformSignature) -> Mat4 {
    let translation = signature.translation_mm.map(|value| value as f32 / 1_000.0);
    let rotation = signature
        .rotation_quantized
        .map(|value| value as f32 / 100_000.0);
    let scale = signature
        .scale_quantized
        .map(|value| value as f32 / 10_000.0);
    Mat4::from_scale_rotation_translation(
        Vec3::from_array(scale),
        Quat::from_xyzw(rotation[1], rotation[2], rotation[3], rotation[0]),
        Vec3::from_array(translation),
    )
}
fn transformed_bounds(bounds: Bounds3, matrix: Mat4) -> Bounds3 {
    let (min, max) = normalized_bounds(bounds);
    let mut transformed_min = [f64::INFINITY; 3];
    let mut transformed_max = [f64::NEG_INFINITY; 3];
    for index in 0..8 {
        let corner = Vec3::new(
            (if index & 1 == 0 { min[0] } else { max[0] }) as f32,
            (if index & 2 == 0 { min[1] } else { max[1] }) as f32,
            (if index & 4 == 0 { min[2] } else { max[2] }) as f32,
        );
        let world = matrix.transform_point3(corner);
        for axis in 0..3 {
            let value = f64::from(world.to_array()[axis]);
            transformed_min[axis] = transformed_min[axis].min(value);
            transformed_max[axis] = transformed_max[axis].max(value);
        }
    }
    Bounds3 {
        min: transformed_min,
        max: transformed_max,
    }
}
impl RTreeObject for SceneSpatialPayload {
    type Envelope = AABB<[f64; 3]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope()
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CameraRegion {
    pub(crate) min: [f64; 3],
    pub(crate) max: [f64; 3],
}
impl CameraRegion {
    pub(crate) fn around(center: [f64; 3], half_extent: f64, preload_margin: f64) -> Self {
        let extent = (half_extent + preload_margin.max(0.0)).max(0.0);
        Self {
            min: [center[0] - extent, center[1] - extent, center[2] - extent],
            max: [center[0] + extent, center[1] + extent, center[2] + extent],
        }
    }

    fn envelope(self) -> AABB<[f64; 3]> {
        AABB::from_corners(self.min, self.max)
    }
}
#[derive(Debug, Default)]
pub(crate) struct CameraCandidateIndex {
    tree: RTree<SceneSpatialPayload>,
}
impl CameraCandidateIndex {
    pub(crate) fn from_payloads(payloads: Vec<SceneSpatialPayload>) -> Self {
        Self {
            tree: RTree::bulk_load(payloads),
        }
    }

    pub(crate) fn candidates(
        &self,
        region: CameraRegion,
    ) -> impl Iterator<Item = &SceneSpatialPayload> {
        self.tree.locate_in_envelope_intersecting(region.envelope())
    }

    pub(crate) fn len(&self) -> usize {
        self.tree.size()
    }
}
fn normalized_bounds(bounds: Bounds3) -> ([f64; 3], [f64; 3]) {
    let mut min = [0.0; 3];
    let mut max = [0.0; 3];
    for axis in 0..3 {
        let left = finite_or_zero(bounds.min[axis]);
        let right = finite_or_zero(bounds.max[axis]);
        min[axis] = left.min(right);
        max[axis] = left.max(right);
    }
    (min, max)
}
fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}
fn squared_distance_to_aabb(point: [f64; 3], min: [f64; 3], max: [f64; 3]) -> f64 {
    point
        .into_iter()
        .zip(min.into_iter().zip(max))
        .map(|(point, (min, max))| {
            if point < min {
                (min - point).powi(2)
            } else if point > max {
                (point - max).powi(2)
            } else {
                0.0
            }
        })
        .sum()
}
fn intersects_clip_planes(min: [f64; 3], max: [f64; 3], planes: [[f64; 4]; 6]) -> bool {
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let half = [
        (max[0] - min[0]).abs() * 0.5,
        (max[1] - min[1]).abs() * 0.5,
        (max[2] - min[2]).abs() * 0.5,
    ];
    planes.into_iter().all(|plane| {
        let signed_center =
            plane[0] * center[0] + plane[1] * center[1] + plane[2] * center[2] + plane[3];
        let radius = plane[0].abs() * half[0] + plane[1].abs() * half[1] + plane[2].abs() * half[2];
        signed_center + radius >= 0.0
    })
}
#[cfg(test)]
#[path = "../../../tests/unit/viewport/residency/spatial_inline_tests.rs"]
mod tests;
