//! Deterministic virtual BIM classification index.

use std::borrow::Cow;
use std::collections::HashMap;

use usd_model::{CanonicalValue, EntityKey, EntitySnapshot, SemanticSnapshot};
use viewport_protocol::{
    BimFieldKey, BimPageRequest, ClassificationChildrenPage, ClassificationLeafReadModel,
    ClassificationNodeReadModel, ClassificationRecipe, ClassificationRow, SceneAnchor,
    UNCLASSIFIED_LABEL,
};

use super::BimQueryError;

const ROOT_PARENT: usize = usize::MAX;

struct ClassificationNode {
    id: String,
    parent: Option<usize>,
    level: usize,
    label: String,
    entity_count: u32,
    children: Vec<usize>,
    leaves: Vec<EntityKey>,
}

enum PageChildren<'a> {
    Groups(&'a [usize]),
    Leaves(&'a [EntityKey]),
}

pub(super) struct ClassificationIndex {
    roots: Vec<usize>,
    nodes: Vec<ClassificationNode>,
    by_id: HashMap<String, usize>,
    child_lookup: HashMap<(usize, String), usize>,
}

impl ClassificationIndex {
    pub(super) fn build(snapshot: &SemanticSnapshot, recipe: &ClassificationRecipe) -> Self {
        let mut index = Self {
            roots: Vec::new(),
            nodes: Vec::new(),
            by_id: HashMap::new(),
            child_lookup: HashMap::new(),
        };

        for entity in snapshot.entities.values() {
            let mut parent = None;
            for (level_index, level) in recipe.levels.iter().enumerate() {
                let value = group_value(entity, &level.field);
                let node_index = index.get_or_insert_node(parent, level_index, level, value);
                index.nodes[node_index].entity_count =
                    index.nodes[node_index].entity_count.saturating_add(1);
                parent = Some(node_index);
            }
            if let Some(leaf_parent) = parent {
                index.nodes[leaf_parent].leaves.push(entity.key.clone());
            }
        }

        for node_index in 0..index.nodes.len() {
            let mut children = std::mem::take(&mut index.nodes[node_index].children);
            children.sort_unstable_by_key(|child| index.nodes[*child].id.clone());
            index.nodes[node_index].children = children;
            index.nodes[node_index].leaves.sort_unstable();
        }
        index
            .roots
            .sort_unstable_by(|left, right| index.nodes[*left].id.cmp(&index.nodes[*right].id));
        index
    }

    pub(super) fn page(
        &self,
        snapshot: &SemanticSnapshot,
        parent_id: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> Result<ClassificationChildrenPage, BimQueryError> {
        let request = BimPageRequest::new(page.saturating_mul(page_size), page_size);
        request.validate_max(
            "bim.classification.page_size",
            viewport_protocol::MAX_BIM_CLASSIFICATION_PAGE_SIZE,
        )?;
        let children = match parent_id {
            None => PageChildren::Groups(&self.roots),
            Some(id) => {
                let parent = self
                    .by_id
                    .get(id)
                    .copied()
                    .ok_or_else(|| BimQueryError::ClassificationNodeNotFound(id.to_owned()))?;
                let node = &self.nodes[parent];
                if !node.children.is_empty() {
                    PageChildren::Groups(&node.children)
                } else {
                    PageChildren::Leaves(&node.leaves)
                }
            }
        };
        let total = match &children {
            PageChildren::Groups(indices) => indices.len(),
            PageChildren::Leaves(keys) => keys.len(),
        };
        let offset = request.offset as usize;
        let start = offset.min(total);
        let end = start.saturating_add(request.limit as usize).min(total);
        let mut rows = Vec::with_capacity(end.saturating_sub(start));
        match children {
            PageChildren::Groups(indices) => {
                for &index in &indices[start..end] {
                    rows.push(ClassificationRow::Group(self.node_read_model(index)));
                }
            }
            PageChildren::Leaves(keys) => {
                for key in &keys[start..end] {
                    let entity = snapshot
                        .entities
                        .get(key)
                        .ok_or_else(|| BimQueryError::EntityNotFound(key.as_str().to_owned()))?;
                    rows.push(ClassificationRow::Leaf(ClassificationLeafReadModel {
                        anchor: SceneAnchor::active_session(entity.prim_path.clone()),
                        label: entity_label(entity),
                    }));
                }
            }
        }

        let row_count = rows.len() as u32;
        Ok(ClassificationChildrenPage {
            parent_id: parent_id.map(str::to_owned),
            page,
            page_size,
            total: total as u32,
            rows,
            has_more: request.offset.saturating_add(row_count) < total as u32,
        })
    }

    fn get_or_insert_node(
        &mut self,
        parent: Option<usize>,
        level_index: usize,
        level: &viewport_protocol::ClassificationLevel,
        label: String,
    ) -> usize {
        let lookup_key = (parent.unwrap_or(ROOT_PARENT), label.clone());
        if let Some(index) = self.child_lookup.get(&lookup_key) {
            return *index;
        }
        let parent_id = parent
            .map(|index| self.nodes[index].id.as_str())
            .unwrap_or("root")
            .to_owned();
        let id = node_id(&parent_id, level_index, level, &label);
        let index = self.nodes.len();
        self.nodes.push(ClassificationNode {
            id: id.clone(),
            parent,
            level: level_index,
            label,
            entity_count: 0,
            children: Vec::new(),
            leaves: Vec::new(),
        });
        self.by_id.insert(id, index);
        self.child_lookup.insert(lookup_key, index);
        if let Some(parent) = parent {
            self.nodes[parent].children.push(index);
        } else {
            self.roots.push(index);
        }
        index
    }

    fn node_read_model(&self, index: usize) -> ClassificationNodeReadModel {
        let node = &self.nodes[index];
        ClassificationNodeReadModel {
            id: node.id.clone(),
            parent_id: node.parent.map(|parent| self.nodes[parent].id.clone()),
            level: node.level as u32,
            label: node.label.clone(),
            entity_count: node.entity_count,
            has_children: !node.children.is_empty() || !node.leaves.is_empty(),
        }
    }
}

pub(super) fn canonical_value_text(value: &CanonicalValue) -> Option<Cow<'_, str>> {
    Some(match value {
        CanonicalValue::Null => Cow::Borrowed("null"),
        CanonicalValue::Bool(value) => Cow::Owned(value.to_string()),
        CanonicalValue::Integer(value) => Cow::Owned(value.to_string()),
        CanonicalValue::Real(value) => Cow::Owned(value.to_string()),
        CanonicalValue::Text(value) => Cow::Borrowed(value),
        CanonicalValue::TextArray(values) => Cow::Owned(serde_json::to_string(values).ok()?),
        CanonicalValue::NumberArray(values) => Cow::Owned(serde_json::to_string(values).ok()?),
        CanonicalValue::Json(value) => Cow::Borrowed(value),
    })
}

fn group_value(entity: &EntitySnapshot, field: &BimFieldKey) -> String {
    let value = match field {
        BimFieldKey::Category => entity.semantic.category.as_deref().map(Cow::Borrowed),
        BimFieldKey::Family => entity.semantic.family.as_deref().map(Cow::Borrowed),
        BimFieldKey::Type => entity.semantic.type_name.as_deref().map(Cow::Borrowed),
        BimFieldKey::Property(name) => entity
            .properties
            .iter()
            .find(|property| property.name == *name)
            .and_then(|property| canonical_value_text(&property.value)),
    };
    value.filter(|value| !value.trim().is_empty()).map_or_else(
        || UNCLASSIFIED_LABEL.to_owned(),
        |value| value.trim().to_owned(),
    )
}

fn node_id(
    parent_id: &str,
    level_index: usize,
    level: &viewport_protocol::ClassificationLevel,
    label: &str,
) -> String {
    let mut input = Vec::with_capacity(parent_id.len() + level.id.len() + label.len() + 32);
    input.extend_from_slice(parent_id.as_bytes());
    input.push(0);
    input.extend_from_slice(level.id.as_bytes());
    input.push(0);
    input.extend_from_slice(level.field.stable_key().as_bytes());
    input.push(0);
    input.extend_from_slice(label.as_bytes());
    format!("bim-group-{level_index}-{}", blake3::hash(&input).to_hex())
}

fn entity_label(entity: &EntitySnapshot) -> String {
    entity
        .semantic
        .display_name
        .as_deref()
        .unwrap_or(entity.prim_path.as_str())
        .to_owned()
}
