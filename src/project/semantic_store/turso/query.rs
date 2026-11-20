use anyhow::{Context, Result, anyhow};
use usd_model::{EntityKey, SnapshotId};

use super::TursoSemanticStore;
use super::entity::{nullable_integer, nullable_text};
use crate::semantic_query::{SemanticGroup, SemanticQuery, SemanticQueryResult, SemanticQueryRow};
use crate::semantic_query::sql::{append_order_by, build_where, group_column, limit};

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
                    e.family, e.type_name, e.bim_enabled, e.tx_mm, e.ty_mm, e.tz_mm
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
                bim_enabled: row.get::<i64>(6)? != 0,
                translation_mm: [
                    nullable_integer(&row, 7)?.unwrap_or_default(),
                    nullable_integer(&row, 8)?.unwrap_or_default(),
                    nullable_integer(&row, 9)?.unwrap_or_default(),
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
