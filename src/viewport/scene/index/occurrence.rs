use std::collections::HashMap;

use bevy::ecs::lifecycle::{Insert, Remove};
use bevy::prelude::*;
use usd_bevy::UsdPointOccurrence;
use viewport_protocol::{HierarchyVisibilityState, SceneAnchor};

use super::SceneAnchorIndex;

/// Reverse index from a prim path to all scene-local occurrences of that path.
///
/// Native-instance projection can expose one semantic path under multiple
/// instance contexts. Keeping this index separate from the hierarchy maps
/// makes path-only presentation lookup proportional to its actual matches.
#[derive(Debug, Default)]
pub(crate) struct SceneOccurrenceIndex {
    by_prim_path: HashMap<String, Vec<Entity>>,
}

impl SceneOccurrenceIndex {
    pub(crate) fn insert(&mut self, prim_path: &str, entity: Entity) {
        self.by_prim_path
            .entry(prim_path.to_owned())
            .or_default()
            .push(entity);
    }

    pub(crate) fn resolve(&self, prim_path: &str) -> &[Entity] {
        self.by_prim_path.get(prim_path).map_or(&[], Vec::as_slice)
    }
}

/// Presentation roots share the scene anchor API without adding rows to the
/// authored USD prim tree. One root covers all of an occurrence's mesh parts.
#[derive(Default)]
pub(super) struct ScenePointOccurrences {
    anchors: HashMap<Entity, PointAnchor>,
    entities: HashMap<SceneAnchor, Entity>,
    logical: HashMap<String, HashMap<i64, Entity>>,
    pub(super) revision: u64,
}

struct PointAnchor {
    anchor: SceneAnchor,
    logical_id: i64,
    incarnation: u64,
    masked: bool,
}

impl ScenePointOccurrences {
    pub(super) fn resolve(&self, anchor: &SceneAnchor) -> Option<Entity> {
        self.entities.get(anchor).copied()
    }

    pub(super) fn presentation_key(&self, anchor: &SceneAnchor) -> Option<(i64, u64)> {
        let record = self.anchors.get(&self.resolve(anchor)?)?;
        (!record.masked).then_some((record.logical_id, record.incarnation))
    }

    pub(super) fn anchor_for(&self, entity: Entity) -> Option<SceneAnchor> {
        self.anchors
            .get(&entity)
            .map(|record| record.anchor.clone())
    }

    pub(super) fn anchor_for_logical_id(
        &self,
        instancer: &str,
        logical_id: i64,
    ) -> Option<SceneAnchor> {
        let entity = self.logical.get(instancer)?.get(&logical_id)?;
        let record = self.anchors.get(entity)?;
        (!record.masked).then(|| record.anchor.clone())
    }

    pub(super) fn visibility(&self, anchor: &SceneAnchor) -> Option<HierarchyVisibilityState> {
        let entity = self.resolve(anchor)?;
        self.anchors
            .get(&entity)
            .map(|record| HierarchyVisibilityState::from_visible(!record.masked))
    }

    fn remove(&mut self, entity: Entity) {
        if let Some(record) = self.anchors.remove(&entity) {
            if self.entities.get(&record.anchor) == Some(&entity) {
                self.entities.remove(&record.anchor);
            }
            if let Some(instancer) = self.logical.get_mut(&record.anchor.prim_path) {
                if instancer.get(&record.logical_id) == Some(&entity) {
                    instancer.remove(&record.logical_id);
                }
                if instancer.is_empty() {
                    self.logical.remove(&record.anchor.prim_path);
                }
            }
            self.revision = self.revision.saturating_add(1);
        }
    }
}

pub(super) fn upsert_point_occurrence(
    event: On<Insert<UsdPointOccurrence>>,
    occurrences: Query<&UsdPointOccurrence>,
    mut index: ResMut<SceneAnchorIndex>,
) {
    let entity = event.entity;
    let Ok(occurrence) = occurrences.get(entity) else {
        return;
    };
    let kind = if occurrence.authored { "id" } else { "row" };
    let anchor = SceneAnchor {
        session_id: None,
        prim_path: occurrence.instancer.to_string(),
        instance_context: Some(format!(
            "point:{kind}:{}:{}",
            occurrence.logical_id, occurrence.incarnation
        )),
    };
    index.point_occurrences.remove(entity);
    index
        .point_occurrences
        .entities
        .insert(anchor.clone(), entity);
    index
        .point_occurrences
        .logical
        .entry(anchor.prim_path.clone())
        .or_default()
        .insert(occurrence.logical_id, entity);
    index.point_occurrences.anchors.insert(
        entity,
        PointAnchor {
            anchor,
            logical_id: occurrence.logical_id,
            incarnation: occurrence.incarnation,
            masked: occurrence.masked,
        },
    );
    index.point_occurrences.revision = index.point_occurrences.revision.saturating_add(1);
}

pub(super) fn remove_point_occurrence(
    event: On<Remove<UsdPointOccurrence>>,
    mut index: ResMut<SceneAnchorIndex>,
) {
    index.point_occurrences.remove(event.entity);
}
