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
#[path = "../../../tests/unit/viewport/residency/viewpoint_inline_tests.rs"]
mod tests;
