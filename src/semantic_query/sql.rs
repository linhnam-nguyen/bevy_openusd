//! Shared SQL builder for both semantic database modes.

use super::{GroupField, SemanticFilter, SemanticQuery, SortField};

pub(crate) fn build_where(query: &SemanticQuery, snapshot_id: &str) -> (String, Vec<turso::Value>) {
    let mut clauses = vec!["e.snapshot_id = ?".to_owned()];
    let mut params = vec![turso::Value::Text(snapshot_id.to_owned())];
    if let Some(text) = query
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        clauses.push(
            "(LOWER(COALESCE(e.prim_path, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.display_name, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.category, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.family, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.type_name, '')) LIKE LOWER('%' || ? || '%')
             OR LOWER(COALESCE(e.type_id, '')) LIKE LOWER('%' || ? || '%'))"
                .to_owned(),
        );
        params.extend((0..6).map(|_| turso::Value::Text(text.to_owned())));
    }
    for filter in &query.filters {
        match filter {
            SemanticFilter::DisplayNameContains(value) => {
                clauses.push(
                    "LOWER(COALESCE(e.display_name, '')) LIKE LOWER('%' || ? || '%')".to_owned(),
                );
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::CategoryEquals(value) => {
                clauses.push("e.category = ?".to_owned());
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::FamilyEquals(value) => {
                clauses.push("e.family = ?".to_owned());
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::TypeEquals(value) => {
                clauses.push("e.type_name = ?".to_owned());
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::PropertyTextEquals { name, value } => {
                clauses.push(
                    "EXISTS (SELECT 1 FROM properties p
                     WHERE p.snapshot_id = e.snapshot_id AND p.entity_key = e.entity_key
                       AND p.name = ? AND p.value_text = ?)"
                        .to_owned(),
                );
                params.push(turso::Value::Text(name.clone()));
                params.push(turso::Value::Text(value.clone()));
            }
            SemanticFilter::PropertyNumberRange { name, min, max } => {
                let mut clause = "EXISTS (SELECT 1 FROM properties p
                     WHERE p.snapshot_id = e.snapshot_id AND p.entity_key = e.entity_key
                       AND p.name = ?"
                    .to_owned();
                params.push(turso::Value::Text(name.clone()));
                if let Some(min) = min {
                    clause.push_str(" AND (p.value_real >= ? OR p.value_integer >= ?)");
                    params.push(turso::Value::Real(*min));
                    params.push(turso::Value::Real(*min));
                }
                if let Some(max) = max {
                    clause.push_str(" AND (p.value_real <= ? OR p.value_integer <= ?)");
                    params.push(turso::Value::Real(*max));
                    params.push(turso::Value::Real(*max));
                }
                clause.push(')');
                clauses.push(clause);
            }
        }
    }
    (format!("WHERE {}", clauses.join(" AND ")), params)
}

pub(crate) fn append_order_by(sql: &mut String, query: &SemanticQuery) {
    sql.push_str(" ORDER BY ");
    if query.sort.is_empty() {
        sql.push_str("e.prim_path ASC");
        return;
    }
    for (index, rule) in query.sort.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str(sort_column(rule.field));
        sql.push_str(if rule.descending { " DESC" } else { " ASC" });
    }
}

pub(crate) fn group_column(field: GroupField) -> &'static str {
    match field {
        GroupField::Category => "category",
        GroupField::Family => "family",
        GroupField::TypeName => "type_name",
    }
}

fn sort_column(field: SortField) -> &'static str {
    match field {
        SortField::DisplayName => "e.display_name",
        SortField::PrimPath => "e.prim_path",
        SortField::Category => "e.category",
        SortField::Family => "e.family",
        SortField::TypeName => "e.type_name",
        SortField::TranslationX => "e.tx_mm",
    }
}

pub(crate) fn limit(limit: u32) -> u32 {
    if limit == 0 { 100 } else { limit.min(1_000) }
}
