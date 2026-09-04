//! Snapshot-owned immutable indexes shared by BIM read operations.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use usd_model::{EntityKey, EntitySnapshot, SemanticSnapshot, SnapshotId};
use viewport_protocol::{
    BimClassificationFieldCatalogue, BimClassificationFieldDescriptor, BimFieldKey,
    BimPropertyScope, ClassificationRecipe, MAX_BIM_CLASSIFICATION_FIELDS,
};

use super::classification::ClassificationIndex;

/// One occurrence of a property name in the immutable snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PropertyPosting {
    pub(super) entity: usize,
    pub(super) property: usize,
}

/// Derived BIM data for one immutable semantic snapshot.
///
/// The index owns only stable lookup data and entity/property positions. The
/// semantic snapshot remains the source of truth for values and protocol
/// projection strings.
#[derive(Debug)]
pub(crate) struct BimReadIndex {
    pub(crate) snapshot_id: SnapshotId,
    entity_order: Box<[EntityKey]>,
    by_path: HashMap<String, EntityKey>,
    property_ids: HashMap<String, usize>,
    property_names: Box<[String]>,
    property_postings: Box<[Box<[PropertyPosting]>]>,
    field_catalogue: BimClassificationFieldCatalogue,
    classification_cache: Mutex<Vec<(ClassificationRecipe, Arc<ClassificationIndex>)>>,
}

impl BimReadIndex {
    pub(crate) fn build(snapshot: &SemanticSnapshot) -> Self {
        let mut all_entities = snapshot.entities.values().collect::<Vec<_>>();
        all_entities.sort_unstable_by(|left, right| {
            left.prim_path
                .cmp(&right.prim_path)
                .then_with(|| left.key.cmp(&right.key))
        });

        let by_path = all_entities
            .iter()
            .map(|entity| (entity.prim_path.clone(), entity.key.clone()))
            .collect::<HashMap<_, _>>();
        let entity_order = all_entities
            .iter()
            .filter(|entity| entity.semantic.is_bim_entity())
            .map(|entity| entity.key.clone())
            .collect::<Vec<_>>()
            .into_boxed_slice();

        let mut property_ids = HashMap::new();
        let mut property_names = Vec::new();
        let mut postings = Vec::<Vec<PropertyPosting>>::new();
        for (entity_index, key) in entity_order.iter().enumerate() {
            let entity = snapshot
                .entities
                .get(key)
                .expect("BIM entity order must belong to its snapshot");
            for (property_index, property) in entity.properties.iter().enumerate() {
                let property_id = match property_ids.get(&property.name).copied() {
                    Some(property_id) => property_id,
                    None => {
                        let property_id = property_names.len();
                        property_ids.insert(property.name.clone(), property_id);
                        property_names.push(property.name.clone());
                        postings.push(Vec::new());
                        property_id
                    }
                };
                postings[property_id].push(PropertyPosting {
                    entity: entity_index,
                    property: property_index,
                });
            }
        }

        Self {
            snapshot_id: snapshot.snapshot_id.clone(),
            entity_order,
            by_path,
            property_ids,
            property_names: property_names.into_boxed_slice(),
            property_postings: postings
                .into_iter()
                .map(Vec::into_boxed_slice)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            field_catalogue: build_field_catalogue(snapshot),
            classification_cache: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn entity_order(&self) -> &[EntityKey] {
        &self.entity_order
    }

    pub(super) fn entity_key_for_path(&self, path: &str) -> Option<&EntityKey> {
        self.by_path.get(path)
    }

    pub(super) fn property_id(&self, name: &str) -> Option<usize> {
        self.property_ids.get(name).copied()
    }

    pub(super) fn property_names(&self) -> &[String] {
        &self.property_names
    }

    pub(super) fn property_postings(&self, property_id: usize) -> &[PropertyPosting] {
        self.property_postings
            .get(property_id)
            .map_or(&[], AsRef::as_ref)
    }

    pub(super) fn field_catalogue(
        &self,
        semantic_revision: u64,
    ) -> BimClassificationFieldCatalogue {
        let mut catalogue = self.field_catalogue.clone();
        catalogue.semantic_revision = semantic_revision;
        catalogue
    }

    pub(super) fn classification(
        &self,
        snapshot: &SemanticSnapshot,
        recipe: &ClassificationRecipe,
    ) -> Arc<ClassificationIndex> {
        let mut cache = self
            .classification_cache
            .lock()
            .expect("BIM classification cache mutex is not poisoned");
        if let Some((_, index)) = cache.iter().find(|(cached, _)| cached == recipe) {
            return Arc::clone(index);
        }
        let index = Arc::new(ClassificationIndex::build(snapshot, recipe));
        cache.push((recipe.clone(), Arc::clone(&index)));
        index
    }

    #[cfg(test)]
    pub(super) fn classification_cache_len(&self) -> usize {
        self.classification_cache
            .lock()
            .expect("BIM classification cache mutex is not poisoned")
            .len()
    }
}

fn build_field_catalogue(snapshot: &SemanticSnapshot) -> BimClassificationFieldCatalogue {
    let entities = snapshot
        .entities
        .values()
        .filter(|entity| entity.semantic.is_bim_entity())
        .collect::<Vec<_>>();
    let mut represented_properties = BTreeSet::new();
    for entity in &entities {
        let classification = &entity.semantic.bim_classification;
        represented_properties.extend(
            [
                classification.category_property.as_deref(),
                classification.family_name_property.as_deref(),
                classification.type_name_property.as_deref(),
            ]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|property| !property.is_empty())
            .map(str::to_owned),
        );
    }

    let category_property = verified_classification_property(&entities, |classification| {
        classification.category_property.as_deref()
    });
    let family_property = verified_classification_property(&entities, |classification| {
        classification.family_name_property.as_deref()
    });
    let type_property = verified_classification_property(&entities, |classification| {
        classification.type_name_property.as_deref()
    });
    let mut fields = BTreeMap::new();
    if category_property.is_some() {
        fields.insert(
            BimFieldKey::Category,
            classification_descriptor(BimFieldKey::Category, "Category", category_property),
        );
    }
    if family_property.is_some() {
        fields.insert(
            BimFieldKey::Family,
            classification_descriptor(BimFieldKey::Family, "Family", family_property),
        );
    }
    if type_property.is_some() {
        fields.insert(
            BimFieldKey::Type,
            classification_descriptor(BimFieldKey::Type, "Type", type_property),
        );
    }

    let mut property_metadata: BTreeMap<String, Option<String>> = BTreeMap::new();
    for entity in entities {
        for property in &entity.properties {
            let name = property.name.trim();
            if name.is_empty()
                || name.len() > viewport_protocol::MAX_BIM_FIELD_KEY_BYTES
                || name.contains('\0')
            {
                continue;
            }
            property_metadata
                .entry(name.to_owned())
                .or_insert_with(|| property.display_name.clone());
        }
    }
    for (name, display_name) in property_metadata {
        let field = BimFieldKey::Property(name.clone());
        if represented_properties.contains(name.as_str()) || fields.contains_key(&field) {
            continue;
        }
        if fields.len() >= MAX_BIM_CLASSIFICATION_FIELDS {
            break;
        }
        let descriptor = usd_semantic::nvidia_revit_property_descriptor_with_display_name(
            &name,
            display_name.as_deref(),
        );
        fields.insert(
            field.clone(),
            BimClassificationFieldDescriptor::new(field, descriptor.label, descriptor.scope),
        );
    }

    BimClassificationFieldCatalogue {
        semantic_revision: 0,
        fields: fields.into_values().collect(),
    }
}

fn verified_classification_property<'a, F>(
    entities: &'a [&'a EntitySnapshot],
    source: F,
) -> Option<&'a str>
where
    F: Fn(&'a usd_model::BimClassificationInfo) -> Option<&'a str>,
{
    entities.iter().find_map(|entity| {
        let property = source(&entity.semantic.bim_classification)?;
        let normalized = property.trim();
        (!normalized.is_empty()
            && entity
                .properties
                .iter()
                .any(|candidate| candidate.name.trim() == normalized))
        .then_some(normalized)
    })
}

fn classification_descriptor(
    key: BimFieldKey,
    label: &'static str,
    source_property: Option<&str>,
) -> BimClassificationFieldDescriptor {
    let scope = source_property
        .map(usd_semantic::nvidia_revit_property_descriptor)
        .map_or(BimPropertyScope::Other, |descriptor| descriptor.scope);
    BimClassificationFieldDescriptor::new(key, label, scope)
}
