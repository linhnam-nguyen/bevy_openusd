use std::collections::HashMap;

use bevy::ecs::hierarchy::ChildOf;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdHierarchyTarget, UsdPrimRef, UsdTransparentHierarchyNode};
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::{DenseSceneIndex, SceneAnchorIndex};
use crate::viewport::session::StagePresentationContext;

fn display_name(
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
    let mut visited = std::collections::HashSet::new();
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
    pub(super) fn record_topology(
        &mut self,
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
    ) {
        self.parent_by_entity.clear();
        self.transparent_by_entity.clear();
        for (entity, _, _, _, transparent, _, _) in prims.iter() {
            if let Some(parent) = parents.get(entity).ok().flatten() {
                self.parent_by_entity.insert(entity, parent.parent());
            }
            self.transparent_by_entity
                .insert(entity, transparent.is_some());
        }
    }

    /// Ingests a batch containing only newly projected prim rows.
    ///
    /// Existing metadata edits, removals, and duplicate-path occurrence
    /// changes deliberately return `None`, allowing the authoritative full
    /// rebuild path to preserve its stronger identity guarantees. The common
    /// progressive unique-path case updates only the changed rows and emits a
    /// new immutable projection once for the batch.
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
    ) -> Option<CurrentHierarchyProjection> {
        struct Addition {
            entity: Entity,
            anchor: SceneAnchor,
            name: String,
            display_name: Option<String>,
            transparent: bool,
            visible: bool,
        }

        let mut additions = Vec::new();
        for entity in entities {
            let Ok((entity, prim, authored, target, transparent, visibility, _)) =
                prims.get(entity)
            else {
                continue;
            };
            if prim.path == "/" || self.by_entity.contains_key(&entity) {
                continue;
            }
            if !self.occurrence_index.resolve(&prim.path).is_empty() {
                return None;
            }
            let parent = parents.get(entity).ok().flatten().map(ChildOf::parent);
            self.parent_by_entity
                .extend(parent.map(|parent| (entity, parent)));
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
            return Some(self.prim_projection());
        }

        let added_count = additions.len();
        for addition in &additions {
            self.by_anchor
                .insert(addition.anchor.clone(), addition.entity);
            self.by_entity
                .insert(addition.entity, addition.anchor.clone());
            self.occurrence_index
                .insert(&addition.anchor.prim_path, addition.entity);
        }

        for addition in additions {
            if addition.transparent {
                continue;
            }
            let parent = visual_parent(
                addition.entity,
                &self.parent_by_entity,
                &self.transparent_by_entity,
            )
            .and_then(|parent| self.by_entity.get(&parent).cloned());
            self.nodes.push(PrimNodeReadModel {
                anchor: addition.anchor,
                parent: parent.clone(),
                label: addition.name,
                display_name: addition.display_name,
                visible: addition.visible,
                has_children: false,
            });
            if let Some(parent) = parent
                && let Some(node) = self.nodes.iter_mut().find(|node| node.anchor == parent)
            {
                node.has_children = true;
            }
        }

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
        self.dense = DenseSceneIndex::from_nodes(&self.nodes, &self.by_anchor);
        self.incremental_work = self.incremental_work.saturating_add(added_count as u64);
        self.revision = self.revision.saturating_add(1);
        self.initialized = true;
        Some(self.prim_projection())
    }
}
