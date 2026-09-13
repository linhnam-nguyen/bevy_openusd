//! Bounded animation lookahead for Scene-owned residency.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use usd_project::SceneId;

use crate::project::cache_contract::SceneCacheEntryKind;
use crate::viewport::animation::UsdStageTime;
use crate::viewport::session::SceneCachePresentation;

use super::{ResidencyAuthority, ResidencyReason, ScenePayloadCatalog, ScenePayloadKey};

pub(crate) const ANIMATION_LOOKAHEAD: usize = 64;

#[derive(Clone, Copy, Debug)]
struct AnimationPayload {
    key: ScenePayloadKey,
    generation: u64,
    cpu_bytes: u64,
    gpu_bytes: u64,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct AnimationResidencyState {
    scene_id: Option<SceneId>,
    generation: Option<u64>,
    candidates: Vec<AnimationPayload>,
    active: HashMap<ScenePayloadKey, u64>,
    cursor: usize,
}

pub(crate) fn sync_animation_residency(
    presentation: Option<Res<SceneCachePresentation>>,
    catalog: Res<ScenePayloadCatalog>,
    clock: Option<Res<UsdStageTime>>,
    mut authority: ResMut<ResidencyAuthority>,
    mut state: ResMut<AnimationResidencyState>,
) {
    let Some(presentation) = presentation else {
        state.release(&mut authority);
        state.reset();
        return;
    };
    let scene_changed = state.scene_id != Some(presentation.scene_id)
        || state.generation != Some(presentation.generation)
        || presentation.is_changed();
    if scene_changed {
        state.release(&mut authority);
        let catalog = (catalog.scene_id == Some(presentation.scene_id)
            && catalog.generation == Some(presentation.generation))
            .then_some(catalog.as_ref());
        state.rebuild(&presentation, catalog);
        authority.register_scene_generation(presentation.scene_id, presentation.generation);
    }

    if !clock.is_some_and(|clock| clock.playing) {
        state.release(&mut authority);
        return;
    }
    state.advance(&mut authority);
}

impl AnimationResidencyState {
    fn rebuild(
        &mut self,
        presentation: &SceneCachePresentation,
        catalog: Option<&ScenePayloadCatalog>,
    ) {
        self.scene_id = Some(presentation.scene_id);
        self.generation = Some(presentation.generation);
        self.cursor = 0;
        let Some(catalog) = catalog.filter(|catalog| {
            catalog.scene_id == Some(presentation.scene_id)
                && catalog.generation == Some(presentation.generation)
        }) else {
            self.candidates.clear();
            return;
        };
        self.candidates = presentation
            .entries
            .iter()
            .filter_map(|entry| {
                if !entry.cacheable || !matches!(&entry.kind, SceneCacheEntryKind::OwnedPrim { .. })
                {
                    return None;
                }
                let _animation = entry.animation.as_ref()?;
                if entry.address.scene_id != presentation.scene_id {
                    return None;
                }
                let geometry = entry.geometry.as_ref()?;
                let blob_hash = entry.content_hash?;
                let key = ScenePayloadKey {
                    scene_id: entry.address.scene_id,
                    blob_hash,
                };
                if !catalog.descriptors_for(key).iter().any(|descriptor| {
                    descriptor.address == entry.address && descriptor.animation.is_some()
                }) {
                    return None;
                }
                let resident_bytes = geometry.byte_size.saturating_add(_animation.byte_size);
                Some(AnimationPayload {
                    key,
                    generation: presentation.generation,
                    cpu_bytes: ResidencyAuthority::conservative_resident_footprint(resident_bytes)
                        .0,
                    gpu_bytes: ResidencyAuthority::conservative_resident_footprint(resident_bytes)
                        .1,
                })
            })
            .scan(HashSet::new(), |seen, candidate| {
                seen.insert(candidate.key).then_some(candidate)
            })
            .collect();
        self.candidates.sort_by_key(|candidate| candidate.key);
    }

    fn advance(&mut self, authority: &mut ResidencyAuthority) {
        if self.candidates.is_empty() {
            self.release(authority);
            return;
        }
        let window = ANIMATION_LOOKAHEAD.min(self.candidates.len());
        let desired = (0..window)
            .map(|offset| self.candidates[(self.cursor + offset) % self.candidates.len()])
            .collect::<Vec<_>>();
        let desired_keys = desired
            .iter()
            .map(|candidate| candidate.key)
            .collect::<HashSet<_>>();
        let stale = self
            .active
            .keys()
            .filter(|key| !desired_keys.contains(key))
            .copied()
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(generation) = self.active.remove(&key) {
                let _ =
                    authority.remove_reason(key, ResidencyReason::AnimationRequired, generation);
            }
        }
        for candidate in desired {
            if self.active.contains_key(&candidate.key) {
                continue;
            }
            let _ = authority.request_reason(
                candidate.key,
                ResidencyReason::AnimationRequired,
                candidate.generation,
                candidate.cpu_bytes,
                candidate.gpu_bytes,
            );
            if authority
                .reasons(&candidate.key)
                .is_some_and(|reasons| reasons.contains(&ResidencyReason::AnimationRequired))
            {
                self.active.insert(candidate.key, candidate.generation);
            }
        }
        self.cursor = (self.cursor + 1) % self.candidates.len();
    }

    fn release(&mut self, authority: &mut ResidencyAuthority) {
        for (key, generation) in self.active.drain() {
            let _ = authority.remove_reason(key, ResidencyReason::AnimationRequired, generation);
        }
    }

    fn reset(&mut self) {
        self.scene_id = None;
        self.generation = None;
        self.candidates.clear();
        self.cursor = 0;
    }
}

#[cfg(test)]
#[path = "animation_tests.rs"]
mod tests;
