//! Bounded BIM regex search and replacement preview.

use std::collections::BTreeMap;

use regex::Regex;
use usd_model::{CanonicalValue, MeasurementMetadata};
use viewport_protocol::{
    BimObjectMatch, BimPropertyNameMatch, BimPropertyValueMatch, BimReplacementPreviewRow,
    BimSearchQuery, BimSearchResult,
};

use super::classification::canonical_value_text;
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
    let matches = grouped
        .into_iter()
        .map(
            |(name, (measurement, consistent, object_count))| BimPropertyNameMatch {
                name,
                measurement: consistent.then_some(measurement).flatten(),
                object_count,
            },
        )
        .collect();
    let (offset, total, matches, has_more) = page_items(matches, page);
    Ok(BimSearchResult::PropertyNames {
        offset,
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
    let matches = grouped
        .into_iter()
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
        .collect();
    let (offset, total, matches, has_more) = page_items(matches, page);
    Ok(BimSearchResult::PropertyValues {
        offset,
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
    let mut matches = Vec::new();
    for entity in service.entities() {
        for property in &entity.properties {
            if property.name != property_name {
                continue;
            }
            let Some(display_value) = canonical_value_text(&property.value) else {
                continue;
            };
            if regex.is_match(&display_value) {
                matches.push(BimObjectMatch {
                    anchor: BimReadService::anchor_for_entity(entity),
                    label: entity_label(entity),
                    property: property.name.clone(),
                    value: property.value.clone(),
                    display_value: display_value.into_owned(),
                });
            }
        }
    }
    matches.sort_unstable_by(|left, right| left.anchor.prim_path.cmp(&right.anchor.prim_path));
    let (offset, total, matches, has_more) = page_items(matches, page);
    Ok(BimSearchResult::Objects {
        offset,
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
    let mut rows = Vec::new();
    for entity in service.entities() {
        for property in &entity.properties {
            if property.name != property_name {
                continue;
            }
            let Some(old_value) = canonical_value_text(&property.value) else {
                continue;
            };
            if regex.is_match(&old_value) {
                rows.push(BimReplacementPreviewRow {
                    anchor: BimReadService::anchor_for_entity(entity),
                    label: entity_label(entity),
                    property: property.name.clone(),
                    proposed_value: regex.replace(&old_value, replacement).into_owned(),
                    old_value: old_value.into_owned(),
                });
            }
        }
    }
    rows.sort_unstable_by(|left, right| left.anchor.prim_path.cmp(&right.anchor.prim_path));
    let (offset, total, rows, has_more) = page_items(rows, page);
    Ok(BimSearchResult::ReplacementPreview {
        offset,
        total,
        rows,
        has_more,
    })
}

fn page_items<T>(
    items: Vec<T>,
    page: viewport_protocol::BimPageRequest,
) -> (u32, u32, Vec<T>, bool) {
    let total = items.len() as u32;
    let start = (page.offset as usize).min(items.len());
    let end = start.saturating_add(page.limit as usize).min(items.len());
    let rows = items
        .into_iter()
        .skip(start)
        .take(end - start)
        .collect::<Vec<_>>();
    let has_more = page.offset.saturating_add(rows.len() as u32) < total;
    (page.offset, total, rows, has_more)
}

fn entity_label(entity: &usd_model::EntitySnapshot) -> String {
    entity
        .semantic
        .display_name
        .as_deref()
        .unwrap_or(entity.prim_path.as_str())
        .to_owned()
}
