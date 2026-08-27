//! BIM property projection for one or more selected semantic entities.

use std::collections::{BTreeMap, HashMap};

use usd_model::{MeasurementMetadata, SemanticProperty};
use usd_semantic::UnitRegistry;
use viewport_protocol::{
    BimPropertiesReadModel, BimPropertyGroupId, BimPropertyReadModel, BimUnitOption, CommonValue,
    SelectionReadModel,
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
            properties: Vec::new(),
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

    let properties = common
        .into_iter()
        .map(|(key, values)| project_property(key, &values, policy))
        .collect();
    Ok(BimPropertiesReadModel {
        targets: targets.to_vec(),
        selection_revision,
        properties,
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
            UnitRegistry::global()
                .units_for_quantity(&metadata.quantity)
                .into_iter()
                .map(|unit| BimUnitOption {
                    label: unit.as_str().to_owned(),
                    unit,
                })
                .collect()
        })
        .unwrap_or_default();

    BimPropertyReadModel {
        key,
        group_id: BimPropertyGroupId::Semantic,
        value: if same_value {
            CommonValue::Same(first.value.clone())
        } else {
            CommonValue::Multiple
        },
        measurement,
        units,
        editable: policy.allow_value_edit,
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
