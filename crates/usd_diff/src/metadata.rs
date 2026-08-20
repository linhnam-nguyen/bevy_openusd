//! Property-level metadata comparison.

use std::collections::{BTreeMap, BTreeSet};

use usd_model::{CanonicalValue, EntitySnapshot, SemanticProperty};

/// One metadata field that was added, removed, or changed.
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataChange {
    pub name: String,
    pub old: Option<CanonicalValue>,
    pub new: Option<CanonicalValue>,
}

/// Compare metadata details for one matched entity.
///
/// Callers should first compare `metadata_hash`. This function intentionally
/// performs the more expensive property-level pass only after that hash has
/// changed.
pub fn metadata_changes(old: &EntitySnapshot, new: &EntitySnapshot) -> Vec<MetadataChange> {
    let old_values = metadata_values(old);
    let new_values = metadata_values(new);
    let names = old_values
        .keys()
        .chain(new_values.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    names
        .into_iter()
        .filter_map(|name| {
            let old_value = old_values.get(&name).cloned().unwrap_or(None);
            let new_value = new_values.get(&name).cloned().unwrap_or(None);
            (old_value != new_value).then_some(MetadataChange {
                name,
                old: old_value,
                new: new_value,
            })
        })
        .collect()
}

fn metadata_values(entity: &EntitySnapshot) -> BTreeMap<String, Option<CanonicalValue>> {
    let mut values = BTreeMap::new();
    values.insert(
        "semantic.category".to_owned(),
        entity.semantic.category.clone().map(CanonicalValue::Text),
    );
    values.insert(
        "semantic.family".to_owned(),
        entity.semantic.family.clone().map(CanonicalValue::Text),
    );
    values.insert(
        "semantic.type_name".to_owned(),
        entity.semantic.type_name.clone().map(CanonicalValue::Text),
    );
    values.insert(
        "semantic.type_id".to_owned(),
        entity.semantic.type_id.clone().map(CanonicalValue::Text),
    );
    values.insert(
        "semantic.display_name".to_owned(),
        entity
            .semantic
            .display_name
            .clone()
            .map(CanonicalValue::Text),
    );

    for SemanticProperty { name, value } in &entity.properties {
        values.insert(format!("property.{name}"), Some(value.clone()));
    }

    values
}
