use anyhow::{Context, Result, anyhow};
use usd_model::{EntityKey, SnapshotId};

use super::TursoSemanticStore;
use super::entity::{nullable_integer, nullable_text};
use crate::project::semantic_store::query::{
    GroupField, SemanticFilter, SemanticGroup, SemanticQuery, SemanticQueryResult,
    SemanticQueryRow, SortField,
};

impl TursoSemanticStore {
    pub(super) async fn query_impl(
        &self,
        snapshot: &SnapshotId,
        query: &SemanticQuery,
    ) -> Result<SemanticQueryResult> {
        if self.get_snapshot_impl(snapshot).await?.is_none() {
            return Ok(SemanticQueryResult::default());
        }

        let (where_sql, where_params) = build_where(query, &snapshot.0);
        let count_sql = format!("SELECT COUNT(*) FROM entities e {where_sql}");
        let count_row = self
            .connection
            .query(&count_sql, turso::params_from_iter(where_params.clone()))
            .await
            .context("counting durable semantic query results")?
            .next()
            .await
            .context("reading durable semantic query count")?
            .ok_or_else(|| anyhow!("durable semantic count query returned no row"))?;
        let total = count_row
            .get::<i64>(0)
            .context("decoding durable semantic query count")? as u32;

        let mut sql = format!(
            "SELECT e.entity_key, e.prim_path, e.display_name, e.category,
                    e.family, e.type_name, e.tx_mm, e.ty_mm, e.tz_mm
             FROM entities e {where_sql}"
        );
        append_order_by(&mut sql, query);
        sql.push_str(" LIMIT ? OFFSET ?");
        let mut params = where_params;
        params.push(turso::Value::Integer(limit(query.limit) as i64));
        params.push(turso::Value::Integer(query.offset as i64));

        let mut rows = self
            .connection
            .query(&sql, turso::params_from_iter(params))
            .await
            .context("querying durable semantic entities")?;
        let mut result_rows = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .context("reading durable semantic entity row")?
        {
            result_rows.push(SemanticQueryRow {
                entity_key: EntityKey::from(row.get::<String>(0)?),
                prim_path: row.get(1)?,
                display_name: nullable_text(&row, 2)?,
                category: nullable_text(&row, 3)?,
                family: nullable_text(&row, 4)?,
                type_name: nullable_text(&row, 5)?,
                translation_mm: [
                    nullable_integer(&row, 6)?.unwrap_or_default(),
                    nullable_integer(&row, 7)?.unwrap_or_default(),
                    nullable_integer(&row, 8)?.unwrap_or_default(),
                ],
            });
        }

        let (_, group_params) = build_where(query, &snapshot.0);
        let groups = groups(&self.connection, query, &where_sql, &group_params).await?;
        let row_count = result_rows.len() as u32;
        Ok(SemanticQueryResult {
            total,
            rows: result_rows,
            groups,
            has_more: query.offset.saturating_add(row_count) < total,
        })
    }
}

pub(super) async fn groups(
    connection: &turso::Connection,
    query: &SemanticQuery,
    where_sql: &str,
    where_params: &[turso::Value],
) -> Result<Vec<SemanticGroup>> {
    let mut result = Vec::new();
    for field in &query.group_by {
        let column = group_column(*field);
        let sql = format!(
            "SELECT e.{column}, COUNT(*) FROM entities e {where_sql}
             GROUP BY e.{column} ORDER BY COUNT(*) DESC, e.{column} ASC"
        );
        let mut rows = connection
            .query(&sql, turso::params_from_iter(where_params.to_vec()))
            .await
            .with_context(|| format!("grouping durable semantic entities by {field:?}"))?;
        while let Some(row) = rows
            .next()
            .await
            .context("reading durable semantic group row")?
        {
            result.push(SemanticGroup {
                field: *field,
                value: nullable_text(&row, 0)?,
                count: row.get::<i64>(1)? as u32,
            });
        }
    }
    Ok(result)
}

pub(super) fn build_where(query: &SemanticQuery, snapshot_id: &str) -> (String, Vec<turso::Value>) {
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

pub(super) fn append_order_by(sql: &mut String, query: &SemanticQuery) {
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

pub(super) fn group_column(field: GroupField) -> &'static str {
    match field {
        GroupField::Category => "category",
        GroupField::Family => "family",
        GroupField::TypeName => "type_name",
    }
}

pub(super) fn sort_column(field: SortField) -> &'static str {
    match field {
        SortField::DisplayName => "e.display_name",
        SortField::PrimPath => "e.prim_path",
        SortField::Category => "e.category",
        SortField::Family => "e.family",
        SortField::TypeName => "e.type_name",
        SortField::TranslationX => "e.tx_mm",
    }
}

pub(super) fn limit(limit: u32) -> u32 {
    if limit == 0 { 100 } else { limit.min(1_000) }
}
