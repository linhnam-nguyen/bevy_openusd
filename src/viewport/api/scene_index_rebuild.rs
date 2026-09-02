use std::collections::{HashMap, HashSet};

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdHierarchyTarget, UsdPrimRef, UsdTransparentHierarchyNode};
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::super::scene_occurrence_index::SceneOccurrenceIndex;
use super::SceneAnchorIndex;
use crate::viewport::session::StagePresentationContext;

fn resolve_visual_parent(
    entity: Entity,
    parent_by_child: &HashMap<Entity, Entity>,
    transparent_by_entity: &HashMap<Entity, bool>,
    resolved_by_entity: &mut HashMap<Entity, Option<Entity>>,
) -> Option<Entity> {
    if let Some(parent) = resolved_by_entity.get(&entity) {
        return *parent;
    }

    let mut transparent_chain = Vec::new();
    let mut parent = parent_by_child.get(&entity).copied();
    while let Some(candidate) = parent {
        let is_transparent = transparent_by_entity
            .get(&candidate)
            .copied()
            .unwrap_or(false);
        if !is_transparent {
            break;
        }
        if let Some(resolved) = resolved_by_entity.get(&candidate) {
            parent = *resolved;
            break;
        }
        transparent_chain.push(candidate);
        parent = parent_by_child.get(&candidate).copied();
    }

    for transparent in transparent_chain {
        resolved_by_entity.insert(transparent, parent);
    }
    resolved_by_entity.insert(entity, parent);
    parent
}

impl SceneAnchorIndex {
    pub(super) fn rebuild(
        &mut self,
        prims: &Query<(
            Entity,
            &UsdPrimRef,
            Option<&UsdDisplayName>,
            Option<&UsdHierarchyTarget>,
            Option<&UsdTransparentHierarchyNode>,
            Option<&Visibility>,
            Option<&Children>,
        )>,
        presentation: Option<&StagePresentationContext>,
    ) -> CurrentHierarchyProjection {
        #[derive(Debug)]
        struct Candidate {
            entity: Entity,
            path: String,
            name: String,
            display_name: Option<String>,
            transparent: bool,
            visible: bool,
            children: Vec<Entity>,
        }

        let prim_entities: HashSet<Entity> = prims
            .iter()
            .filter(|(_, prim, ..)| prim.path != "/")
            .map(|(entity, ..)| entity)
            .collect();
        let mut candidates: Vec<Candidate> = prims
            .iter()
            .filter(|(_, prim, ..)| prim.path != "/")
            .map(
                |(entity, prim, display_name, target, transparent, visibility, children)| {
                    let display_name = display_name.map(|display_name| display_name.0.clone());
                    let is_presentation_root = presentation.is_some_and(|presentation| {
                        presentation.root_path.as_deref() == Some(prim.path.as_str())
                    });
                    let display_name = if is_presentation_root {
                        presentation
                            .and_then(|presentation| presentation.root_name.clone())
                            .or_else(|| {
                                target
                                    .and_then(|target| {
                                        presentation?.target_name(&target.kind, &target.id)
                                    })
                                    .map(str::to_owned)
                            })
                            .or(display_name)
                    } else {
                        target
                            .and_then(|target| presentation?.target_name(&target.kind, &target.id))
                            .map(str::to_owned)
                            .or(display_name)
                    };
                    Candidate {
                        entity,
                        path: prim.path.clone(),
                        name: super::prim_name(&prim.path).to_owned(),
                        display_name,
                        transparent: transparent.is_some(),
                        visible: !matches!(visibility, Some(Visibility::Hidden)),
                        children: children
                            .map(|children| {
                                children
                                    .iter()
                                    .filter(|child| prim_entities.contains(child))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    }
                },
            )
            .collect();

        let mut parent_by_child = HashMap::new();
        for candidate in &candidates {
            for child in &candidate.children {
                parent_by_child.insert(*child, candidate.entity);
            }
        }

        let transparent_by_entity: HashMap<Entity, bool> = candidates
            .iter()
            .map(|candidate| (candidate.entity, candidate.transparent))
            .collect();
        let mut resolved_visual_parents = HashMap::with_capacity(candidates.len());
        for candidate in &candidates {
            resolve_visual_parent(
                candidate.entity,
                &parent_by_child,
                &transparent_by_entity,
                &mut resolved_visual_parents,
            );
        }
        let mut visual_child_counts: HashMap<Entity, usize> = HashMap::new();
        for candidate in &candidates {
            if candidate.transparent {
                continue;
            }
            if let Some(parent) = resolved_visual_parents
                .get(&candidate.entity)
                .copied()
                .flatten()
            {
                *visual_child_counts.entry(parent).or_default() += 1;
            }
        }

        let mut path_counts: HashMap<String, usize> = HashMap::new();
        for candidate in &candidates {
            *path_counts.entry(candidate.path.clone()).or_default() += 1;
        }
        candidates.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.entity.to_bits().cmp(&right.entity.to_bits()))
        });

        let mut next_occurrence: HashMap<String, usize> = HashMap::new();
        let mut by_anchor = HashMap::with_capacity(candidates.len());
        let mut by_entity = HashMap::with_capacity(candidates.len());
        let mut occurrence_index = SceneOccurrenceIndex::default();
        for candidate in &candidates {
            let count = path_counts[&candidate.path];
            let occurrence = next_occurrence.entry(candidate.path.clone()).or_default();
            let instance_context = if count > 1 {
                let context = format!("occurrence-{occurrence}");
                *occurrence += 1;
                Some(context)
            } else {
                None
            };
            let anchor = SceneAnchor {
                session_id: None,
                prim_path: candidate.path.clone(),
                instance_context,
            };
            by_anchor.insert(anchor.clone(), candidate.entity);
            by_entity.insert(candidate.entity, anchor);
            occurrence_index.insert(&candidate.path, candidate.entity);
        }

        let mut nodes: Vec<PrimNodeReadModel> = candidates
            .into_iter()
            .filter_map(|candidate| {
                if candidate.transparent {
                    return None;
                }
                let anchor = by_entity.get(&candidate.entity)?.clone();
                let parent = resolved_visual_parents
                    .get(&candidate.entity)
                    .copied()
                    .flatten()
                    .and_then(|entity| by_entity.get(&entity))
                    .cloned();
                Some(PrimNodeReadModel {
                    anchor,
                    parent,
                    label: candidate.name,
                    display_name: candidate.display_name,
                    visible: candidate.visible,
                    has_children: visual_child_counts
                        .get(&candidate.entity)
                        .copied()
                        .unwrap_or_default()
                        > 0,
                })
            })
            .collect();
        nodes.sort_by(|left, right| {
            left.anchor
                .prim_path
                .cmp(&right.anchor.prim_path)
                .then_with(|| {
                    left.anchor
                        .instance_context
                        .cmp(&right.anchor.instance_context)
                })
        });

        self.dense = super::DenseSceneIndex::from_nodes(&nodes, &by_anchor);
        self.by_anchor = by_anchor;
        self.by_entity = by_entity;
        self.occurrence_index = occurrence_index;
        self.nodes = nodes;
        self.revision = self.revision.saturating_add(1);
        self.initialized = true;
        CurrentHierarchyProjection::from_prim_nodes(&self.nodes, self.revision)
    }
}
