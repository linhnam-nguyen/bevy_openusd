use std::collections::{HashMap, HashSet};

use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdHierarchyTarget, UsdPrimRef, UsdTransparentHierarchyNode};
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::{DenseSceneIndex, SceneAnchorIndex};
use crate::viewport::session::StagePresentationContext;

pub(super) fn display_name(
    prim: &UsdPrimRef,
    authored: Option<&UsdDisplayName>,
    target: Option<&UsdHierarchyTarget>,
    presentation: Option<&StagePresentationContext>,
) -> Option<String> {
    let is_presentation_root = presentation
        .is_some_and(|presentation| presentation.root_path.as_deref() == Some(prim.path.as_str()));
    if is_presentation_root {
        presentation
            .and_then(|presentation| presentation.root_name.clone())
            .or_else(|| {
                target
                    .and_then(|target| presentation?.target_name(&target.kind, &target.id))
                    .map(str::to_owned)
            })
            .or_else(|| authored.map(|display_name| display_name.0.clone()))
    } else {
        target
            .and_then(|target| presentation?.target_name(&target.kind, &target.id))
            .map(str::to_owned)
            .or_else(|| authored.map(|display_name| display_name.0.clone()))
    }
}

fn visual_parent(
    entity: Entity,
    parent_by_entity: &HashMap<Entity, Entity>,
    transparent_by_entity: &HashMap<Entity, bool>,
) -> Option<Entity> {
    let mut current = parent_by_entity.get(&entity).copied();
    let mut visited = HashSet::new();
    while let Some(candidate) = current {
        if !visited.insert(candidate) {
            return None;
        }
        if !transparent_by_entity
            .get(&candidate)
            .copied()
            .unwrap_or(false)
        {
            return Some(candidate);
        }
        current = parent_by_entity.get(&candidate).copied();
    }
    None
}

impl SceneAnchorIndex {
    /// Admits only new unique prim rows. Derived tree structures are published
    /// later by [`Self::flush_incremental_derived`] when the progressive batch
    /// reaches a quiescent update, preventing a full sort/reindex/projection
    /// for every streaming batch.
    pub(super) fn ingest_added(
        &mut self,
        entities: impl IntoIterator<Item = Entity>,
        prims: &Query<(
            Entity,
            &UsdPrimRef,
            Option<&UsdDisplayName>,
            Option<&UsdHierarchyTarget>,
            Option<&UsdTransparentHierarchyNode>,
            Option<&Visibility>,
            Option<&bevy::ecs::hierarchy::Children>,
        )>,
        parents: &Query<Option<&ChildOf>>,
        presentation: Option<&StagePresentationContext>,
    ) -> Option<usize> {
        struct Addition {
            entity: Entity,
            anchor: SceneAnchor,
            name: String,
            display_name: Option<String>,
            transparent: bool,
            visible: bool,
        }

        let mut additions = Vec::new();
        let mut batch_paths = HashSet::new();
        for entity in entities {
            let Ok((entity, prim, authored, target, transparent, visibility, _)) =
                prims.get(entity)
            else {
                continue;
            };
            if prim.path == "/" || self.by_entity.contains_key(&entity) {
                continue;
            }
            if !batch_paths.insert(prim.path.clone())
                || !self.occurrence_index.resolve(&prim.path).is_empty()
            {
                return None;
            }
            let parent = parents.get(entity).ok().flatten().map(ChildOf::parent);
            if let Some(parent) = parent {
                self.parent_by_entity.insert(entity, parent);
            }
            self.transparent_by_entity
                .insert(entity, transparent.is_some());
            additions.push(Addition {
                entity,
                anchor: SceneAnchor::active_session(&prim.path),
                name: super::prim_name(&prim.path).to_owned(),
                display_name: display_name(prim, authored, target, presentation),
                transparent: transparent.is_some(),
                visible: !matches!(visibility, Some(Visibility::Hidden)),
            });
        }
        if additions.is_empty() {
            return Some(0);
        }

        for addition in &additions {
            self.by_anchor
                .insert(addition.anchor.clone(), addition.entity);
            self.by_entity
                .insert(addition.entity, addition.anchor.clone());
            self.occurrence_index
                .insert(&addition.anchor.prim_path, addition.entity);
        }

        let mut visual_parents = Vec::new();
        for addition in &additions {
            if addition.transparent {
                continue;
            }
            let parent = visual_parent(
                addition.entity,
                &self.parent_by_entity,
                &self.transparent_by_entity,
            )
            .and_then(|parent| self.by_entity.get(&parent).cloned());
            let index = self.nodes.len();
            self.nodes.push(PrimNodeReadModel {
                anchor: addition.anchor.clone(),
                parent: parent.clone(),
                label: addition.name.clone(),
                display_name: addition.display_name.clone(),
                visible: addition.visible,
                has_children: false,
            });
            self.node_index_by_anchor
                .insert(addition.anchor.clone(), index);
            visual_parents.push(parent);
        }
        for parent in visual_parents.into_iter().flatten() {
            if let Some(index) = self.node_index_by_anchor.get(&parent).copied()
                && let Some(node) = self.nodes.get_mut(index)
            {
                node.has_children = true;
            }
        }

        self.incremental_work.admitted_rows = self
            .incremental_work
            .admitted_rows
            .saturating_add(additions.len() as u64);
        self.revision = self.revision.saturating_add(1);
        self.initialized = true;
        self.derived_dirty = true;
        Some(additions.len())
    }

    /// Coalesces the expensive immutable representations once no new rows
    /// arrived during this update. The mutable maps above remain authoritative
    /// for O(1) anchor resolution while this work is deferred.
    pub(super) fn flush_incremental_derived(&mut self) -> CurrentHierarchyProjection {
        self.nodes.sort_by(|left, right| {
            left.anchor
                .prim_path
                .cmp(&right.anchor.prim_path)
                .then_with(|| {
                    left.anchor
                        .instance_context
                        .cmp(&right.anchor.instance_context)
                })
        });
        self.node_index_by_anchor = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.anchor.clone(), index))
            .collect();
        self.dense = DenseSceneIndex::from_nodes(&self.nodes, &self.by_anchor);
        let row_count = self.nodes.len() as u64;
        self.incremental_work.reindexed_rows = self
            .incremental_work
            .reindexed_rows
            .saturating_add(row_count);
        self.incremental_work.projected_rows = self
            .incremental_work
            .projected_rows
            .saturating_add(row_count);
        self.incremental_work.derived_flushes =
            self.incremental_work.derived_flushes.saturating_add(1);
        self.derived_dirty = false;
        self.prim_projection()
    }
}
