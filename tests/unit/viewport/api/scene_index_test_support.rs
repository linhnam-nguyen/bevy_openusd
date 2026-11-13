//! Test-only helpers attached to the owning production module.
use super::*;

impl SceneAnchorIndex {
    pub(crate) fn from_test_nodes(nodes: Vec<PrimNodeReadModel>) -> Self {
        let dense = DenseSceneIndex::from_nodes(&nodes, &HashMap::new());
        let node_index_by_anchor = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.anchor.clone(), index))
            .collect();
        Self {
            nodes,
            node_index_by_anchor,
            dense,
            initialized: true,
            revision: 1,
            ..Default::default()
        }
    }

    pub(crate) fn from_test_entity(anchor: SceneAnchor, entity: Entity) -> Self {
        let mut occurrence_index = SceneOccurrenceIndex::default();
        occurrence_index.insert(&anchor.prim_path, entity);
        Self {
            by_anchor: HashMap::from([(anchor.clone(), entity)]),
            by_entity: HashMap::from([(entity, anchor)]),
            occurrence_index,
            initialized: true,
            revision: 1,
            ..Default::default()
        }
    }

    pub(crate) fn from_test_entities(entries: Vec<(SceneAnchor, Entity)>) -> Self {
        let mut occurrence_index = SceneOccurrenceIndex::default();
        for (anchor, entity) in &entries {
            occurrence_index.insert(&anchor.prim_path, *entity);
        }
        let by_anchor = entries
            .iter()
            .cloned()
            .collect::<HashMap<SceneAnchor, Entity>>();
        let by_entity = entries
            .into_iter()
            .map(|(anchor, entity)| (entity, anchor))
            .collect::<HashMap<Entity, SceneAnchor>>();
        Self {
            by_anchor,
            by_entity,
            occurrence_index,
            initialized: true,
            revision: 1,
            ..Default::default()
        }
    }

    pub(crate) fn rebuild_count(&self) -> u64 {
        self.rebuild_count
    }

    pub(crate) fn incremental_work(&self) -> SceneIndexWorkCounters {
        self.incremental_work
    }

    pub(crate) fn pending_addition_count(&self) -> usize {
        self.pending_additions.len()
    }

    pub(crate) fn last_refresh_admitted(&self) -> usize {
        self.last_refresh_admitted
    }

    pub(crate) fn last_reconcile_capture_work(&self) -> usize {
        self.reconcile.last_capture_work()
    }

    pub(crate) fn reconcile_is_pending(&self) -> bool {
        self.reconcile.is_pending()
    }
}
