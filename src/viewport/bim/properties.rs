//! BIM property projection for one or more selected semantic entities.

use std::collections::{BTreeMap, HashMap};

use usd_model::{MeasurementMetadata, SemanticProperty};
use usd_semantic::UnitRegistry;
use viewport_protocol::{
    BimPropertiesReadModel, BimPropertyGroupId, BimPropertyGroupReadModel, BimPropertyReadModel,
    BimUnitOption, CommonValue, SelectionReadModel,
};

use super::{BimQueryError, BimReadPolicy, BimReadService};

pub(super) fn read_properties<'snapshot>(
    service: &BimReadService<'snapshot>,
    selection: &SelectionReadModel,
    selection_revision: u64,
    policy: BimReadPolicy,
) -> Result<BimPropertiesReadModel, BimQueryError> {
    selection.validate()?;
    let targets = &selection.targets;
    if targets.len() > viewport_protocol::MAX_BIM_SELECTION_TARGETS {
        return Err(BimQueryError::Invalid(
            viewport_protocol::ProtocolValidationError::InvalidInput {
                field: "bim.selection.targets",
            },
        ));
    }
    let mut entities = Vec::with_capacity(targets.len());
    for target in targets {
        entities.push(service.entity_for_anchor(target)?);
    }
    if entities.is_empty() {
        return Ok(BimPropertiesReadModel {
            targets: Vec::new(),
            selection_revision,
            groups: Vec::new(),
        });
    }

    let mut common: BTreeMap<String, Vec<&SemanticProperty>> = BTreeMap::new();
    for property in &entities[0].properties {
        common
            .entry(property.name.clone())
            .or_default()
            .push(property);
    }
    for entity in entities.iter().skip(1) {
        let properties: HashMap<&str, &SemanticProperty> = entity
            .properties
            .iter()
            .map(|property| (property.name.as_str(), property))
            .collect();
        common.retain(|name, entries| {
            let Some(property) = properties.get(name.as_str()) else {
                return false;
            };
            entries.push(*property);
            true
        });
    }

    let mut grouped: BTreeMap<BimPropertyGroupId, Vec<BimPropertyReadModel>> = BTreeMap::new();
    for (key, values) in common {
        let property = project_property(key, &values, policy);
        grouped.entry(property.group_id).or_default().push(property);
    }
    let groups = grouped
        .into_iter()
        .map(|(id, properties)| BimPropertyGroupReadModel {
            id,
            name: property_group_name(id).to_owned(),
            editable_group: policy.allow_value_edit,
            properties,
        })
        .collect();
    Ok(BimPropertiesReadModel {
        targets: targets.to_vec(),
        selection_revision,
        groups,
    })
}

fn project_property(
    key: String,
    values: &[&SemanticProperty],
    policy: BimReadPolicy,
) -> BimPropertyReadModel {
    let first = values.first().expect("common property has one value");
    let same_value = values.iter().all(|property| property.value == first.value);
    let measurement = common_measurement(values);
    let units = measurement
        .as_ref()
        .map(|metadata| {
            let registry = UnitRegistry::global();
            registry
                .units_for_quantity(&metadata.quantity)
                .into_iter()
                .filter_map(|unit| {
                    let definition = registry.definition(&unit)?;
                    Some(BimUnitOption {
                        label: unit.as_str().to_owned(),
                        unit,
                        scale_to_canonical: definition.scale_to_canonical(),
                        offset_to_canonical: definition.offset_to_canonical(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let group_id = property_group_id(&key);
    let current_display_unit = measurement.as_ref().and_then(|metadata| {
        metadata
            .source_unit
            .clone()
            .or_else(|| Some(metadata.canonical_unit.clone()))
    });

    BimPropertyReadModel {
        key,
        group_id,
        value: if same_value {
            CommonValue::Same(first.value.clone())
        } else {
            CommonValue::Multiple
        },
        target_values: values
            .iter()
            .map(|property| property.value.clone())
            .collect(),
        measurement,
        units,
        current_display_unit,
        editable: policy.allow_value_edit,
    }
}

fn property_group_id(_key: &str) -> BimPropertyGroupId {
    // SemanticProperty currently carries no validated source-group metadata.
    // Keep all such rows in the explicit fallback group until extraction
    // promotes observed connector metadata into the semantic model.
    BimPropertyGroupId::SourceFallback
}

fn property_group_name(group_id: BimPropertyGroupId) -> &'static str {
    match group_id {
        BimPropertyGroupId::Semantic => "Semantic",
        BimPropertyGroupId::SourceFallback => "<Ungrouped>",
    }
}

fn common_measurement(values: &[&SemanticProperty]) -> Option<MeasurementMetadata> {
    let first = values.first()?.measurement.as_ref();
    if values
        .iter()
        .all(|property| property.measurement.as_ref() == first)
    {
        first.cloned()
    } else {
        None
    }
}
