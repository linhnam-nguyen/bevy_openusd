//! Bounded BIM regex search and replacement preview.

use std::collections::{BTreeMap, BinaryHeap};

use regex::Regex;
use usd_model::{CanonicalValue, MeasurementMetadata};
use viewport_protocol::{
    BimObjectMatch, BimPageRequest, BimPropertyNameMatch, BimPropertyValueMatch,
    BimReplacementPreviewRow, BimSearchQuery, BimSearchResult, MAX_BIM_SEARCH_GROUPS,
};

use super::classification::{canonical_value_text, projected_entity_name};
use super::{BimQueryError, BimReadService};

pub(super) fn execute(
    service: &BimReadService<'_>,
    query: &BimSearchQuery,
) -> Result<BimSearchResult, BimQueryError> {
    query.validate()?;
    let pattern = match query {
        BimSearchQuery::PropertyNameRegex { pattern, .. }
        | BimSearchQuery::PropertyValueRegex { pattern, .. }
        | BimSearchQuery::ObjectPropertyMatch { pattern, .. }
        | BimSearchQuery::ReplacementPreview { pattern, .. } => pattern,
    };
    let regex =
        Regex::new(pattern).map_err(|error| BimQueryError::InvalidRegex(error.to_string()))?;

    match query {
        BimSearchQuery::PropertyNameRegex { page, .. } => property_names(service, &regex, *page),
        BimSearchQuery::PropertyValueRegex { page, .. } => property_values(service, &regex, *page),
        BimSearchQuery::ObjectPropertyMatch { property, page, .. } => {
            object_matches(service, property, &regex, *page)
        }
        BimSearchQuery::ReplacementPreview {
            property,
            replacement,
            page,
            ..
        } => replacement_preview(service, property, &regex, replacement, *page),
    }
}

fn property_names(
    service: &BimReadService<'_>,
    regex: &Regex,
    page: viewport_protocol::BimPageRequest,
) -> Result<BimSearchResult, BimQueryError> {
    let mut grouped: BTreeMap<String, (Option<MeasurementMetadata>, bool, u32)> = BTreeMap::new();
    for entity in service.entities() {
        for property in &entity.properties {
            if !regex.is_match(&property.name) {
                continue;
            }
            if !grouped.contains_key(&property.name) && grouped.len() >= MAX_BIM_SEARCH_GROUPS {
                return Err(BimQueryError::TooManyResults {
                    kind: "property-name",
                    limit: MAX_BIM_SEARCH_GROUPS,
                });
            }
            let entry = grouped.entry(property.name.clone()).or_insert((
                property.measurement.clone(),
                true,
                0,
            ));
            if entry.1 && entry.0 != property.measurement {
                entry.1 = false;
            }
            entry.2 = entry.2.saturating_add(1);
        }
    }
    let total = grouped.len() as u32;
    let matches = grouped
        .into_iter()
        .skip(page.offset as usize)
        .take(page.limit as usize)
        .map(
            |(name, (measurement, consistent, object_count))| BimPropertyNameMatch {
                name,
                measurement: consistent.then_some(measurement).flatten(),
                object_count,
            },
        )
        .collect::<Vec<_>>();
    let has_more = page.offset.saturating_add(matches.len() as u32) < total;
    Ok(BimSearchResult::PropertyNames {
        offset: page.offset,
        total,
        matches,
        has_more,
    })
}

fn property_values(
    service: &BimReadService<'_>,
    regex: &Regex,
    page: viewport_protocol::BimPageRequest,
) -> Result<BimSearchResult, BimQueryError> {
    let mut grouped: BTreeMap<
        (String, String),
        (CanonicalValue, Option<MeasurementMetadata>, bool, u32),
    > = BTreeMap::new();
    for entity in service.entities() {
        for property in &entity.properties {
            let Some(display_value) = canonical_value_text(&property.value) else {
                continue;
            };
            if !regex.is_match(&display_value) {
                continue;
            }
            let key = (property.name.clone(), display_value.into_owned());
            if !grouped.contains_key(&key) && grouped.len() >= MAX_BIM_SEARCH_GROUPS {
                return Err(BimQueryError::TooManyResults {
                    kind: "property-value",
                    limit: MAX_BIM_SEARCH_GROUPS,
                });
            }
            let entry = grouped.entry(key).or_insert((
                property.value.clone(),
                property.measurement.clone(),
                true,
                0,
            ));
            if entry.2 && entry.1 != property.measurement {
                entry.2 = false;
            }
            entry.3 = entry.3.saturating_add(1);
        }
    }
    let total = grouped.len() as u32;
    let matches = grouped
        .into_iter()
        .skip(page.offset as usize)
        .take(page.limit as usize)
        .map(
            |((name, display_value), (value, measurement, consistent, object_count))| {
                BimPropertyValueMatch {
                    name,
                    value,
                    display_value,
                    measurement: consistent.then_some(measurement).flatten(),
                    object_count,
                }
            },
        )
        .collect::<Vec<_>>();
    let has_more = page.offset.saturating_add(matches.len() as u32) < total;
    Ok(BimSearchResult::PropertyValues {
        offset: page.offset,
        total,
        matches,
        has_more,
    })
}

fn object_matches(
    service: &BimReadService<'_>,
    property_name: &str,
    regex: &Regex,
    page: viewport_protocol::BimPageRequest,
) -> Result<BimSearchResult, BimQueryError> {
    let mut page_items = BoundedPage::new(page);
    for entity in service.entities() {
        for (property_index, property) in entity.properties.iter().enumerate() {
            if property.name != property_name {
                continue;
            }
            let Some(display_value) = canonical_value_text(&property.value) else {
                continue;
            };
            if regex.is_match(&display_value) {
                page_items.push(
                    object_order_key(entity, property_index),
                    BimObjectMatch {
                        anchor: BimReadService::anchor_for_entity(entity),
                        label: entity_label(entity),
                        property: property.name.clone(),
                        value: property.value.clone(),
                        display_value: display_value.into_owned(),
                    },
                );
            }
        }
    }
    let (total, matches, has_more) = page_items.finish(page);
    Ok(BimSearchResult::Objects {
        offset: page.offset,
        total,
        matches,
        has_more,
    })
}

fn replacement_preview(
    service: &BimReadService<'_>,
    property_name: &str,
    regex: &Regex,
    replacement: &str,
    page: viewport_protocol::BimPageRequest,
) -> Result<BimSearchResult, BimQueryError> {
    let mut page_items = BoundedPage::new(page);
    for entity in service.entities() {
        for (property_index, property) in entity.properties.iter().enumerate() {
            if property.name != property_name {
                continue;
            }
            let Some(old_value) = canonical_value_text(&property.value) else {
                continue;
            };
            if regex.is_match(&old_value) {
                let proposed_value = regex.replace(old_value.as_ref(), replacement).into_owned();
                page_items.push(
                    object_order_key(entity, property_index),
                    BimReplacementPreviewRow {
                        anchor: BimReadService::anchor_for_entity(entity),
                        label: entity_label(entity),
                        property: property.name.clone(),
                        proposed_value: proposed_value.clone(),
                        expected_old_value: property.value.clone(),
                        proposed_canonical_value: replacement_canonical_value(
                            &property.value,
                            &proposed_value,
                        ),
                        measurement: property.measurement.clone(),
                        old_value: old_value.into_owned(),
                    },
                );
            }
        }
    }
    let (total, rows, has_more) = page_items.finish(page);
    Ok(BimSearchResult::ReplacementPreview {
        offset: page.offset,
        total,
        rows,
        has_more,
    })
}

fn replacement_canonical_value(
    old_value: &CanonicalValue,
    proposed_value: &str,
) -> Option<CanonicalValue> {
    match old_value {
        CanonicalValue::Null => (proposed_value == "null").then_some(CanonicalValue::Null),
        CanonicalValue::Bool(_) => proposed_value.parse().ok().map(CanonicalValue::Bool),
        CanonicalValue::Integer(_) => proposed_value.parse().ok().map(CanonicalValue::Integer),
        CanonicalValue::Real(_) => proposed_value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(CanonicalValue::Real),
        CanonicalValue::Text(_) => Some(CanonicalValue::Text(proposed_value.to_owned())),
        CanonicalValue::TextArray(_) => serde_json::from_str(proposed_value)
            .ok()
            .map(CanonicalValue::TextArray),
        CanonicalValue::NumberArray(_) => serde_json::from_str::<Vec<f64>>(proposed_value)
            .ok()
            .filter(|values| values.iter().all(|value| value.is_finite()))
            .map(CanonicalValue::NumberArray),
        CanonicalValue::Json(_) => None,
    }
}

struct BoundedPage<K, T> {
    candidates: BinaryHeap<Ranked<K, T>>,
    capacity: usize,
    total: u32,
    next_ordinal: u64,
}

impl<K: Ord, T> BoundedPage<K, T> {
    fn new(page: BimPageRequest) -> Self {
        Self {
            candidates: BinaryHeap::with_capacity(page.offset.saturating_add(page.limit) as usize),
            capacity: page.offset.saturating_add(page.limit) as usize,
            total: 0,
            next_ordinal: 0,
        }
    }

    fn push(&mut self, key: K, value: T) {
        let ordinal = self.next_ordinal;
        self.next_ordinal = self.next_ordinal.saturating_add(1);
        self.total = self.total.saturating_add(1);
        if self.capacity == 0 {
            return;
        }
        let candidate = Ranked {
            key,
            ordinal,
            value,
        };
        let replace = self.candidates.peek().is_some_and(|worst| {
            candidate
                .key
                .cmp(&worst.key)
                .then_with(|| candidate.ordinal.cmp(&worst.ordinal))
                .is_lt()
        });
        if self.candidates.len() < self.capacity {
            self.candidates.push(candidate);
        } else if replace {
            let _ = self.candidates.pop();
            self.candidates.push(candidate);
        }
    }

    fn finish(self, page: BimPageRequest) -> (u32, Vec<T>, bool) {
        let mut candidates = self.candidates.into_vec();
        candidates.sort_unstable_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| left.ordinal.cmp(&right.ordinal))
        });
        let rows = candidates
            .into_iter()
            .skip(page.offset as usize)
            .take(page.limit as usize)
            .map(|candidate| candidate.value)
            .collect::<Vec<_>>();
        let has_more = page.offset.saturating_add(rows.len() as u32) < self.total;
        (self.total, rows, has_more)
    }
}

struct Ranked<K, T> {
    key: K,
    ordinal: u64,
    value: T,
}

impl<K: Ord, T> Ord for Ranked<K, T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| self.ordinal.cmp(&other.ordinal))
    }
}

impl<K: Ord, T> PartialOrd for Ranked<K, T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: Ord, T> PartialEq for Ranked<K, T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.ordinal == other.ordinal
    }
}

impl<K: Ord, T> Eq for Ranked<K, T> {}

fn object_order_key(
    entity: &usd_model::EntitySnapshot,
    property_index: usize,
) -> (String, String, usize) {
    (
        entity.prim_path.clone(),
        entity.key.as_str().to_owned(),
        property_index,
    )
}

fn entity_label(entity: &usd_model::EntitySnapshot) -> String {
    projected_entity_name(entity)
}
