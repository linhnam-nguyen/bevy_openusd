//! Immutable runtime lookup for published Scene cache rows.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, ensure};
use bevy::ecs::resource::Resource;
use usd_model::HashDigest;
use usd_project::{SceneId, SceneMemberId};

use super::{
    CacheObjectRef, SceneCacheAddress, SceneCacheEntry, SceneCacheEntryKind, SceneCacheIndex,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SceneObjectKey {
    pub(crate) scene_id: SceneId,
    pub(crate) content_hash: HashDigest,
}

#[derive(Clone, Debug, Default, Resource)]
pub(crate) struct ProjectCacheLookup {
    by_address: HashMap<SceneCacheAddress, CacheObjectRef>,
    by_hash: HashMap<SceneObjectKey, Vec<SceneCacheAddress>>,
    generations: HashMap<SceneId, u64>,
    scene_addresses: HashMap<SceneId, Vec<SceneCacheAddress>>,
    scene_hashes: HashMap<SceneId, Vec<SceneObjectKey>>,
    child_members: HashMap<(SceneId, SceneMemberId), SceneId>,
    scene_member_keys: HashMap<SceneId, Vec<(SceneId, SceneMemberId)>>,
}

impl ProjectCacheLookup {
    pub(crate) fn from_scene_indexes(indexes: &[SceneCacheIndex]) -> Result<Self> {
        let mut scenes = HashSet::new();
        let mut lookup = Self::default();
        for index in indexes {
            ensure!(
                scenes.insert(index.scene_id),
                "Project cache lookup cannot merge multiple generations of one Scene"
            );
            index.validate(index.scene_id, index.generation)?;
            lookup.insert_scene_index(index)?;
        }
        Ok(lookup)
    }

    pub(crate) fn from_persistent_rows(
        generations: Vec<(SceneId, u64)>,
        rows: Vec<(SceneCacheAddress, CacheObjectRef)>,
    ) -> Result<Self> {
        let mut lookup = Self::default();
        for (scene_id, generation) in generations {
            ensure!(
                lookup.generations.insert(scene_id, generation).is_none(),
                "Project cache lookup has duplicate Scene generations"
            );
        }
        for (address, object) in rows {
            lookup.insert_row(address, object)?;
        }
        Ok(lookup)
    }

    /// Replace one Scene's immutable lookup rows without rebuilding unrelated
    /// Scenes. Presentation changes therefore touch only the active Scene.
    pub(crate) fn replace_scene_index(&mut self, index: &SceneCacheIndex) -> Result<()> {
        index.validate(index.scene_id, index.generation)?;
        self.remove_scene(index.scene_id);
        self.insert_scene_index(index)
    }

    pub(crate) fn get(&self, address: &SceneCacheAddress) -> Option<&CacheObjectRef> {
        self.by_address.get(address)
    }

    pub(crate) fn addresses_for_hash(&self, key: SceneObjectKey) -> &[SceneCacheAddress] {
        self.by_hash.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(crate) fn generation(&self, scene_id: SceneId) -> Option<u64> {
        self.generations.get(&scene_id).copied()
    }

    pub(crate) fn child_scene_for_member(
        &self,
        parent_scene: SceneId,
        member_id: SceneMemberId,
    ) -> Option<SceneId> {
        self.child_members.get(&(parent_scene, member_id)).copied()
    }

    pub(crate) fn persistent_rows(&self) -> Vec<(SceneCacheAddress, CacheObjectRef)> {
        let mut rows = self
            .by_address
            .iter()
            .map(|(address, object)| (address.clone(), object.clone()))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        rows
    }

    pub(crate) fn persistent_generations(&self) -> Vec<(SceneId, u64)> {
        let mut generations = self
            .generations
            .iter()
            .map(|(scene_id, generation)| (*scene_id, *generation))
            .collect::<Vec<_>>();
        generations.sort_by_key(|(scene_id, _)| *scene_id);
        generations
    }

    fn insert_scene_index(&mut self, index: &SceneCacheIndex) -> Result<()> {
        ensure!(
            !self.generations.contains_key(&index.scene_id),
            "Project cache lookup cannot merge multiple generations of one Scene"
        );
        self.generations.insert(index.scene_id, index.generation);
        for entry in &index.entries {
            self.insert_row(entry.address.clone(), cache_object_ref(entry))?;
        }
        Ok(())
    }

    fn insert_row(&mut self, address: SceneCacheAddress, object: CacheObjectRef) -> Result<()> {
        let scene_id = address.scene_id;
        ensure!(
            self.by_address
                .insert(address.clone(), object.clone())
                .is_none(),
            "duplicate Project cache address"
        );
        self.scene_addresses
            .entry(scene_id)
            .or_default()
            .push(address.clone());
        if let Some(content_hash) = object_content_hash(&object) {
            let key = SceneObjectKey {
                scene_id,
                content_hash,
            };
            self.by_hash.entry(key).or_default().push(address.clone());
            self.scene_hashes.entry(scene_id).or_default().push(key);
        }
        if let CacheObjectRef::ChildScene {
            scene_id: child,
            member_id,
        } = object
        {
            let key = (scene_id, member_id);
            self.child_members.insert(key, child);
            self.scene_member_keys
                .entry(scene_id)
                .or_default()
                .push(key);
        }
        Ok(())
    }

    fn remove_scene(&mut self, scene_id: SceneId) {
        if let Some(addresses) = self.scene_addresses.remove(&scene_id) {
            for address in addresses {
                self.by_address.remove(&address);
            }
        }
        if let Some(keys) = self.scene_hashes.remove(&scene_id) {
            for key in keys {
                if let Some(addresses) = self.by_hash.get_mut(&key) {
                    addresses.retain(|address| address.scene_id != scene_id);
                    if addresses.is_empty() {
                        self.by_hash.remove(&key);
                    }
                }
            }
        }
        if let Some(keys) = self.scene_member_keys.remove(&scene_id) {
            for key in keys {
                self.child_members.remove(&key);
            }
        }
        self.generations.remove(&scene_id);
    }
}

fn object_content_hash(object: &CacheObjectRef) -> Option<HashDigest> {
    match object {
        CacheObjectRef::OwnedPrim { content_hash, .. } => *content_hash,
        CacheObjectRef::ChildScene { .. } | CacheObjectRef::ChildModel { .. } => None,
    }
}

fn cache_object_ref(entry: &SceneCacheEntry) -> CacheObjectRef {
    match &entry.kind {
        SceneCacheEntryKind::OwnedPrim { prim_path } => CacheObjectRef::OwnedPrim {
            prim_path: prim_path.clone(),
            geometry: entry.geometry.clone(),
            material: entry.material.clone(),
            animation: entry.animation.clone(),
            semantic_key: entry.semantic_key.clone(),
            content_hash: entry.content_hash,
        },
        SceneCacheEntryKind::ChildScene {
            scene_id,
            member_id,
        } => CacheObjectRef::ChildScene {
            scene_id: *scene_id,
            member_id: *member_id,
        },
        SceneCacheEntryKind::ChildModel {
            model_id,
            member_id,
        } => CacheObjectRef::ChildModel {
            model_id: *model_id,
            member_id: *member_id,
        },
    }
}

#[cfg(test)]
#[path = "../../tests/unit/project/cache_lookup_tests.rs"]
mod tests;
