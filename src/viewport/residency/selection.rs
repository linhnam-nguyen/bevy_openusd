//! Bounded selection-to-cache residency admission.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use viewport_protocol::SceneAnchor;

use crate::project::{
    cache::SceneCacheStore,
    cache_contract::{
        CacheObjectRef, ProjectCacheLookup, SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheAddress,
        SceneCacheIndex, SceneCacheOccurrence,
    },
    cache_hydration::ActiveProjectCacheContext,
    catalog::manifest_store::ManifestStore,
};
use crate::viewport::{
    api::SceneAnchorIndex, scene::SelectedTargets, session::SceneCachePresentation,
};

use super::{ResidencyAuthority, ResidencyReason, ScenePayloadKey};

pub(crate) const SELECTION_RESIDENCY_BATCH: usize = 512;
pub(crate) const SELECTION_RESIDENCY_QUEUE_CAPACITY: usize = SELECTION_RESIDENCY_BATCH;

#[derive(Clone, Copy, Debug)]
struct SelectedPayload {
    key: ScenePayloadKey,
    generation: u64,
    cpu_bytes: u64,
    gpu_bytes: u64,
}

#[derive(Clone, Debug)]
struct PendingSelection {
    target: SceneAnchor,
    selection_revision: u64,
    generation: u64,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct SelectionResidencyState {
    generation: Option<u64>,
    selection_revision: Option<u64>,
    scene_revision: Option<u64>,
    lookup: Option<ProjectCacheLookup>,
    selected: HashMap<SceneAnchor, ScenePayloadKey>,
    pending: VecDeque<PendingSelection>,
    queued: HashSet<SceneAnchor>,
    selection_cursor: usize,
    last_batch_work: usize,
}

#[cfg(test)]
impl SelectionResidencyState {
    fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn last_batch_work(&self) -> usize {
        self.last_batch_work
    }
}

pub(crate) fn sync_selected_residency(
    presentation: Option<Res<SceneCachePresentation>>,
    cache_context: Option<Res<ActiveProjectCacheContext>>,
    selection: Res<SelectedTargets>,
    scene_index: Res<SceneAnchorIndex>,
    mut authority: ResMut<ResidencyAuthority>,
    mut state: ResMut<SelectionResidencyState>,
) {
    let Some(presentation) = presentation else {
        state.release_selected_reasons(&mut authority);
        state.clear();
        return;
    };
    let generation = presentation.generation;
    let scene_revision = scene_index.revision();
    let scene_changed = state.generation != Some(generation);
    let selection_changed = state.selection_revision != Some(selection.revision());
    let presentation_changed = presentation.is_changed();

    if scene_changed || presentation_changed {
        state.release_selected_reasons(&mut authority);
        state.clear();
        state.generation = Some(generation);
        state.selection_revision = Some(selection.revision());
        state.lookup = build_lookup(&presentation, cache_context.as_deref());
        if let Some(lookup) = state.lookup.as_ref() {
            for (scene_id, scene_generation) in lookup.persistent_generations() {
                authority.register_scene_generation(scene_id, scene_generation);
            }
        }
    } else if selection_changed {
        cleanup_deselected(&selection, &mut state, &mut authority, generation);
        state.selection_revision = Some(selection.revision());
        state.reset_work();
    }

    fill_selection_queue(&selection, &mut state, generation);
    state.scene_revision = Some(scene_revision);
    advance_selection_queue(
        &selection,
        &scene_index,
        presentation.scene_id,
        &mut authority,
        &mut state,
        generation,
    );
}

pub(crate) fn release_selected_residency(world: &mut World) {
    let Some(mut state) = world.remove_resource::<SelectionResidencyState>() else {
        return;
    };
    if let Some(mut authority) = world.get_resource_mut::<ResidencyAuthority>() {
        state.release_selected_reasons(&mut authority);
    }
    state.clear();
    world.insert_resource(state);
}

fn build_lookup(
    presentation: &SceneCachePresentation,
    context: Option<&ActiveProjectCacheContext>,
) -> Option<ProjectCacheLookup> {
    let mut entries = presentation.entries.clone();
    entries.sort_by(|left, right| left.address.cmp(&right.address));
    let active = SceneCacheIndex {
        schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
        scene_id: presentation.scene_id,
        generation: presentation.generation,
        entries,
    };
    let mut indexes = vec![active];
    if let Some(context) = context
        && let Ok(manifest) = ManifestStore::read_validated(&context.project_root)
    {
        let store = SceneCacheStore::new(&context.project_root);
        for scene in manifest.scenes() {
            if scene.id == presentation.scene_id {
                continue;
            }
            if let Ok(Some(index)) = store.load_index(scene.id) {
                indexes.push(index);
            }
        }
    }
    ProjectCacheLookup::from_scene_indexes(&indexes).ok()
}

fn selected_payload(
    lookup: &ProjectCacheLookup,
    scene_id: usd_project::SceneId,
    target: &SceneAnchor,
) -> Option<SelectedPayload> {
    let mut visited = HashSet::new();
    selected_payload_at(lookup, scene_id, &target.prim_path, &mut visited)
}

fn selected_payload_at(
    lookup: &ProjectCacheLookup,
    scene_id: usd_project::SceneId,
    prim_path: &str,
    visited: &mut HashSet<(usd_project::SceneId, String)>,
) -> Option<SelectedPayload> {
    if !visited.insert((scene_id, prim_path.to_owned())) {
        return None;
    }
    let address = SceneCacheAddress {
        scene_id,
        occurrence: SceneCacheOccurrence::PrimPath(prim_path.to_owned()),
    };
    if let Some(object) = lookup.get(&address) {
        if let CacheObjectRef::OwnedPrim {
            geometry,
            content_hash: Some(blob_hash),
            ..
        } = object
        {
            let (cpu_bytes, gpu_bytes) = geometry.as_ref().map_or(
                (
                    crate::viewport::residency::authority::RESIDENT_CPU_METADATA_BYTES,
                    0,
                ),
                |geometry| ResidencyAuthority::conservative_resident_footprint(geometry.byte_size),
            );
            return Some(SelectedPayload {
                key: ScenePayloadKey {
                    scene_id,
                    blob_hash: *blob_hash,
                },
                generation: lookup
                    .persistent_generations()
                    .into_iter()
                    .find_map(|(id, generation)| (id == scene_id).then_some(generation))?,
                cpu_bytes,
                gpu_bytes,
            });
        }
    }

    for (address, object) in lookup.persistent_rows() {
        let SceneCacheOccurrence::Member(member_id) = address.occurrence else {
            continue;
        };
        let CacheObjectRef::ChildScene {
            scene_id: child_scene,
            ..
        } = object
        else {
            continue;
        };
        if address.scene_id == scene_id
            && let Some(child_path) = child_scene_path(prim_path, member_id)
            && let Some(payload) = selected_payload_at(lookup, child_scene, &child_path, visited)
        {
            return Some(payload);
        }
    }
    None
}

fn child_scene_path(prim_path: &str, member_id: usd_project::SceneMemberId) -> Option<String> {
    let prefixes = [
        crate::project::scene::authoring::scene_member_path(member_id),
        crate::project::scene::authoring::legacy_scene_member_path(member_id),
    ];
    prefixes.into_iter().find_map(|prefix| {
        let root = format!("{prefix}/SceneRoot");
        prim_path
            .strip_prefix(&root)
            .map(|suffix| format!("/SceneRoot{suffix}"))
    })
}

fn fill_selection_queue(
    selection: &SelectedTargets,
    state: &mut SelectionResidencyState,
    generation: u64,
) {
    let target_count = selection.0.targets.len();
    if target_count == 0 {
        state.selection_cursor = 0;
        return;
    }
    let mut scanned = 0;
    while state.pending.len() < SELECTION_RESIDENCY_QUEUE_CAPACITY && scanned < target_count {
        if state.selection_cursor >= target_count {
            state.selection_cursor = 0;
        }
        let target = &selection.0.targets[state.selection_cursor];
        state.selection_cursor = (state.selection_cursor + 1) % target_count;
        scanned += 1;
        if state.selected.contains_key(target) || !state.queued.insert(target.clone()) {
            continue;
        }
        state.pending.push_back(PendingSelection {
            target: target.clone(),
            selection_revision: selection.revision(),
            generation,
        });
    }
}

fn cleanup_deselected(
    selection: &SelectedTargets,
    state: &mut SelectionResidencyState,
    authority: &mut ResidencyAuthority,
    generation: u64,
) {
    let desired = selection.0.targets.iter().collect::<HashSet<_>>();
    let old_keys = state.selected.values().copied().collect::<HashSet<_>>();
    let removed = state
        .selected
        .keys()
        .filter(|target| !desired.contains(target))
        .cloned()
        .collect::<Vec<_>>();
    for target in removed {
        state.selected.remove(&target);
    }
    let retained = state.selected.values().copied().collect::<HashSet<_>>();
    for key in old_keys.difference(&retained).copied() {
        let reason_generation = authority.generation_for(&key).unwrap_or(generation);
        let _ = authority.remove_reason(key, ResidencyReason::Selected, reason_generation);
    }
    state.queued.retain(|target| desired.contains(target));
    state.pending.retain(|work| desired.contains(&work.target));
    state.selection_cursor = 0;
}

fn advance_selection_queue(
    selection: &SelectedTargets,
    scene_index: &SceneAnchorIndex,
    scene_id: usd_project::SceneId,
    authority: &mut ResidencyAuthority,
    state: &mut SelectionResidencyState,
    generation: u64,
) {
    state.last_batch_work = 0;
    while state.last_batch_work < SELECTION_RESIDENCY_BATCH {
        let Some(work) = state.pending.pop_front() else {
            break;
        };
        state.last_batch_work += 1;
        state.queued.remove(&work.target);
        if work.generation != generation || work.selection_revision != selection.revision() {
            continue;
        }
        if !selection.0.targets.contains(&work.target) {
            continue;
        }
        let Some(entity) = scene_index.resolve(&work.target) else {
            continue;
        };
        let Some(anchor) = scene_index.anchor_for(entity) else {
            continue;
        };
        let Some(lookup) = state.lookup.as_ref() else {
            continue;
        };
        let Some(payload) = selected_payload(lookup, scene_id, &anchor) else {
            continue;
        };
        let target = work.target.clone();
        let already_selected = authority
            .reasons(&payload.key)
            .is_some_and(|reasons| reasons.contains(&ResidencyReason::Selected));
        let admitted = authority.request_reason(
            payload.key,
            ResidencyReason::Selected,
            payload.generation,
            payload.cpu_bytes,
            payload.gpu_bytes,
        );
        if admitted || already_selected {
            state.selected.insert(target, payload.key);
        } else {
            let _ =
                authority.remove_reason(payload.key, ResidencyReason::Selected, payload.generation);
        }
    }
}

impl SelectionResidencyState {
    fn reset_work(&mut self) {
        self.pending.clear();
        self.queued.clear();
        self.selection_cursor = 0;
        self.last_batch_work = 0;
    }

    pub(crate) fn release_selected_reasons(&mut self, authority: &mut ResidencyAuthority) {
        let Some(generation) = self.generation else {
            self.selected.clear();
            return;
        };
        let keys = self.selected.values().copied().collect::<HashSet<_>>();
        for key in keys {
            let reason_generation = authority.generation_for(&key).unwrap_or(generation);
            let _ = authority.remove_reason(key, ResidencyReason::Selected, reason_generation);
        }
        self.selected.clear();
    }

    fn clear(&mut self) {
        self.generation = None;
        self.selection_revision = None;
        self.scene_revision = None;
        self.lookup = None;
        self.selected.clear();
        self.reset_work();
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
