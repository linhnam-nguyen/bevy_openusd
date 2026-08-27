//! Stage traversal and semantic snapshot assembly.

use std::collections::HashMap;

use anyhow::{Context, Result};
use openusd::sdf::Path;
use openusd::usd::{PrimPredicate, Stage};
use usd_model::{
    Bounds3, CanonicalValue, EntityKey, EntitySnapshot, GeometrySignature, HashDigest,
    SemanticInfo, SemanticProperty, SemanticSnapshot, SnapshotId, SnapshotSource,
    TransformSignature,
};

use crate::config::SemanticConfig;
use crate::geometry::extract_geometry;
use crate::identity::resolve_identity;
use crate::metadata::extract_metadata;
use crate::transform::extract_transform;

#[derive(Clone, Debug, Default)]
pub struct SemanticExtractor {
    config: SemanticConfig,
}

impl SemanticExtractor {
    pub fn new(config: SemanticConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &SemanticConfig {
        &self.config
    }

    pub fn extract(&self, stage: &Stage, source: SnapshotSource) -> Result<SemanticSnapshot> {
        extract_stage(stage, source, &self.config)
    }

    /// Extract one prim using the same configuration as a full snapshot.
    ///
    /// Live-stage synchronization uses this after a changed-info notice so a
    /// semantic update touches only the affected entity rows.
    pub fn extract_entity(&self, stage: &Stage, path: &Path) -> Result<EntitySnapshot> {
        extract_entity(stage, path, &self.config)
    }

    /// Rebuild the deterministic snapshot identity after replacing entities.
    ///
    /// The entity map is supplied by the caller so incremental consumers can
    /// preserve unaffected entities without re-extracting the whole stage.
    pub fn snapshot_from_entities(
        &self,
        source: SnapshotSource,
        entities: HashMap<EntityKey, EntitySnapshot>,
    ) -> SemanticSnapshot {
        let config_hash = self.config.hash();
        let snapshot_id = SnapshotId(snapshot_hash(&entities, config_hash).to_hex());
        SemanticSnapshot {
            snapshot_id,
            source,
            config_hash,
            entities,
        }
    }
}

/// Extract a deterministic semantic snapshot from the composed stage.
pub fn extract_stage(
    stage: &Stage,
    source: SnapshotSource,
    config: &SemanticConfig,
) -> Result<SemanticSnapshot> {
    let mut paths = Vec::new();
    stage
        .traverse(PrimPredicate::DEFAULT, |path| paths.push(path.clone()))
        .context("traversing OpenUSD stage for semantic extraction")?;

    let mut entities = HashMap::with_capacity(paths.len());
    for path in paths {
        let entity = extract_entity(stage, &path, config)
            .with_context(|| format!("extracting semantic entity {}", path.as_str()))?;
        entities.insert(entity.key.clone(), entity);
    }

    let config_hash = config.hash();
    let snapshot_id = SnapshotId(snapshot_hash(&entities, config_hash).to_hex());
    Ok(SemanticSnapshot {
        snapshot_id,
        source,
        config_hash,
        entities,
    })
}

fn extract_entity(stage: &Stage, path: &Path, config: &SemanticConfig) -> Result<EntitySnapshot> {
    let prim_path = path.as_str().to_owned();
    let (key, identity_source) = resolve_identity(stage, path, &config.identity)?;
    let (semantic, properties) = extract_metadata(stage, path, config)?;
    let transform = extract_transform(stage, path, config)?;
    let geometry = extract_geometry(stage, path, config)?;
    let metadata_hash = metadata_hash(&semantic, &properties);
    let full_hash = entity_hash(
        &key,
        &prim_path,
        identity_source,
        &semantic,
        &transform,
        geometry.as_ref(),
        &properties,
    );

    Ok(EntitySnapshot {
        key,
        prim_path,
        identity_source,
        semantic,
        transform,
        geometry,
        properties,
        metadata_hash,
        full_hash,
    })
}

fn metadata_hash(semantic: &SemanticInfo, properties: &[SemanticProperty]) -> HashDigest {
    let mut bytes = Vec::new();
    write_option_string(&mut bytes, semantic.category.as_deref());
    write_option_string(&mut bytes, semantic.family.as_deref());
    write_option_string(&mut bytes, semantic.type_name.as_deref());
    write_option_string(&mut bytes, semantic.type_id.as_deref());
    write_option_string(&mut bytes, semantic.display_name.as_deref());
    write_properties(&mut bytes, properties);
    digest(&bytes)
}

fn entity_hash(
    key: &EntityKey,
    prim_path: &str,
    identity_source: usd_model::IdentitySource,
    semantic: &SemanticInfo,
    transform: &TransformSignature,
    geometry: Option<&GeometrySignature>,
    properties: &[SemanticProperty],
) -> HashDigest {
    let mut bytes = Vec::new();
    write_string(&mut bytes, key.as_str());
    write_string(&mut bytes, prim_path);
    bytes.push(identity_source_tag(identity_source));
    write_option_string(&mut bytes, semantic.category.as_deref());
    write_option_string(&mut bytes, semantic.family.as_deref());
    write_option_string(&mut bytes, semantic.type_name.as_deref());
    write_option_string(&mut bytes, semantic.type_id.as_deref());
    write_option_string(&mut bytes, semantic.display_name.as_deref());
    write_transform(&mut bytes, transform);
    write_geometry(&mut bytes, geometry);
    write_properties(&mut bytes, properties);
    digest(&bytes)
}

fn snapshot_hash(
    entities: &HashMap<EntityKey, EntitySnapshot>,
    config_hash: HashDigest,
) -> HashDigest {
    let mut entries: Vec<_> = entities.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    let mut bytes = Vec::new();
    bytes.extend_from_slice(config_hash.as_bytes());
    for (key, entity) in entries {
        write_string(&mut bytes, key.as_str());
        bytes.extend_from_slice(entity.full_hash.as_bytes());
    }
    digest(&bytes)
}

fn write_transform(bytes: &mut Vec<u8>, transform: &TransformSignature) {
    for value in transform.translation_mm {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in transform.rotation_quantized {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in transform.scale_quantized {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(transform.hash.as_bytes());
}

fn write_geometry(bytes: &mut Vec<u8>, geometry: Option<&GeometrySignature>) {
    let Some(geometry) = geometry else {
        bytes.push(0);
        return;
    };
    bytes.push(1);
    bytes.extend_from_slice(&geometry.vertex_count.to_le_bytes());
    bytes.extend_from_slice(&geometry.index_count.to_le_bytes());
    write_bounds(bytes, geometry.local_bounds);
    for value in geometry.local_centroid.0 {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(geometry.topology_hash.as_bytes());
    bytes.extend_from_slice(geometry.shape_hash.as_bytes());
    match &geometry.render_blob {
        Some(blob) => {
            bytes.push(1);
            write_string(bytes, &blob.0);
        }
        None => bytes.push(0),
    }
}

fn write_bounds(bytes: &mut Vec<u8>, bounds: Bounds3) {
    for value in bounds.min.into_iter().chain(bounds.max) {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}

fn write_properties(bytes: &mut Vec<u8>, properties: &[SemanticProperty]) {
    bytes.extend_from_slice(&(properties.len() as u64).to_le_bytes());
    for property in properties {
        write_string(bytes, &property.name);
        write_value(bytes, &property.value);
        write_measurement(bytes, property.measurement.as_ref());
    }
}

fn write_measurement(bytes: &mut Vec<u8>, measurement: Option<&usd_model::MeasurementMetadata>) {
    let Some(measurement) = measurement else {
        bytes.push(0);
        return;
    };
    bytes.push(1);
    write_string(bytes, measurement.quantity.as_str());
    write_string(bytes, measurement.canonical_unit.as_str());
    write_option_string(
        bytes,
        measurement.source_unit.as_ref().map(|unit| unit.as_str()),
    );
}

fn write_value(bytes: &mut Vec<u8>, value: &CanonicalValue) {
    match value {
        CanonicalValue::Null => bytes.push(0),
        CanonicalValue::Bool(value) => {
            bytes.push(1);
            bytes.push(u8::from(*value));
        }
        CanonicalValue::Integer(value) => {
            bytes.push(2);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        CanonicalValue::Real(value) => {
            bytes.push(3);
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        CanonicalValue::Text(value) => {
            bytes.push(4);
            write_string(bytes, value);
        }
        CanonicalValue::TextArray(values) => {
            bytes.push(5);
            bytes.extend_from_slice(&(values.len() as u64).to_le_bytes());
            for value in values {
                write_string(bytes, value);
            }
        }
        CanonicalValue::NumberArray(values) => {
            bytes.push(6);
            bytes.extend_from_slice(&(values.len() as u64).to_le_bytes());
            for value in values {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        CanonicalValue::Json(value) => {
            bytes.push(7);
            write_string(bytes, value);
        }
    }
}

fn write_option_string(bytes: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            bytes.push(1);
            write_string(bytes, value);
        }
        None => bytes.push(0),
    }
}

fn write_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn identity_source_tag(source: usd_model::IdentitySource) -> u8 {
    match source {
        usd_model::IdentitySource::RevitUniqueId => 0,
        usd_model::IdentitySource::IfcGuid => 1,
        usd_model::IdentitySource::ApplicationGuid => 2,
        usd_model::IdentitySource::AssetIdentifier => 3,
        usd_model::IdentitySource::PrimPath => 4,
        usd_model::IdentitySource::Synthetic => 5,
    }
}

fn digest(bytes: &[u8]) -> HashDigest {
    HashDigest::new(*blake3::hash(bytes).as_bytes())
}

#[cfg(test)]
#[path = "extractor_tests.rs"]
mod tests;
