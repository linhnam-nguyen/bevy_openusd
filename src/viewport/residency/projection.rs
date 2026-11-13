//! Disposable Bevy projection for admitted Scene-cache occurrences.

use std::collections::HashMap;

use bevy::asset::{AssetId, Handle};
use bevy::ecs::component::Component;
use bevy::ecs::system::Commands;
use bevy::math::Mat4;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::prelude::{Entity, Resource, Transform, Visibility, World};
use usd_model::Bounds3;

use crate::project::cache_contract::SceneCacheAddress;
use usd_project::ScenePlacementTransform;

use super::authority::ScenePayloadKey;
use super::spatial::SceneSpatialPayload;

#[derive(Component, Clone, Debug)]
pub(crate) struct SceneResidencyOccurrence {
    pub(crate) address: SceneCacheAddress,
    pub(crate) bounds: Bounds3,
}

#[derive(Component, Clone, Debug)]
pub(crate) struct SceneResidencySourceTransform(pub(crate) ScenePlacementTransform);

#[derive(Clone, Debug)]
struct ProjectionEntry {
    payload_key: ScenePayloadKey,
    transform: ScenePlacementTransform,
    bounds: Bounds3,
    mesh: Option<Handle<Mesh>>,
    entity: Option<Entity>,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct SceneResidencyProjection {
    entries: HashMap<SceneCacheAddress, ProjectionEntry>,
    payload_occurrences: HashMap<ScenePayloadKey, Vec<SceneCacheAddress>>,
    asset_occurrences: HashMap<AssetId<Mesh>, Vec<SceneCacheAddress>>,
}

impl SceneResidencyProjection {
    pub(crate) fn install_scene(&mut self, payloads: &[SceneSpatialPayload]) {
        self.entries.clear();
        self.payload_occurrences.clear();
        self.asset_occurrences.clear();
        for payload in payloads {
            let address = payload.address.clone();
            self.payload_occurrences
                .entry(payload.payload_key)
                .or_default()
                .push(address.clone());
            self.entries.insert(
                address,
                ProjectionEntry {
                    payload_key: payload.payload_key,
                    transform: payload.transform.clone(),
                    bounds: payload.bounds,
                    mesh: None,
                    entity: None,
                },
            );
        }
    }

    pub(crate) fn attach_payload(
        &mut self,
        key: ScenePayloadKey,
        handle: Handle<Mesh>,
        commands: &mut Commands,
    ) {
        let addresses = self
            .payload_occurrences
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let asset_id = handle.id();
        for address in addresses {
            let Some(entry) = self.entries.get_mut(&address) else {
                continue;
            };
            if entry.entity.is_some() {
                continue;
            }
            let entity = commands
                .spawn((
                    Mesh3d(handle.clone()),
                    transform_for_cache(&entry.transform),
                    SceneResidencyOccurrence {
                        address: address.clone(),
                        bounds: entry.bounds,
                    },
                    SceneResidencySourceTransform(entry.transform.clone()),
                    Visibility::Visible,
                ))
                .id();
            entry.mesh = Some(handle.clone());
            entry.entity = Some(entity);
            self.asset_occurrences
                .entry(asset_id)
                .or_default()
                .push(address);
        }
    }

    pub(crate) fn release_asset(&mut self, asset_id: AssetId<Mesh>, commands: &mut Commands) {
        let addresses = self.asset_occurrences.remove(&asset_id).unwrap_or_default();
        for address in addresses {
            let Some(entry) = self.entries.get_mut(&address) else {
                continue;
            };
            if entry
                .mesh
                .as_ref()
                .is_some_and(|handle| handle.id() == asset_id)
            {
                if let Some(entity) = entry.entity.take() {
                    commands.entity(entity).despawn();
                }
                entry.mesh = None;
            }
        }
    }

    pub(crate) fn retire(&mut self, commands: &mut Commands) {
        for entry in self.entries.values_mut() {
            if let Some(entity) = entry.entity.take() {
                commands.entity(entity).despawn();
            }
        }
        self.entries.clear();
        self.payload_occurrences.clear();
        self.asset_occurrences.clear();
    }

    pub(crate) fn retire_world(&mut self, world: &mut World) {
        for entry in self.entries.values_mut() {
            if let Some(entity) = entry.entity.take() {
                let _ = world.despawn(entity);
            }
        }
        self.entries.clear();
        self.payload_occurrences.clear();
        self.asset_occurrences.clear();
    }
}

fn transform_for_cache(transform: &ScenePlacementTransform) -> Transform {
    let matrix = transform.0.map(|value| value as f32);
    Transform::from_matrix(Mat4::from_cols_array(&matrix))
}

#[cfg(test)]
#[path = "../../../tests/unit/viewport/residency/projection_inline_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/viewport/residency/projection_test_support.rs"]
mod test_support;
