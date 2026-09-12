//! Active authored-camera residency demand.

use bevy::prelude::*;
use usd_project::SceneId;

use crate::project::cache_contract::SceneCacheEntryKind;
use crate::viewport::camera::CameraMount;
use crate::viewport::session::SceneCachePresentation;

use super::{ResidencyAuthority, ResidencyReason, ScenePayloadKey};

#[derive(Clone, Copy, Debug)]
struct ViewpointPayload {
    key: ScenePayloadKey,
    generation: u64,
    cpu_bytes: u64,
    gpu_bytes: u64,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct ActiveViewpointResidencyState {
    scene_id: Option<SceneId>,
    generation: Option<u64>,
    active: Option<ViewpointPayload>,
}

pub(crate) fn sync_active_viewpoint_residency(
    presentation: Option<Res<SceneCachePresentation>>,
    mount: Option<Res<CameraMount>>,
    mut authority: ResMut<ResidencyAuthority>,
    mut state: ResMut<ActiveViewpointResidencyState>,
) {
    let Some(presentation) = presentation else {
        state.release(&mut authority);
        state.reset();
        return;
    };
    if state.scene_id != Some(presentation.scene_id)
        || state.generation != Some(presentation.generation)
        || presentation.is_changed()
    {
        state.release(&mut authority);
        state.scene_id = Some(presentation.scene_id);
        state.generation = Some(presentation.generation);
        authority.register_scene_generation(presentation.scene_id, presentation.generation);
    }
    let desired = mount
        .as_deref()
        .and_then(|mount| match mount {
            CameraMount::Mounted { prim_path } => Some(prim_path.as_str()),
            CameraMount::Arcball => None,
        })
        .and_then(|prim_path| viewpoint_payload(&presentation, prim_path));
    state.set_active(desired, &mut authority);
}

fn viewpoint_payload(
    presentation: &SceneCachePresentation,
    prim_path: &str,
) -> Option<ViewpointPayload> {
    presentation.entries.iter().find_map(|entry| {
        let SceneCacheEntryKind::OwnedPrim {
            prim_path: entry_path,
        } = &entry.kind
        else {
            return None;
        };
        if entry_path != prim_path || !entry.cacheable {
            return None;
        }
        let geometry = entry.geometry.as_ref()?;
        let (cpu_bytes, gpu_bytes) =
            ResidencyAuthority::conservative_resident_footprint(geometry.byte_size);
        Some(ViewpointPayload {
            key: ResidencyAuthority::payload_key(entry)?,
            generation: presentation.generation,
            cpu_bytes,
            gpu_bytes,
        })
    })
}

impl ActiveViewpointResidencyState {
    fn set_active(
        &mut self,
        desired: Option<ViewpointPayload>,
        authority: &mut ResidencyAuthority,
    ) {
        if self.active.map(|payload| payload.key) != desired.map(|payload| payload.key) {
            self.release(authority);
        }
        let Some(desired) = desired else {
            return;
        };
        if self.active.is_some() {
            return;
        }
        let _ = authority.request_reason(
            desired.key,
            ResidencyReason::ActiveViewpoint,
            desired.generation,
            desired.cpu_bytes,
            desired.gpu_bytes,
        );
        if authority
            .reasons(&desired.key)
            .is_some_and(|reasons| reasons.contains(&ResidencyReason::ActiveViewpoint))
        {
            self.active = Some(desired);
        }
    }

    fn release(&mut self, authority: &mut ResidencyAuthority) {
        if let Some(payload) = self.active.take() {
            let _ = authority.remove_reason(
                payload.key,
                ResidencyReason::ActiveViewpoint,
                payload.generation,
            );
        }
    }

    fn reset(&mut self) {
        self.scene_id = None;
        self.generation = None;
        self.active = None;
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
    use usd_project::ScenePlacementTransform;

    fn key(scene_id: SceneId, value: u8) -> ScenePayloadKey {
        ScenePayloadKey {
            scene_id,
            blob_hash: HashDigest::new([value; HashDigest::BYTE_LEN]),
        }
    }

    fn payload(scene_id: SceneId, value: u8, generation: u64) -> ViewpointPayload {
        ViewpointPayload {
            key: key(scene_id, value),
            generation,
            cpu_bytes: 2,
            gpu_bytes: 2,
        }
    }

    #[test]
    fn mounted_viewpoint_resolves_only_owned_cache_geometry() {
        let scene = SceneId::new_v4();
        let hash = HashDigest::new([7; HashDigest::BYTE_LEN]);
        let presentation = SceneCachePresentation {
            scene_id: scene,
            generation: 4,
            state: SceneCacheState::Ready,
            entries: vec![SceneCacheEntry {
                address: SceneCacheAddress {
                    scene_id: scene,
                    occurrence: SceneCacheOccurrence::PrimPath("/World/Camera".to_owned()),
                },
                parent: None,
                transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
                bounds: Some(Bounds3 {
                    min: [-1.0; 3],
                    max: [1.0; 3],
                }),
                cacheable: true,
                bim_enabled: false,
                geometry: Some(SceneCacheBlobRef {
                    blob_id: BlobId(hash.to_hex()),
                    byte_size: 8,
                }),
                material: None,
                animation: None,
                semantic_key: None,
                kind: SceneCacheEntryKind::OwnedPrim {
                    prim_path: "/World/Camera".to_owned(),
                },
                content_hash: Some(hash),
            }],
        };

        let resolved = viewpoint_payload(&presentation, "/World/Camera").expect("owned cache");
        assert_eq!(resolved.key.scene_id, scene);
        assert_eq!(resolved.key.blob_hash, hash);
        assert!(viewpoint_payload(&presentation, "/World/Other").is_none());
    }

    #[test]
    fn viewpoint_reason_coexists_and_releases_on_switch() {
        let scene = SceneId::new_v4();
        let first = payload(scene, 1, 3);
        let second = payload(scene, 2, 3);
        let mut authority = ResidencyAuthority::default();
        authority.install_scene(scene, 3, Vec::new());
        assert!(authority.request_reason(
            first.key,
            ResidencyReason::CameraNear,
            3,
            first.cpu_bytes,
            first.gpu_bytes,
        ));
        let mut state = ActiveViewpointResidencyState::default();
        state.set_active(Some(first), &mut authority);
        assert!(
            authority
                .reasons(&first.key)
                .is_some_and(|reasons| reasons.contains(&ResidencyReason::CameraNear))
        );
        state.set_active(Some(second), &mut authority);
        assert!(
            authority
                .reasons(&first.key)
                .is_some_and(|reasons| !reasons.contains(&ResidencyReason::ActiveViewpoint))
        );
        assert!(
            authority
                .reasons(&second.key)
                .is_some_and(|reasons| reasons.contains(&ResidencyReason::ActiveViewpoint))
        );
    }

    #[test]
    fn stale_viewpoint_generation_is_rejected() {
        let scene = SceneId::new_v4();
        let mut authority = ResidencyAuthority::default();
        authority.install_scene(scene, 4, Vec::new());
        let mut state = ActiveViewpointResidencyState::default();
        state.set_active(Some(payload(scene, 1, 3)), &mut authority);
        assert!(state.active.is_none());
        assert!(authority.reasons(&key(scene, 1)).is_none());
    }
}
