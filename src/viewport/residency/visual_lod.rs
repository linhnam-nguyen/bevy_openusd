use std::collections::{BTreeMap, BTreeSet};

use bevy::camera::{CameraProjection, Projection};
use bevy::math::DVec3;
use bevy::prelude::*;
use usd_bevy::{LiveStage, ProgressiveProjectionState};
use usd_model::Bounds3;

use crate::project::cache::contract::SceneCacheOccurrence;
use crate::viewport::scene::{SectionBoxState, SelectedTargets};
use crate::viewport::stage::{SceneCachePresentation, VerifiedSceneVisualInputs};

use super::visual_policy::{LodChoice, LodThresholds, choose_authored_lod, is_ancestor_of};

#[derive(Debug, Default)]
pub(crate) struct AuthoredLodState {
    identity: Option<((u64, u64), usd_bevy::live::LiveRevision, u64)>,
    candidates: BTreeMap<String, Candidate>,
    pending: Vec<(String, Bounds3)>,
    reset_pending: bool,
    cleanup_retry_delay: u16,
    cleanup_failures: u8,
}

#[derive(Debug)]
struct Candidate {
    bounds: Bounds3,
    choices: Vec<LodChoice>,
    active_detail: Option<usize>,
    rejected: BTreeSet<usize>,
}

/// Clear every previous camera choice before discarding the ledger.
/// Failed clears remain active and will be retried; no cache-generation
/// rollover can silently preserve a stale visual session override.
fn clear_camera_opinions(stage: &LiveStage, state: &mut AuthoredLodState) -> bool {
    if state.cleanup_retry_delay > 0 {
        state.cleanup_retry_delay -= 1;
        return false;
    }
    let mut all_cleared = true;
    let mut attempted = 0;
    for (path, candidate) in &mut state.candidates {
        if candidate.active_detail.is_none() {
            continue;
        }
        if attempted == 8 {
            all_cleared = false;
            break;
        }
        attempted += 1;
        let set_name = &candidate.choices[0].set_name;
        let result = stage
            .select_visual_authored_lod(path, set_name, None)
            .and_then(|_| stage.visual_lod_matches_canonical(path, set_name));
        match result {
            Ok(true) => candidate.active_detail = None,
            Ok(false) => {
                all_cleared = false;
                bevy::log::warn!("visual LOD reset unconfirmed at {path}; keeping cleanup ledger");
            }
            Err(error) => {
                all_cleared = false;
                bevy::log::warn!("visual LOD reset failed at {path}: {error:#}; will retry");
            }
        }
    }
    if all_cleared {
        state.cleanup_failures = 0;
        state.cleanup_retry_delay = 0;
    } else {
        // Retry forever at a bounded low cadence rather than spinning at the
        // renderer frame rate. State remains explicitly cleanup-pending.
        state.cleanup_failures = state.cleanup_failures.saturating_add(1);
        state.cleanup_retry_delay = (1u16 << state.cleanup_failures.min(7)).min(120);
    }
    all_cleared
}

fn named_detail(name: &str) -> Option<usize> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "full" | "high" => Some(100),
        "medium" | "mid" => Some(50),
        "low" | "coarse" | "proxy" => Some(1),
        name => {
            let level = name.strip_prefix("lod")?.trim_start_matches(['_', '-']);
            100usize.checked_sub(level.parse().ok()?)
        }
    }
}

pub(super) fn synchronize_authored_lod(
    stage: Option<NonSend<LiveStage>>,
    progressive: Option<Res<ProgressiveProjectionState>>,
    presentation: Option<Res<SceneCachePresentation>>,
    verified: Option<Res<VerifiedSceneVisualInputs>>,
    selection: Option<Res<SelectedTargets>>,
    section: Option<Res<SectionBoxState>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    mut authority: ResMut<super::ResidencyAuthority>,
    mut health: ResMut<super::VisualStreamingHealth>,
) {
    let state = &mut authority.authored_lods;
    let Some(stage) = stage else {
        *state = AuthoredLodState::default();
        health.lod_error = None;
        return;
    };
    // Initial projection retains a precomputed entity plan across updates.
    // A variant switch structurally resyncs that plan's entities, so wait
    // for the same settled projection boundary as native payload streaming.
    if stage.has_changes()
        || stage.is_authoring_frozen()
        || !super::visual_projection_settled(progressive.as_deref())
    {
        return;
    }
    // A genuinely new Stage owns a new sealed session layer; it is safe to
    // forget old-stage bookkeeping. Cache-generation changes do NOT imply this.
    if state
        .identity
        .is_some_and(|(identity, _, _)| identity != stage.stage_identity())
    {
        *state = AuthoredLodState::default();
        health.lod_error = None;
    }
    let enabled = std::env::var_os("NEMETON_DISABLE_AUTHORED_LOD").is_none();
    let usable = presentation.as_deref().filter(|cache| {
        enabled
            && verified
                .as_ref()
                .is_some_and(|proof| proof.matches_authoring(cache, &stage))
    });
    let identity = usable.map(|cache| {
        (
            stage.stage_identity(),
            stage.authoring_generation(),
            cache.generation,
        )
    });
    if state.identity != identity || state.reset_pending {
        state.reset_pending = true;
        if !clear_camera_opinions(&stage, state) {
            health.lod_error = Some("Authored visual LOD cleanup is pending".to_owned());
            return;
        }
        health.lod_error = None;
        *state = AuthoredLodState {
            identity,
            ..Default::default()
        };
        let Some(cache) = usable else { return };
        let mut bounds = BTreeMap::<String, Bounds3>::new();
        for payload in super::spatial::scene_payloads(&cache.entries) {
            let SceneCacheOccurrence::PrimPath(path) = &payload.address.occurrence else {
                continue;
            };
            let mut path = path.as_str();
            while !path.is_empty() {
                let next = payload.bounds;
                bounds
                    .entry(path.to_owned())
                    .and_modify(|current| {
                        current.min =
                            std::array::from_fn(|axis| current.min[axis].min(next.min[axis]));
                        current.max =
                            std::array::from_fn(|axis| current.max[axis].max(next.max[axis]));
                    })
                    .or_insert(next);
                let Some((parent, _)) = path.rsplit_once('/') else {
                    break;
                };
                path = parent;
            }
        }
        *state = AuthoredLodState {
            identity,
            pending: bounds.into_iter().rev().collect(),
            ..Default::default()
        };
    }
    for _ in 0..64 {
        let Some((path, bounds)) = state.pending.pop() else {
            break;
        };
        let Ok(options) = stage.authored_lod_options(&path) else {
            continue;
        };
        let [(set, names)] = options.as_slice() else {
            continue;
        };
        let choices: Option<Vec<_>> = names
            .iter()
            .map(|name| {
                Some(LodChoice {
                    set_name: set.clone(),
                    variant_name: name.clone(),
                    detail: named_detail(name)?,
                })
            })
            .collect();
        if let Some(choices) = choices
            && choices
                .iter()
                .map(|choice| choice.detail)
                .collect::<BTreeSet<_>>()
                .len()
                == choices.len()
            && choices.len() >= 2
        {
            state.candidates.insert(
                path,
                Candidate {
                    bounds,
                    choices,
                    active_detail: None,
                    rejected: BTreeSet::new(),
                },
            );
        }
    }

    let Ok((camera, transform, projection)) = cameras.single() else {
        return;
    };
    let Some(viewport) = camera.physical_viewport_size() else {
        return;
    };
    let source_position =
        (stage.stage_orientation().inverse() * transform.translation()).as_dvec3();
    let scale = f64::from(projection.get_clip_from_view().y_axis.y.abs());
    for (path, candidate) in &mut state.candidates {
        let min = DVec3::from_array(candidate.bounds.min);
        let max = DVec3::from_array(candidate.bounds.max);
        let radius = (max - min).length() * 0.5;
        let distance = ((min + max) * 0.5 - source_position).length() - radius;
        let pixels = radius * scale * f64::from(viewport.y)
            / if matches!(projection, Projection::Orthographic(_)) {
                1.0
            } else {
                distance.max(0.001)
            };
        let exact = section.as_deref().is_some_and(|section| section.enabled)
            || selection.as_ref().is_some_and(|selection| {
                selection.0.targets.iter().any(|target| {
                    target.prim_path == *path
                        || is_ancestor_of(path, &target.prim_path)
                        || is_ancestor_of(&target.prim_path, path)
                })
            });
        let choice = (!exact)
            .then(|| {
                choose_authored_lod(
                    &candidate.choices,
                    pixels,
                    candidate.active_detail,
                    false,
                    LodThresholds::default(),
                )
            })
            .flatten()
            .filter(|choice| !candidate.rejected.contains(&choice.detail));
        let detail = choice.as_ref().map(|choice| choice.detail);
        if detail == candidate.active_detail {
            continue;
        }
        match stage.select_visual_authored_lod(
            path,
            &candidate.choices[0].set_name,
            choice.as_ref().map(|choice| choice.variant_name.as_str()),
        ) {
            Ok(true) => {
                candidate.active_detail = detail;
                bevy::log::debug!(target: "viewport::visual_residency", path, ?detail, "updated authored visual LOD");
                break;
            }
            Ok(false) => {
                if detail.is_none() {
                    candidate.active_detail = None;
                }
            }
            Err(error) => {
                if let Some(detail) = detail {
                    candidate.rejected.insert(detail);
                    if candidate.active_detail.is_some() {
                        state.reset_pending = true;
                    }
                } else {
                    state.reset_pending = true;
                    health.lod_error = Some(format!(
                        "Authored visual LOD reset failed at {path}: {error:#}"
                    ));
                    break;
                }
                bevy::log::warn!("visual LOD candidate rejected at {path}: {error:#}");
            }
        }
    }
}
