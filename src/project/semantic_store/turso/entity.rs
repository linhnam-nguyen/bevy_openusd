use anyhow::{Context, Result, anyhow};
use usd_model::{
    CanonicalValue, EntityKey, EntitySnapshot, IdentitySource, SemanticSnapshot, SnapshotId,
};

use super::TursoSemanticStore;

impl TursoSemanticStore {
    pub(super) async fn get_entity_impl(
        &self,
        snapshot: &SnapshotId,
        key: &EntityKey,
    ) -> Result<Option<EntitySnapshot>> {
        Ok(self
            .get_snapshot_impl(snapshot)
            .await?
            .and_then(|snapshot| snapshot.entities.get(key).cloned()))
    }
}

pub(super) async fn insert_entity(
    transaction: &turso::transaction::Transaction<'_>,
    snapshot: &SemanticSnapshot,
    entity: &EntitySnapshot,
) -> Result<()> {
    let geometry = entity.geometry.as_ref();
    transaction
        .execute(
            "INSERT INTO entities
                (snapshot_id, entity_key, identity_source, prim_path, display_name,
                 category, family, type_name, type_id, transform_hash, topology_hash,
                 shape_hash, metadata_hash, full_hash, tx_mm, ty_mm, tz_mm)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            turso::params![
                snapshot.snapshot_id.0.clone(),
                entity.key.0.clone(),
                identity_source_name(entity.identity_source),
                entity.prim_path.clone(),
                optional_text(entity.semantic.display_name.as_deref()),
                optional_text(entity.semantic.category.as_deref()),
                optional_text(entity.semantic.family.as_deref()),
                optional_text(entity.semantic.type_name.as_deref()),
                optional_text(entity.semantic.type_id.as_deref()),
                entity.transform.hash.to_hex(),
                geometry.map(|value| value.topology_hash.to_hex()),
                geometry.map(|value| value.shape_hash.to_hex()),
                entity.metadata_hash.to_hex(),
                entity.full_hash.to_hex(),
                turso::Value::Integer(entity.transform.translation_mm[0]),
                turso::Value::Integer(entity.transform.translation_mm[1]),
                turso::Value::Integer(entity.transform.translation_mm[2]),
            ],
        )
        .await
        .with_context(|| format!("inserting durable semantic entity {}", entity.key.0))?;
    Ok(())
}

pub(super) async fn insert_property(
    transaction: &turso::transaction::Transaction<'_>,
    snapshot_id: &str,
    entity_key: &str,
    name: &str,
    value: &CanonicalValue,
) -> Result<()> {
    let (value_kind, value_text, value_integer, value_real, value_hash) = property_columns(value)?;
    transaction
        .execute(
            "INSERT INTO properties
                (snapshot_id, entity_key, name, value_kind, value_text,
                 value_integer, value_real, value_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            turso::params![
                snapshot_id.to_owned(),
                entity_key.to_owned(),
                name.to_owned(),
                value_kind,
                value_text,
                value_integer,
                value_real,
                value_hash,
            ],
        )
        .await
        .with_context(|| format!("inserting durable semantic property {entity_key}.{name}"))?;
    Ok(())
}

pub(super) fn identity_source_name(source: IdentitySource) -> &'static str {
    match source {
        IdentitySource::RevitUniqueId => "revit_unique_id",
        IdentitySource::IfcGuid => "ifc_guid",
        IdentitySource::ApplicationGuid => "application_guid",
        IdentitySource::AssetIdentifier => "asset_identifier",
        IdentitySource::PrimPath => "prim_path",
        IdentitySource::Synthetic => "synthetic",
    }
}

pub(super) fn optional_text(value: Option<&str>) -> turso::Value {
    value
        .map(|value| turso::Value::Text(value.to_owned()))
        .unwrap_or(turso::Value::Null)
}

pub(super) fn nullable_text(row: &turso::Row, index: usize) -> Result<Option<String>> {
    match row.get_value(index)? {
        turso::Value::Null => Ok(None),
        turso::Value::Text(value) => Ok(Some(value)),
        other => Err(anyhow!(
            "expected nullable text at column {index}, got {other:?}"
        )),
    }
}

pub(super) fn nullable_integer(row: &turso::Row, index: usize) -> Result<Option<i64>> {
    match row.get_value(index)? {
        turso::Value::Null => Ok(None),
        turso::Value::Integer(value) => Ok(Some(value)),
        other => Err(anyhow!(
            "expected nullable integer at column {index}, got {other:?}"
        )),
    }
}

pub(super) fn property_columns(
    value: &CanonicalValue,
) -> Result<(
    &'static str,
    Option<String>,
    Option<i64>,
    Option<f64>,
    String,
)> {
    let (kind, text, integer, real) = match value {
        CanonicalValue::Null => ("null", None, None, None),
        CanonicalValue::Bool(value) => ("bool", Some(value.to_string()), None, None),
        CanonicalValue::Integer(value) => ("integer", None, Some(*value), None),
        CanonicalValue::Real(value) => ("real", None, None, Some(*value)),
        CanonicalValue::Text(value) => ("text", Some(value.clone()), None, None),
        CanonicalValue::TextArray(values) => (
            "text_array",
            Some(serde_json::to_string(values)?),
            None,
            None,
        ),
        CanonicalValue::NumberArray(values) => (
            "number_array",
            Some(serde_json::to_string(values)?),
            None,
            None,
        ),
        CanonicalValue::Json(value) => ("json", Some(value.clone()), None, None),
    };
    let hash = blake3::hash(format!("{kind}:{text:?}:{integer:?}:{real:?}").as_bytes()).to_hex();
    Ok((kind, text, integer, real, hash.to_string()))
}
