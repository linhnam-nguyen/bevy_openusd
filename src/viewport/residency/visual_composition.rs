use std::collections::{BTreeMap, BTreeSet};

use bevy::camera::{CameraProjection, Projection};
use bevy::math::DVec3;
use bevy::prelude::*;
use usd_bevy::{LiveStage, ProgressiveProjectionState};
use usd_model::Bounds3;

use crate::project::cache::contract::{SceneCacheOccurrence, SceneCacheState};
use crate::viewport::camera::CameraMount;
use crate::viewport::scene::{SceneAnchorIndex, SelectedTargets};
use crate::viewport::stage::{SceneCachePresentation, VerifiedSceneVisualInputs};

use super::visual_policy::{VisualCandidate, VisualPolicy, VisualPolicyConfig, is_ancestor_of};
use viewport_protocol::{RuntimeMutation, ViewportCommand};

#[derive(Debug, Default)]
pub(crate) struct VisualCompositionState {
    identity: Option<((u64, u64), usd_bevy::live::LiveRevision, u64)>,
    owners: BTreeMap<String, (Option<Bounds3>, u64)>,
    loaded: BTreeSet<String>,
    camera_unloaded: BTreeSet<String>,
    policy: VisualPolicy,
    /// A failed batch disables optional camera unloading for this Stage generation.
    recovering: bool,
    suspended_after_recovery: bool,
    recovery_delay: u16,
    recovery_failures: u8,
}

#[derive(Resource)]
pub(crate) struct VisualStreaming(pub(crate) bool);

impl Default for VisualStreaming {
    fn default() -> Self {
        Self(std::env::var_os("NEMETON_DISABLE_VISUAL_STREAMING").is_none())
    }
}

/// Command-intent pinning is deliberately separate from authored USD and from
/// the existing CPU/GPU residency authority. A pin lives until the relevant
/// visual payload owners are composed and observed stable twice.
#[derive(Resource, Debug, Default)]
pub(crate) struct EditPayloadPins {
    stage_identity: Option<(u64, u64)>,
    paths: BTreeSet<String>,
    global_barrier: bool,
    settled_observations: u8,
}

#[derive(Debug)]
pub(crate) enum EditPinRequest {
    Paths(Vec<String>),
    Global,
}

impl EditPayloadPins {
    pub(crate) fn command_intent(command: &ViewportCommand) -> Option<EditPinRequest> {
        use ViewportCommand as C;
        match command {
            C::DefinePrim { path, .. } | C::RemovePrim { path } | C::RenamePrim { path, .. } => {
                Some(EditPinRequest::Paths(vec![path.clone()]))
            }
            C::ReparentPrim { path, new_parent } => Some(EditPinRequest::Paths(vec![
                path.clone(),
                new_parent.clone(),
            ])),
            C::MovePrim { old_path, new_path } => Some(EditPinRequest::Paths(vec![
                old_path.clone(),
                new_path.clone(),
            ])),
            C::SetAttribute { prim_path, .. }
            | C::ClearAttribute { prim_path, .. }
            | C::SetTransform { prim_path, .. } => {
                Some(EditPinRequest::Paths(vec![prim_path.clone()]))
            }
            C::EditBimProperty { mutation } => Some(EditPinRequest::Paths(vec![
                mutation.target.prim_path.clone(),
            ])),
            C::EditBimProperties { mutations, .. } | C::ApplyBimReplacementBatch { mutations } => {
                Some(EditPinRequest::Paths(
                    mutations
                        .iter()
                        .map(|m| m.target.prim_path.clone())
                        .collect(),
                ))
            }
            C::ApplyRuntimeMutationBatch { batch } => {
                let mut paths = Vec::new();
                for operation in &batch.operations {
                    match operation {
                        RuntimeMutation::DefinePrim { path, .. }
                        | RuntimeMutation::RemovePrim { path }
                        | RuntimeMutation::RenamePrim { path, .. } => paths.push(path.clone()),
                        RuntimeMutation::ReparentPrim { path, new_parent } => {
                            paths.extend([path.clone(), new_parent.clone()])
                        }
                        RuntimeMutation::MovePrim { old_path, new_path } => {
                            paths.extend([old_path.clone(), new_path.clone()]);
                        }
                        RuntimeMutation::SetAttribute { prim_path, .. }
                        | RuntimeMutation::ClearAttribute { prim_path, .. }
                        | RuntimeMutation::SetTransform { prim_path, .. }
                        | RuntimeMutation::SetVariantSelection { prim_path, .. } => {
                            paths.push(prim_path.clone())
                        }
                    }
                }
                Some(EditPinRequest::Paths(paths))
            }
            // Undo/Redo have no path in their protocol; protect all already
            // loaded owners from eviction through the next stable projection.
            C::UndoEditor | C::RedoEditor => Some(EditPinRequest::Global),
            _ => None,
        }
    }

    pub(crate) fn record(&mut self, stage_identity: (u64, u64), request: EditPinRequest) {
        if self.stage_identity != Some(stage_identity) {
            *self = Self {
                stage_identity: Some(stage_identity),
                ..Default::default()
            };
        }
        match request {
            EditPinRequest::Paths(paths) => {
                self.paths.extend(
                    paths
                        .into_iter()
                        .filter(|path| path.starts_with('/') && !path.contains('{')),
                );
            }
            EditPinRequest::Global => self.global_barrier = true,
        }
        self.settled_observations = 0;
    }

    pub(crate) fn record_path(&mut self, identity: (u64, u64), path: String) {
        self.record(identity, EditPinRequest::Paths(vec![path]));
    }

    fn reset_for_stage(&mut self, identity: (u64, u64)) {
        if self.stage_identity.is_some() && self.stage_identity != Some(identity) {
            *self = Self::default();
        }
    }

    pub(super) fn editing(&self, owner: &str, already_loaded: bool) -> bool {
        (self.global_barrier && already_loaded)
            || self.paths.iter().any(|path| {
                owner == path || is_ancestor_of(owner, path) || is_ancestor_of(path, owner)
            })
    }

    pub(super) fn settle(
        &mut self,
        owners: &BTreeMap<String, (Option<Bounds3>, u64)>,
        loaded: &BTreeSet<String>,
    ) {
        if self.paths.is_empty() && !self.global_barrier {
            return;
        }
        let ready = self.paths.iter().all(|path| {
            owners
                .keys()
                .filter(|owner| {
                    *owner == path || is_ancestor_of(owner, path) || is_ancestor_of(path, owner)
                })
                .all(|owner| loaded.contains(owner))
        });
        if !ready {
            self.settled_observations = 0;
            return;
        }
        self.settled_observations = self.settled_observations.saturating_add(1);
        if self.settled_observations >= 2 {
            self.paths.clear();
            self.global_barrier = false;
            self.settled_observations = 0;
        }
    }
}

#[derive(Resource, Debug, Default)]
pub(crate) struct VisualStreamingHealth {
    pub(crate) incomplete: bool,
    pub(crate) last_error: Option<String>,
    pub(crate) lod_error: Option<String>,
    pub(crate) awaiting_payloads: bool,
}

fn mark_recovery_failure(
    state: &mut VisualCompositionState,
    health: &mut VisualStreamingHealth,
    error: String,
) {
    state.recovery_failures = state.recovery_failures.saturating_add(1);
    state.recovery_delay = (1u16 << state.recovery_failures.min(7)).min(120);
    health.incomplete = true;
    bevy::log::error!("{error}");
    health.last_error = Some(error);
}

pub(super) fn restore_visual_owners(
    state: &mut VisualCompositionState,
    health: &mut VisualStreamingHealth,
    wanted: &BTreeSet<String>,
    read: impl FnOnce() -> anyhow::Result<Vec<String>>,
    apply: impl FnOnce(&[String]) -> anyhow::Result<()>,
) -> bool {
    if wanted.is_empty() {
        return true;
    }
    if state.recovery_delay > 0 {
        state.recovery_delay -= 1;
        return false;
    }
    match read() {
        Ok(actual) => state.loaded = actual.into_iter().collect(),
        Err(error) => {
            mark_recovery_failure(
                state,
                health,
                format!("Could not read visual payloads: {error:#}"),
            );
            return false;
        }
    }
    state
        .camera_unloaded
        .retain(|path| !state.loaded.contains(path));
    // Ordered paths put ancestors before their descendants.
    let batch: Vec<_> = wanted.difference(&state.loaded).take(8).cloned().collect();
    if batch.is_empty() {
        state.recovery_failures = 0;
        return true;
    }
    if let Err(error) = apply(&batch) {
        mark_recovery_failure(
            state,
            health,
            format!("Could not restore visual payloads {batch:?}: {error:#}"),
        );
    } else {
        state.recovery_failures = 0;
    }
    false
}

pub(super) fn synchronize_visual_composition(
    stage: Option<NonSend<LiveStage>>,
    progressive: Option<Res<ProgressiveProjectionState>>,
    presentation: Option<Res<SceneCachePresentation>>,
    verified: Option<Res<VerifiedSceneVisualInputs>>,
    selected: Option<Res<SelectedTargets>>,
    mount: Option<Res<CameraMount>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    enabled: Res<VisualStreaming>,
    mut edit_pins: ResMut<EditPayloadPins>,
    mut health: ResMut<VisualStreamingHealth>,
    mut authority: ResMut<super::ResidencyAuthority>,
    mut scene_index: Option<ResMut<SceneAnchorIndex>>,
) {
    let state = &mut authority.visual_composition;
    let Some(stage) = stage else {
        *state = VisualCompositionState::default();
        *edit_pins = EditPayloadPins::default();
        *health = VisualStreamingHealth::default();
        return;
    };
    edit_pins.reset_for_stage(stage.stage_identity());
    health.awaiting_payloads = state
        .camera_unloaded
        .iter()
        .any(|path| edit_pins.editing(path, false));
    if stage.has_changes()
        || stage.is_authoring_frozen()
        || !super::visual_projection_settled(progressive.as_deref())
    {
        return;
    }
    if state
        .identity
        .is_some_and(|(prior, _, _)| prior != stage.stage_identity())
    {
        *state = VisualCompositionState::default();
        health.incomplete = false;
        health.last_error = None;
    }
    let ready = presentation.as_deref().filter(|cache| {
        verified
            .as_ref()
            .is_some_and(|proof| proof.matches(cache, &stage))
    });
    let identity = ready.map(|cache| {
        (
            stage.stage_identity(),
            stage.authoring_generation(),
            cache.generation,
        )
    });
    let retiring = ready.is_none() || !enabled.0 || stage.has_manual_visual_payloads();
    let changing = state.identity != identity;
    if (state.recovering || retiring || changing) && state.recovery_delay > 0 {
        state.recovery_delay -= 1;
        return;
    }
    if state.recovering || retiring || changing {
        let discovered =
            if changing && state.identity.is_some() && !state.camera_unloaded.is_empty() {
                match stage.visual_loadable_paths() {
                    Ok(paths) => Some(paths.into_iter().collect::<BTreeSet<_>>()),
                    Err(error) => {
                        mark_recovery_failure(
                            state,
                            &mut health,
                            format!("Visual payload rediscovery failed: {error:#}"),
                        );
                        return;
                    }
                }
            } else {
                None
            };
        let wanted: BTreeSet<_> = if state.recovering {
            state.owners.keys().cloned().collect()
        } else {
            state.camera_unloaded.clone()
        };
        let wanted: BTreeSet<_> = wanted
            .into_iter()
            .filter(|path| {
                !stage.has_manual_visual_payload_overlap(path)
                    && discovered.as_ref().is_none_or(|paths| paths.contains(path))
            })
            .collect();
        health.awaiting_payloads |= !wanted.is_empty();
        if !restore_visual_owners(
            state,
            &mut health,
            &wanted,
            || stage.visual_loaded_paths(),
            |load| stage.apply_visual_load_batch(load, &[]),
        ) {
            return;
        }
        state.camera_unloaded.clear();
        health.awaiting_payloads = false;
        if let Some(index) = scene_index.as_deref_mut() {
            index.clear_visual_unload_rows();
        }
        if retiring || !changing {
            health.incomplete = false;
            health.last_error = None;
        }
        if state.recovering {
            state.recovering = false;
            state.suspended_after_recovery = true;
        }
        if retiring {
            *state = VisualCompositionState::default();
            return;
        }
    }
    let Some(presentation) = ready else { return };
    if changing {
        if state.recovery_delay > 0 {
            state.recovery_delay -= 1;
            return;
        }
        let (paths, loaded) = match stage
            .visual_loadable_paths()
            .and_then(|paths| stage.visual_loaded_paths().map(|loaded| (paths, loaded)))
        {
            Ok(paths) => paths,
            Err(error) => {
                mark_recovery_failure(
                    state,
                    &mut health,
                    format!("Could not discover visual payloads: {error:#}"),
                );
                return;
            }
        };
        *state = VisualCompositionState {
            identity,
            ..Default::default()
        };
        health.incomplete = false;
        health.last_error = None;
        state.owners = paths.into_iter().map(|path| (path, (None, 0))).collect();
        state.loaded = loaded.into_iter().collect();
        for (path, (bounds, _)) in &mut state.owners {
            if let Ok(Some((min, max))) = stage.visual_payload_hint(path) {
                *bounds = Some(Bounds3 { min, max });
            }
        }
        for payload in super::spatial::scene_payloads(&presentation.entries) {
            let SceneCacheOccurrence::PrimPath(path) = &payload.address.occurrence else {
                continue;
            };
            let mut prefix = path.as_str();
            loop {
                if let Some((bounds, bytes)) = state.owners.get_mut(prefix) {
                    *bytes = bytes.saturating_add(payload.cpu_bytes);
                    // Partial index extents cannot prove whole-payload coverage.
                    // The validated native model hint is the only bound for
                    // automatic eviction in that case.
                    let trusted_cache_bound = presentation.state == SceneCacheState::Ready
                        && verified
                            .as_ref()
                            .is_some_and(|proof| proof.matches_authoring(presentation, &stage));
                    let next = payload.bounds;
                    if trusted_cache_bound
                        && next.min.into_iter().chain(next.max).all(f64::is_finite)
                    {
                        *bounds = Some(match bounds {
                            Some(current) => Bounds3 {
                                min: std::array::from_fn(|axis| {
                                    current.min[axis].min(next.min[axis])
                                }),
                                max: std::array::from_fn(|axis| {
                                    current.max[axis].max(next.max[axis])
                                }),
                            },
                            None => next,
                        });
                    }
                }
                let Some((parent, _)) = prefix.rsplit_once('/') else {
                    break;
                };
                prefix = parent;
            }
        }
    }
    if state.owners.is_empty() {
        return;
    }

    if state.suspended_after_recovery {
        // Retain the full visual load set until a new scene generation.
        edit_pins.settle(&state.owners, &state.loaded);
        return;
    }
    let Ok((camera, transform, projection)) = cameras.single() else {
        return;
    };
    let Some(viewport) = camera.physical_viewport_size() else {
        return;
    };
    let to_stage = stage.stage_orientation().inverse();
    let position = (to_stage * transform.translation()).as_dvec3();
    let forward = (to_stage * transform.forward().as_vec3()).as_dvec3();
    let y_scale = f64::from(projection.get_clip_from_view().y_axis.y.abs());
    let paths: Vec<_> = state.owners.keys().cloned().collect();
    let candidates: Vec<_> = state.owners.iter().map(|(path, (bounds, bytes))| {
        let metric = bounds.as_ref().filter(|_| y_scale.is_finite() && y_scale > 0.0).map(|bounds| {
            let min = DVec3::from_array(bounds.min);
            let max = DVec3::from_array(bounds.max);
            let radius = (max - min).length() * 0.5;
            let delta = (min + max) * 0.5 - position;
            let distance = (delta.length() - radius).max(0.001);
            let scale = match projection {
                Projection::Orthographic(_) => y_scale,
                _ => y_scale / distance,
            };
            (radius * scale * f64::from(viewport.y), distance, delta.dot(forward) >= -radius)
        });
        let contains = |target: &str| path == target || is_ancestor_of(path, target);
        VisualCandidate {
            path: path.clone(),
            bound_known: metric.is_some(),
            visible: metric.is_none_or(|(_, _, visible)| visible),
            selected: selected.as_ref().is_some_and(|selection| selection.0.targets.iter().any(|anchor| contains(&anchor.prim_path))),
            camera_mount: mount.as_deref().is_some_and(|mount| matches!(mount, CameraMount::Mounted { prim_path } if contains(prim_path))),
            editing: edit_pins.editing(path, state.loaded.contains(path)),
            projected_pixels: metric.map(|(pixels, _, _)| pixels),
            distance: metric.map_or(f64::INFINITY, |(_, distance, _)| distance),
            estimated_bytes: *bytes,
        }
    }).collect();
    let plan = state.policy.plan(
        VisualPolicyConfig::default(),
        &paths,
        &state.loaded,
        &candidates,
    );
    if plan.load.is_empty() && plan.unload.is_empty() {
        edit_pins.settle(&state.owners, &state.loaded);
        return;
    }
    match stage.apply_visual_load_batch(&plan.load, &plan.unload) {
        Ok(()) => {
            if let Some(index) = scene_index.as_deref_mut() {
                index.retain_visual_unload_rows(&plan.unload);
            }
            state.camera_unloaded.extend(plan.unload.iter().cloned());
            // The native load set is the only truth after a batch. A missing
            // readback is NOT a successful visual readiness acknowledgement.
            let actual = match stage.visual_loaded_paths() {
                Ok(paths) => paths.into_iter().collect::<BTreeSet<_>>(),
                Err(error) => {
                    health.incomplete = true;
                    health.last_error = Some(format!("visual batch readback failed: {error:#}"));
                    state.recovering = true;
                    return;
                }
            };
            if !plan.load.iter().all(|path| actual.contains(path)) {
                health.incomplete = true;
                health.last_error =
                    Some("native visual batch did not load every requested owner".to_owned());
                state.loaded = actual;
                state.recovering = true;
                return;
            }
            state.camera_unloaded.retain(|path| !actual.contains(path));
            bevy::log::debug!(
                target: "viewport::visual_residency",
                scene_generation = presentation.generation,
                load = ?plan.load,
                unload = ?plan.unload,
                loaded = actual.len(),
                "applied camera visual payload batch"
            );
            state.loaded = actual;
            edit_pins.settle(&state.owners, &state.loaded);
        }
        Err(error) => {
            // A failed optional unload or mandatory load must never poison
            // future required loads. Next ticks attempt conservative recovery.
            health.incomplete = true;
            health.last_error = Some(format!("visual payload batch failed: {error:#}"));
            bevy::log::error!("{}", health.last_error.as_deref().unwrap_or_default());
            state.recovering = true;
            state.recovery_failures = 0;
            state.recovery_delay = 1;
        }
    }
}
