use std::collections::{HashMap, HashSet};

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use usd_bevy::{UsdDisplayName, UsdPrimRef};
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::super::scene_occurrence_index::SceneOccurrenceIndex;
use super::SceneAnchorIndex;

impl SceneAnchorIndex {
    pub(super) fn rebuild(
        &mut self,
        prims: &Query<(
            Entity,
            &UsdPrimRef,
            Option<&UsdDisplayName>,
            Option<&Visibility>,
            Option<&Children>,
        )>,
    ) -> CurrentHierarchyProjection {
        #[derive(Debug)]
        struct Candidate {
            entity: Entity,
            path: String,
            name: String,
            display_name: Option<String>,
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
            .map(|(entity, prim, display_name, visibility, children)| {
                let display_name = display_name.map(|display_name| display_name.0.clone());
                Candidate {
                    entity,
                    path: prim.path.clone(),
                    name: super::prim_name(&prim.path).to_owned(),
                    display_name,
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
            })
            .collect();

        let mut parent_by_child = HashMap::new();
        for candidate in &candidates {
            for child in &candidate.children {
                parent_by_child.insert(*child, candidate.entity);
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
                let anchor = by_entity.get(&candidate.entity)?.clone();
                let parent = parent_by_child
                    .get(&candidate.entity)
                    .and_then(|entity| by_entity.get(entity))
                    .cloned();
                Some(PrimNodeReadModel {
                    anchor,
                    parent,
                    label: candidate.name,
                    display_name: candidate.display_name,
                    visible: candidate.visible,
                    has_children: !candidate.children.is_empty(),
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
