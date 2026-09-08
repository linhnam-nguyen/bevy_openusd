use std::collections::{HashMap, HashSet};

use bevy::prelude::Entity;
use viewport_protocol::{PrimNodeReadModel, SceneAnchor};

use super::super::hierarchy::CurrentHierarchyProjection;
use super::super::scene_occurrence_index::SceneOccurrenceIndex;
use super::DenseSceneIndex;

#[derive(Debug)]
pub(super) struct SceneIndexCandidate {
    pub(super) entity: Entity,
    pub(super) path: String,
    pub(super) name: String,
    pub(super) display_name: Option<String>,
    pub(super) transparent: bool,
    pub(super) visible: bool,
    pub(super) parent: Option<Entity>,
}

pub(super) struct SceneIndexSnapshot {
    pub(super) by_anchor: HashMap<SceneAnchor, Entity>,
    pub(super) by_entity: HashMap<Entity, SceneAnchor>,
    pub(super) occurrence_index: SceneOccurrenceIndex,
    pub(super) nodes: Vec<PrimNodeReadModel>,
    pub(super) node_index_by_anchor: HashMap<SceneAnchor, usize>,
    pub(super) dense: DenseSceneIndex,
    pub(super) parent_by_entity: HashMap<Entity, Entity>,
    pub(super) transparent_by_entity: HashMap<Entity, bool>,
    pub(super) projection: CurrentHierarchyProjection,
    pub(super) revision: u64,
}

fn resolve_visual_parent(
    entity: Entity,
    parent_by_entity: &HashMap<Entity, Entity>,
    transparent_by_entity: &HashMap<Entity, bool>,
    resolved_by_entity: &mut HashMap<Entity, Option<Entity>>,
) -> Option<Entity> {
    if let Some(parent) = resolved_by_entity.get(&entity) {
        return *parent;
    }

    let mut visited = HashSet::new();
    let mut transparent_chain = Vec::new();
    let mut parent = parent_by_entity.get(&entity).copied();
    while let Some(candidate) = parent {
        if !visited.insert(candidate) {
            parent = None;
            break;
        }
        if !transparent_by_entity
            .get(&candidate)
            .copied()
            .unwrap_or(false)
        {
            break;
        }
        if let Some(resolved) = resolved_by_entity.get(&candidate) {
            parent = *resolved;
            break;
        }
        transparent_chain.push(candidate);
        parent = parent_by_entity.get(&candidate).copied();
    }

    for transparent in transparent_chain {
        resolved_by_entity.insert(transparent, parent);
    }
    resolved_by_entity.insert(entity, parent);
    parent
}

pub(super) fn build_snapshot(
    chunks: Vec<Vec<SceneIndexCandidate>>,
    revision: u64,
) -> SceneIndexSnapshot {
    let mut candidates = chunks.into_iter().flatten().collect::<Vec<_>>();
    let candidate_entities = candidates
        .iter()
        .map(|candidate| candidate.entity)
        .collect::<HashSet<_>>();
    let parent_by_entity = candidates
        .iter()
        .filter_map(|candidate| {
            candidate
                .parent
                .filter(|parent| candidate_entities.contains(parent))
                .map(|parent| (candidate.entity, parent))
        })
        .collect::<HashMap<_, _>>();
    let transparent_by_entity = candidates
        .iter()
        .map(|candidate| (candidate.entity, candidate.transparent))
        .collect::<HashMap<_, _>>();

    let mut resolved_visual_parents = HashMap::with_capacity(candidates.len());
    for candidate in &candidates {
        resolve_visual_parent(
            candidate.entity,
            &parent_by_entity,
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

    let mut nodes = candidates
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
        .collect::<Vec<_>>();
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

    let node_index_by_anchor = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.anchor.clone(), index))
        .collect();
    let dense = DenseSceneIndex::from_nodes(&nodes, &by_anchor);
    let projection = CurrentHierarchyProjection::from_prim_nodes(&nodes, revision);

    SceneIndexSnapshot {
        by_anchor,
        by_entity,
        occurrence_index,
        nodes,
        node_index_by_anchor,
        dense,
        parent_by_entity,
        transparent_by_entity,
        projection,
        revision,
    }
}
