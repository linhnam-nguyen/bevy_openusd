//! Deterministic virtual BIM classification index.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use usd_model::{CanonicalValue, EntityKey, EntitySnapshot, SemanticProperty, SemanticSnapshot};
use viewport_protocol::{
    BimFieldKey, ClassificationRecipe, HierarchyNodeId, HierarchyNodeReadModel, HierarchyReadModel,
    HierarchySource, SceneAnchor, UNCLASSIFIED_LABEL,
};

struct ClassificationNode {
    id: HierarchyNodeId,
    parent: Option<usize>,
    label: String,
    breadcrumb: String,
    children: Vec<usize>,
    leaves: Vec<EntityKey>,
}

pub(super) struct ClassificationIndex {
    nodes: Vec<ClassificationNode>,
    child_lookup: HashMap<(usize, String), usize>,
    color_groups: Vec<ClassificationColorGroup>,
}

#[derive(Clone, Debug)]
pub(super) struct ClassificationColorGroup {
    pub(super) anchor: SceneAnchor,
    pub(super) levels: Vec<(String, String)>,
}

impl ClassificationIndex {
    pub(super) fn build(snapshot: &SemanticSnapshot, recipe: &ClassificationRecipe) -> Self {
        let mut index = Self {
            nodes: Vec::new(),
            child_lookup: HashMap::new(),
            color_groups: Vec::with_capacity(snapshot.entities.len()),
        };

        let property_fields = recipe
            .levels
            .iter()
            .filter_map(|level| match &level.field {
                BimFieldKey::Property(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();

        for entity in snapshot.entities.values() {
            if !entity.semantic.is_bim_entity() {
                continue;
            }
            let properties = indexed_properties(entity, &property_fields);
            let mut parent = None;
            let mut levels = Vec::with_capacity(recipe.levels.len());
            for (level_index, level) in recipe.levels.iter().enumerate() {
                let value = group_value(entity, &level.field, &properties);
                levels.push((level.id.clone(), value.clone()));
                let node_index = index.get_or_insert_node(parent, level_index, level, value);
                parent = Some(node_index);
            }
            if let Some(leaf_parent) = parent {
                index.nodes[leaf_parent].leaves.push(entity.key.clone());
            }
            index.color_groups.push(ClassificationColorGroup {
                anchor: SceneAnchor::active_session(entity.prim_path.clone()),
                levels,
            });
        }

        for node_index in 0..index.nodes.len() {
            let mut children = std::mem::take(&mut index.nodes[node_index].children);
            children.sort_unstable_by_key(|child| index.nodes[*child].id.clone());
            index.nodes[node_index].children = children;
            index.nodes[node_index].leaves.sort_unstable();
        }
        index.color_groups.sort_unstable_by(|left, right| {
            left.anchor
                .cmp(&right.anchor)
                .then_with(|| left.levels.cmp(&right.levels))
        });
        index
    }

    pub(super) fn color_groups(&self) -> &[ClassificationColorGroup] {
        &self.color_groups
    }

    pub(super) fn read_model(
        &self,
        snapshot: &SemanticSnapshot,
        revision: u64,
    ) -> HierarchyReadModel {
        let leaf_count: usize = self.nodes.iter().map(|node| node.leaves.len()).sum();
        let mut nodes = Vec::with_capacity(self.nodes.len() + leaf_count);
        for index in 0..self.nodes.len() {
            nodes.push(self.node_read_model(index));
            let parent_id = self.nodes[index].id.clone();
            let breadcrumb = self.nodes[index].breadcrumb.clone();
            for key in &self.nodes[index].leaves {
                let entity = snapshot
                    .entities
                    .get(key)
                    .expect("classification leaf must belong to its snapshot");
                let name = projected_entity_name(entity);
                nodes.push(HierarchyNodeReadModel::scene(
                    leaf_id(&parent_id, key),
                    Some(parent_id.clone()),
                    name.clone(),
                    format!("{breadcrumb} / {name}"),
                    SceneAnchor::active_session(entity.prim_path.clone()),
                    None,
                    true,
                    false,
                ));
            }
        }
        nodes.sort_unstable_by(|left, right| {
            left.breadcrumb
                .cmp(&right.breadcrumb)
                .then_with(|| left.id.cmp(&right.id))
        });
        HierarchyReadModel {
            source: HierarchySource::BimClassification,
            revision,
            nodes,
        }
    }

    fn get_or_insert_node(
        &mut self,
        parent: Option<usize>,
        level_index: usize,
        level: &viewport_protocol::ClassificationLevel,
        label: String,
    ) -> usize {
        let lookup_key = (parent.unwrap_or(usize::MAX), label.clone());
        if let Some(index) = self.child_lookup.get(&lookup_key) {
            return *index;
        }
        let parent_id = parent
            .map(|index| self.nodes[index].id.as_str())
            .unwrap_or("root")
            .to_owned();
        let id = node_id(&parent_id, level_index, level, &label);
        let breadcrumb = parent
            .map(|index| format!("{} / {label}", self.nodes[index].breadcrumb))
            .unwrap_or_else(|| label.clone());
        let index = self.nodes.len();
        self.nodes.push(ClassificationNode {
            id: id.clone(),
            parent,
            label,
            breadcrumb,
            children: Vec::new(),
            leaves: Vec::new(),
        });
        self.child_lookup.insert(lookup_key, index);
        if let Some(parent) = parent {
            self.nodes[parent].children.push(index);
        }
        index
    }

    fn node_read_model(&self, index: usize) -> HierarchyNodeReadModel {
        let node = &self.nodes[index];
        HierarchyNodeReadModel::virtual_node(
            node.id.clone(),
            node.parent.map(|parent| self.nodes[parent].id.clone()),
            node.label.clone(),
            node.breadcrumb.clone(),
            !node.children.is_empty() || !node.leaves.is_empty(),
        )
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

fn indexed_properties<'a>(
    entity: &'a EntitySnapshot,
    requested: &HashSet<&str>,
) -> HashMap<&'a str, &'a SemanticProperty> {
    entity
        .properties
        .iter()
        .filter(|property| requested.contains(property.name.as_str()))
        .map(|property| (property.name.as_str(), property))
        .collect()
}

fn group_value(
    entity: &EntitySnapshot,
    field: &BimFieldKey,
    properties: &HashMap<&str, &SemanticProperty>,
) -> String {
    let value = match field {
        BimFieldKey::Category => entity
            .semantic
            .bim_classification
            .category
            .as_deref()
            .map(Cow::Borrowed),
        BimFieldKey::Family => entity
            .semantic
            .bim_classification
            .family_name
            .as_deref()
            .map(Cow::Borrowed),
        BimFieldKey::Type => entity
            .semantic
            .bim_classification
            .type_name
            .as_deref()
            .map(Cow::Borrowed),
        BimFieldKey::Property(name) => properties
            .get(name.as_str())
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
) -> HierarchyNodeId {
    let mut input = Vec::with_capacity(parent_id.len() + level.id.len() + label.len() + 32);
    input.extend_from_slice(parent_id.as_bytes());
    input.push(0);
    input.extend_from_slice(level.id.as_bytes());
    input.push(0);
    input.extend_from_slice(level.field.stable_key().as_bytes());
    input.push(0);
    input.extend_from_slice(label.as_bytes());
    HierarchyNodeId::new(format!(
        "bim-group-{level_index}-{}",
        blake3::hash(&input).to_hex()
    ))
}

fn leaf_id(parent_id: &HierarchyNodeId, key: &EntityKey) -> HierarchyNodeId {
    let mut input = Vec::with_capacity(parent_id.as_str().len() + key.as_str().len() + 1);
    input.extend_from_slice(parent_id.as_str().as_bytes());
    input.push(0);
    input.extend_from_slice(key.as_str().as_bytes());
    HierarchyNodeId::new(format!("bim-leaf-{}", blake3::hash(&input).to_hex()))
}

/// The classification tree and hierarchy search share this exact name.
/// `SemanticInfo::bim` contains source-neutral identities populated by the
/// observed connector adapter. It is intentionally preferred over `EntityKey`,
/// whose value may be an IFC, application, path, or synthetic identity and
/// therefore is not necessarily a user-facing element ID.
pub(super) fn projected_entity_name(entity: &EntitySnapshot) -> String {
    let element_id = entity
        .semantic
        .bim
        .element_id
        .as_deref()
        .and_then(non_empty);
    let family = entity
        .semantic
        .bim
        .family_name
        .as_deref()
        .and_then(non_empty);
    match (element_id, family) {
        (Some(element_id), Some(family)) => format!("{element_id}-{family}"),
        (Some(element_id), None) => element_id.to_owned(),
        (None, Some(family)) => family.to_owned(),
        (None, None) => entity
            .semantic
            .display_name
            .as_deref()
            .and_then(non_empty)
            .unwrap_or(entity.prim_path.as_str())
            .to_owned(),
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}
