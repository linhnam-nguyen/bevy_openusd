use std::collections::{BTreeMap, BTreeSet};

use usd_model::EntitySnapshot;
use viewport_protocol::{
    BimClassificationFieldCatalogue, BimClassificationFieldDescriptor, BimFieldKey,
    BimPropertyScope, MAX_BIM_CLASSIFICATION_FIELDS,
};

use super::BimReadService;

impl<'snapshot> BimReadService<'snapshot> {
    /// Builds the bounded, model-wide field catalogue from BIM-eligible
    /// entities only. The caller supplies the semantic revision so the
    /// browser can invalidate this immutable list without coupling it to
    /// selection-scoped property reads.
    pub(crate) fn classification_field_catalogue(
        &self,
        semantic_revision: u64,
    ) -> BimClassificationFieldCatalogue {
        let mut entities = self.entities().collect::<Vec<_>>();
        entities.sort_unstable_by(|left, right| left.key.as_str().cmp(right.key.as_str()));
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

        for entity in entities {
            let mut property_names = entity
                .properties
                .iter()
                .map(|property| property.name.trim())
                .filter(|name| {
                    !name.is_empty()
                        && name.len() <= viewport_protocol::MAX_BIM_FIELD_KEY_BYTES
                        && !name.contains('\0')
                })
                .collect::<Vec<_>>();
            property_names.sort_unstable();
            property_names.dedup();

            for name in property_names {
                let field = BimFieldKey::Property(name.to_owned());
                if represented_properties.contains(name) {
                    continue;
                }
                if fields.len() < MAX_BIM_CLASSIFICATION_FIELDS {
                    let descriptor = usd_semantic::nvidia_revit_property_descriptor(name);
                    fields.entry(field).or_insert_with(|| {
                        BimClassificationFieldDescriptor::new(
                            BimFieldKey::property(name),
                            descriptor.label,
                            descriptor.scope,
                        )
                    });
                }
            }
        }

        BimClassificationFieldCatalogue {
            semantic_revision,
            fields: fields.into_values().collect(),
        }
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
