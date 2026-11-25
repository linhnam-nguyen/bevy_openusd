use anyhow::{Context, Result, anyhow};
use std::collections::BTreeSet;

use super::SemanticDatabase;
use super::row::{nullable_integer, nullable_text};
use crate::semantic::query::{
    DistinctFieldKeys, DistinctFieldKeysResult, SemanticGroup, SemanticKey, SemanticQuery,
    SemanticQueryResult, SemanticQueryRow,
};
use crate::semantic::query::sql::{append_order_by, build_where, group_column, limit};

impl SemanticDatabase {
    pub(crate) async fn distinct_field_keys(
        &self,
        query: &DistinctFieldKeys,
    ) -> Result<DistinctFieldKeysResult> {
        let Some(snapshot_id) = self.latest_snapshot_id().await? else {
            return Ok(DistinctFieldKeysResult::default());
        };
        let candidates = match &query.selection {
            Some(selection) => selection.clone(),
            None => {
                let mut candidates = vec![
                    SemanticKey::Category,
                    SemanticKey::Family,
                    SemanticKey::TypeName,
                ];
                let mut rows = self
                    .connection
                    .query(
                        "SELECT DISTINCT name FROM properties
                         WHERE snapshot_id = ? AND value_kind <> 'null' ORDER BY name ASC",
                        turso::params![snapshot_id.clone()],
                    )
                    .await
                    .context("listing semantic property fields")?;
                while let Some(row) = rows.next().await.context("reading semantic field")? {
                    candidates.push(SemanticKey::Property(row.get(0)?));
                }
                candidates
            }
        };

        let mut keys = BTreeSet::new();
        for key in candidates {
            let present = match &key {
                SemanticKey::Category => self.field_present(&snapshot_id, "category").await?,
                SemanticKey::Family => self.field_present(&snapshot_id, "family").await?,
                SemanticKey::TypeName => self.field_present(&snapshot_id, "type_name").await?,
                SemanticKey::Property(name) => {
                    let row = self
                        .connection
                        .query(
                            "SELECT 1 FROM properties
                             WHERE snapshot_id = ? AND name = ? AND value_kind <> 'null'
                             LIMIT 1",
                            turso::params![snapshot_id.clone(), name.clone()],
                        )
                        .await
                        .with_context(|| format!("checking semantic property field {name}"))?
                        .next()
                        .await
                        .context("reading semantic property field")?;
                    row.is_some()
                }
            };
            if present {
                keys.insert(key);
            }
        }
        Ok(DistinctFieldKeysResult {
            keys: keys.into_iter().collect(),
        })
    }

    async fn field_present(&self, snapshot_id: &str, column: &str) -> Result<bool> {
        let sql = format!(
            "SELECT 1 FROM entities WHERE snapshot_id = ?
             AND NULLIF(TRIM(COALESCE({column}, '')), '') IS NOT NULL LIMIT 1"
        );
        Ok(self
            .connection
            .query(&sql, turso::params![snapshot_id.to_owned()])
            .await
            .with_context(|| format!("checking semantic field {column}"))?
            .next()
            .await
            .context("reading semantic field presence")?
            .is_some())
    }

    pub(crate) async fn query(&self, query: &SemanticQuery) -> Result<SemanticQueryResult> {
        let Some(snapshot_id) = self.latest_snapshot_id().await? else {
            return Ok(SemanticQueryResult::default());
        };

        let (where_sql, where_params) = build_where(query, &snapshot_id);
        let count_sql = format!("SELECT COUNT(*) FROM entities e {where_sql}");
        let count_row = self
            .connection
            .query(&count_sql, turso::params_from_iter(where_params.clone()))
            .await
            .context("counting semantic query results")?
            .next()
            .await
            .context("reading semantic query count")?
            .ok_or_else(|| anyhow!("semantic count query returned no row"))?;
        let total = count_row
            .get::<i64>(0)
            .context("decoding semantic query count")? as u32;

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
            .context("querying semantic entities")?;
        let mut result_rows = Vec::new();
        while let Some(row) = rows.next().await.context("reading semantic entity row")? {
            result_rows.push(SemanticQueryRow {
                entity_key: usd_model::EntityKey::from(row.get::<String>(0)?),
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

        let (_, group_params) = build_where(query, &snapshot_id);
        let groups = self.groups(query, &where_sql, &group_params).await?;
        Ok(SemanticQueryResult {
            total,
            has_more: query.offset.saturating_add(result_rows.len() as u32) < total,
            rows: result_rows,
            groups,
        })
    }

    async fn groups(
        &self,
        query: &SemanticQuery,
        where_sql: &str,
        where_params: &[turso::Value],
    ) -> Result<Vec<SemanticGroup>> {
        let mut groups = Vec::new();
        for field in &query.group_by {
            let column = group_column(*field);
            let sql = format!(
                "SELECT e.{column}, COUNT(*) FROM entities e {where_sql}
                 GROUP BY e.{column} ORDER BY COUNT(*) DESC, e.{column} ASC"
            );
            let mut rows = self
                .connection
                .query(&sql, turso::params_from_iter(where_params.to_vec()))
                .await
                .with_context(|| format!("grouping semantic entities by {field:?}"))?;
            while let Some(row) = rows.next().await.context("reading semantic group row")? {
                groups.push(SemanticGroup {
                    field: *field,
                    value: nullable_text(&row, 0)?,
                    count: row.get::<i64>(1)? as u32,
                });
            }
        }
        Ok(groups)
    }
}
