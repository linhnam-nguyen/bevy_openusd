//! Bounded animation lookahead for Scene-owned residency.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use usd_project::SceneId;

use crate::project::cache_contract::SceneCacheEntryKind;
use crate::viewport::animation::UsdStageTime;
use crate::viewport::session::SceneCachePresentation;

use super::{ResidencyAuthority, ResidencyReason, ScenePayloadKey};

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
        state.rebuild(&presentation);
        authority.register_scene_generation(presentation.scene_id, presentation.generation);
    }

    if !clock.is_some_and(|clock| clock.playing) {
        state.release(&mut authority);
        return;
    }
    state.advance(&mut authority);
}

impl AnimationResidencyState {
    fn rebuild(&mut self, presentation: &SceneCachePresentation) {
        self.scene_id = Some(presentation.scene_id);
        self.generation = Some(presentation.generation);
        self.cursor = 0;
        self.candidates = presentation
            .entries
            .iter()
            .filter_map(|entry| {
                if !entry.cacheable || !matches!(&entry.kind, SceneCacheEntryKind::OwnedPrim { .. })
                {
                    return None;
                }
                let _animation = entry.animation.as_ref()?;
                let geometry = entry.geometry.as_ref()?;
                let blob_hash = entry.content_hash?;
                Some(AnimationPayload {
                    key: ScenePayloadKey {
                        scene_id: entry.address.scene_id,
                        blob_hash,
                    },
                    generation: presentation.generation,
                    cpu_bytes: ResidencyAuthority::conservative_resident_footprint(
                        geometry.byte_size,
                    )
                    .0,
                    gpu_bytes: ResidencyAuthority::conservative_resident_footprint(
                        geometry.byte_size,
                    )
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
mod tests {
    use super::*;
    use crate::project::cache_contract::{
        CachedTransform, SceneCacheAddress, SceneCacheBlobRef, SceneCacheEntry,
        SceneCacheEntryKind, SceneCacheOccurrence, SceneCacheState,
    };
    use usd_model::{BlobId, Bounds3, HashDigest};
    use usd_project::{SceneMemberId, ScenePlacementTransform};

    fn key(scene_id: SceneId, value: u8) -> ScenePayloadKey {
        ScenePayloadKey {
            scene_id,
            blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
        }
    }

    fn candidates(scene_id: SceneId, count: u8, generation: u64) -> Vec<AnimationPayload> {
        (0..count)
            .map(|value| AnimationPayload {
                key: key(scene_id, value),
                generation,
                cpu_bytes: 1,
                gpu_bytes: 1,
            })
            .collect()
    }

    fn animated_entry(
        scene_id: SceneId,
        value: u8,
        cacheable: bool,
        kind: SceneCacheEntryKind,
    ) -> SceneCacheEntry {
        let hash = key(scene_id, value).blob_hash;
        SceneCacheEntry {
            address: SceneCacheAddress {
                scene_id,
                occurrence: SceneCacheOccurrence::PrimPath(format!("/World/Node{value}")),
            },
            parent: None,
            transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
            bounds: Some(Bounds3 {
                min: [-1.0; 3],
                max: [1.0; 3],
            }),
            cacheable,
            bim_enabled: false,
            geometry: Some(SceneCacheBlobRef {
                blob_id: BlobId(hash.to_hex()),
                byte_size: 8,
            }),
            material: None,
            animation: Some(SceneCacheBlobRef {
                blob_id: BlobId(hash.to_hex()),
                byte_size: 8,
            }),
            semantic_key: None,
            kind,
            content_hash: Some(hash),
        }
    }

    #[test]
    fn rebuild_requires_cacheable_owned_prim_animation_rows() {
        let scene = SceneId::new_v4();
        let valid = animated_entry(
            scene,
            1,
            true,
            SceneCacheEntryKind::OwnedPrim {
                prim_path: "/World/Node1".to_owned(),
            },
        );
        let non_cacheable = animated_entry(
            scene,
            2,
            false,
            SceneCacheEntryKind::OwnedPrim {
                prim_path: "/World/Node2".to_owned(),
            },
        );
        let child_scene = animated_entry(
            scene,
            3,
            true,
            SceneCacheEntryKind::ChildScene {
                scene_id: SceneId::new_v4(),
                member_id: SceneMemberId::new_v4(),
            },
        );
        let presentation = SceneCachePresentation {
            scene_id: scene,
            generation: 7,
            state: SceneCacheState::Ready,
            entries: vec![valid, non_cacheable, child_scene],
        };
        let mut state = AnimationResidencyState::default();

        state.rebuild(&presentation);

        assert_eq!(state.candidates.len(), 1);
        assert_eq!(state.candidates[0].key, key(scene, 1));
    }

    #[test]
    fn animation_window_is_bounded_and_rotates_fairly() {
        let scene = SceneId::new_v4();
        let mut authority = ResidencyAuthority::default();
        authority.install_scene(scene, 7, Vec::new());
        let mut state = AnimationResidencyState {
            scene_id: Some(scene),
            generation: Some(7),
            candidates: candidates(scene, 96, 7),
            ..Default::default()
        };

        state.advance(&mut authority);
        assert_eq!(state.active.len(), ANIMATION_LOOKAHEAD);
        assert_eq!(authority.queue_len(), ANIMATION_LOOKAHEAD);
        state.advance(&mut authority);
        assert_eq!(state.active.len(), ANIMATION_LOOKAHEAD);
        assert!(state.active.contains_key(&key(scene, 64)));
        assert!(!state.active.contains_key(&key(scene, 95)));
    }

    #[test]
    fn stale_animation_generation_is_rejected_without_a_reason() {
        let scene = SceneId::new_v4();
        let mut authority = ResidencyAuthority::default();
        authority.install_scene(scene, 8, Vec::new());
        let mut state = AnimationResidencyState {
            scene_id: Some(scene),
            generation: Some(7),
            candidates: candidates(scene, 1, 7),
            ..Default::default()
        };

        state.advance(&mut authority);
        assert!(state.active.is_empty());
        assert!(authority.reasons(&key(scene, 0)).is_none());
    }
}
