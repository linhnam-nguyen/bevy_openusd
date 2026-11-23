use std::collections::BTreeMap;

use bevy::camera::{CameraProjection, Projection};
use bevy::math::DVec3;
use bevy::prelude::*;
use usd_bevy::LiveStage;
use usd_model::Bounds3;
use usd_project::SceneId;

use crate::project::cache::SceneCacheStore;
use crate::project::cache::blobs::{BlobStore, decode_mesh_payload};
use crate::project::cache::contract::SceneCacheOccurrence;
use crate::project::cache::derived_lod::{
    self, DerivedLodManifest, MAX_CANDIDATE_BYTES, read_scene_lods,
};
use crate::viewport::scene::{SceneAnchorIndex, SectionBoxState, SelectedTargets};
use crate::viewport::stage::{
    SceneCacheOwnershipContext, SceneCachePresentation, VerifiedSceneVisualInputs,
};

use super::ResidencyAuthority;
use super::visual_policy::is_ancestor_of;

/// Retained full-resolution geometry for exact hit testing.
#[derive(Component)]
pub(crate) struct GeneratedLodSource(pub(crate) Handle<Mesh>);

const MAX_ACTIVE: usize = 16;
const ENTER_PIXELS: f64 = 52.0;
const EXIT_PIXELS: f64 = 72.0;

#[derive(Debug, Default)]
pub(crate) struct GeneratedLodState {
    identity: Option<((u64, u64), usd_bevy::live::LiveRevision, SceneId, u64)>,
    candidates: BTreeMap<String, (Bounds3, DerivedLodManifest)>,
    active: BTreeMap<String, ActiveLod>,
    last_attempt: Option<String>,
}

#[derive(Debug)]
struct ActiveLod {
    entity: Entity,
    full: Handle<Mesh>,
    derived: Handle<Mesh>,
    bytes: (u64, u64),
}

fn restore(world: &mut World, entry: ActiveLod) {
    if let Some(mut mesh) = world.get_mut::<Mesh3d>(entry.entity)
        && mesh.0 == entry.derived
    {
        mesh.0 = entry.full;
    }
    if let Ok(mut entity) = world.get_entity_mut(entry.entity) {
        entity.remove::<GeneratedLodSource>();
    }
    let _ = world
        .resource_mut::<Assets<Mesh>>()
        .remove(entry.derived.id());
    world
        .resource_mut::<ResidencyAuthority>()
        .release_generated(entry.bytes);
}

pub(super) fn clear_generated_lods(world: &mut World) {
    let Some(mut authority) = world.get_resource_mut::<ResidencyAuthority>() else {
        return;
    };
    if authority.generated_lods.identity.is_none() {
        return;
    }
    let state = std::mem::take(&mut authority.generated_lods);
    for entry in state.active.into_values() {
        restore(world, entry);
    }
}

pub(super) fn synchronize_generated_lod(world: &mut World) {
    if !derived_lod::enabled() {
        clear_generated_lods(world);
        return;
    }
    let current = (|| {
        if !super::visual_projection_settled(
            world.get_resource::<usd_bevy::ProgressiveProjectionState>(),
        ) {
            return None;
        }
        let stage = world.get_non_send::<LiveStage>()?;
        let presentation = world.get_resource::<SceneCachePresentation>()?;
        let ownership = world.get_resource::<SceneCacheOwnershipContext>()?;
        let verified = world.get_resource::<VerifiedSceneVisualInputs>()?;
        (verified.matches_authoring(presentation, stage)
            && presentation.scene_id == ownership.scene_id
            && !stage.has_changes()
            && !stage.is_authoring_frozen())
        .then(|| {
            (
                (
                    stage.stage_identity(),
                    stage.authoring_generation(),
                    presentation.scene_id,
                    presentation.generation,
                ),
                ownership.project_root.clone(),
                stage.stage_orientation().inverse(),
            )
        })
    })();
    let Some((identity, project_root, orientation)) = current else {
        clear_generated_lods(world);
        return;
    };
    let mut state = std::mem::take(&mut world.resource_mut::<ResidencyAuthority>().generated_lods);
    if state.identity != Some(identity) {
        for entry in std::mem::take(&mut state.active).into_values() {
            restore(world, entry);
        }
        state.candidates.clear();
        state.last_attempt = None;
        state.identity = Some(identity);
        if let Some(mut catalogue) = read_scene_lods(&project_root, identity.2, identity.3) {
            let presentation = world.resource::<SceneCachePresentation>();
            for payload in super::spatial::scene_payloads(&presentation.entries) {
                let SceneCacheOccurrence::PrimPath(path) = payload.address.occurrence else {
                    continue;
                };
                let Some(candidate) = catalogue.candidates.remove(&path) else {
                    continue;
                };
                if candidate.source_anchor == path
                    && candidate.source_blob.0 == payload.payload_key.blob_hash.to_string()
                    && candidate.source_digest == candidate.source_blob.0
                    && candidate.derived_bytes <= MAX_CANDIDATE_BYTES
                {
                    state.candidates.insert(path, (payload.bounds, candidate));
                }
            }
        }
    }
    update_candidates(world, &mut state, &project_root, identity.2, orientation);
    world.resource_mut::<ResidencyAuthority>().generated_lods = state;
}

fn update_candidates(
    world: &mut World,
    state: &mut GeneratedLodState,
    project_root: &std::path::Path,
    scene: SceneId,
    orientation: Quat,
) {
    if state.candidates.is_empty() {
        return;
    }
    let camera = world
        .query_filtered::<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>()
        .single(world)
        .ok()
        .and_then(|(camera, transform, projection)| {
            Some((
                camera.physical_viewport_size()?.y,
                transform.translation(),
                transform.forward().as_vec3(),
                projection.get_clip_from_view().y_axis.y.abs(),
                matches!(projection, Projection::Orthographic(_)),
            ))
        });
    let clipped = world
        .get_resource::<SectionBoxState>()
        .is_some_and(|section| section.enabled);
    let mut first = None;
    let mut next = None;
    for (path, (bounds, manifest)) in &state.candidates {
        let exact = clipped
            || world
                .get_resource::<SelectedTargets>()
                .is_some_and(|selection| {
                    selection.0.targets.iter().any(|target| {
                        target.prim_path == *path
                            || is_ancestor_of(path, &target.prim_path)
                            || is_ancestor_of(&target.prim_path, path)
                    })
                });
        let active = state.active.get(path);
        let keep = !exact
            && world
                .resource::<ResidencyAuthority>()
                .can_admit_generated((0, 0))
            && camera.is_some_and(|(height, position, forward, scale, orthographic)| {
                let min = DVec3::from_array(bounds.min);
                let max = DVec3::from_array(bounds.max);
                let radius = (max - min).length() * 0.5;
                let delta = (min + max) * 0.5 - (orientation * position).as_dvec3();
                let distance = (delta.length() - radius).max(0.001);
                let diameter = radius * f64::from(scale) * f64::from(height)
                    / if orthographic { 1.0 } else { distance };
                delta.dot((orientation * forward).as_dvec3()) >= -radius
                    && diameter
                        <= if active.is_some() {
                            EXIT_PIXELS
                        } else {
                            ENTER_PIXELS
                        }
            });
        if let Some(active) = active {
            if !keep
                || world
                    .get::<Mesh3d>(active.entity)
                    .is_none_or(|mesh| mesh.0 != active.derived)
            {
                restore(
                    world,
                    state.active.remove(path).expect("existing active LOD"),
                );
            }
        } else if keep {
            first.get_or_insert((path, manifest));
            if state.last_attempt.as_ref().is_none_or(|last| path > last) {
                next.get_or_insert((path, manifest));
            }
        }
    }
    if state.active.len() < MAX_ACTIVE
        && let Some((path, manifest)) = next.or(first)
    {
        state.last_attempt = Some(path.clone());
        if let Some(active) = activate(world, project_root, scene, path, manifest) {
            bevy::log::debug!(target: "viewport::visual_residency", path, "activated generated visual LOD");
            state.active.insert(path.clone(), active);
        }
    }
}

fn activate(
    world: &mut World,
    project_root: &std::path::Path,
    scene: SceneId,
    path: &str,
    manifest: &DerivedLodManifest,
) -> Option<ActiveLod> {
    let live = world.get_non_send::<LiveStage>()?;
    if !derived_lod::source_eligible(&live.stage.prim(path).ok()?).ok()? {
        return None;
    }
    let index = world.get_resource::<SceneAnchorIndex>()?;
    let [entity] = index.resolve_all_by_prim_path(path) else {
        return None;
    };
    let entity = *entity;
    let full = world.get::<Mesh3d>(entity)?.0.clone();
    let authoritative = world.resource::<Assets<Mesh>>().get(&full)?;
    let encoded = crate::project::cache::blobs::prepare_mesh_payload(authoritative).ok()?;
    if encoded.blob_id != manifest.source_blob {
        return None;
    }
    let bytes = ResidencyAuthority::conservative_resident_footprint(manifest.derived_bytes as u64);
    if !world
        .resource::<ResidencyAuthority>()
        .can_admit_generated(bytes)
    {
        return None;
    }
    let store = SceneCacheStore::new(project_root)
        .object_store(scene)
        .ok()?;
    let blob = store
        .get_bounded(&manifest.visual_blob, manifest.derived_bytes)
        .ok()??;
    if blob.len() != manifest.derived_bytes
        || blake3::hash(&blob).to_hex().as_str() != manifest.visual_blob.0
    {
        return None;
    }
    let mesh = decode_mesh_payload(&blob).ok()?;
    let derived = world.resource_mut::<Assets<Mesh>>().add(mesh);
    world
        .entity_mut(entity)
        .insert((Mesh3d(derived.clone()), GeneratedLodSource(full.clone())));
    world
        .resource_mut::<ResidencyAuthority>()
        .reserve_generated(bytes);
    Some(ActiveLod {
        entity,
        full,
        derived,
        bytes,
    })
}
