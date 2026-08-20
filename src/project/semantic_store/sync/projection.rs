use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use usd_model::{
    EntityKey, GeometrySignature, HashDigest, IdentitySource, SemanticInfo, SemanticProperty,
    SemanticSnapshot, SnapshotId, SnapshotSource, TransformSignature,
};
use viewport_protocol::AuthorizationPolicy;

use super::super::{SemanticStore, TursoSemanticStore};

/// A semantic snapshot view authorized for one self-render client.
///
/// The source hashes remain available for provenance, while `projection_hash`
/// identifies the filtered view and must be used for client cache identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct AuthorizedSemanticSnapshot {
    pub source_snapshot_id: SnapshotId,
    pub source: SnapshotSource,
    pub config_hash: HashDigest,
    pub projection_hash: HashDigest,
    pub entities: BTreeMap<EntityKey, AuthorizedEntitySnapshot>,
}

/// One entity in an authorization-safe semantic snapshot view.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct AuthorizedEntitySnapshot {
    pub key: EntityKey,
    pub prim_path: String,
    pub identity_source: IdentitySource,
    pub semantic: SemanticInfo,
    pub transform: TransformSignature,
    pub geometry: Option<GeometrySignature>,
    pub properties: Vec<SemanticProperty>,
    pub source_metadata_hash: HashDigest,
    pub source_full_hash: HashDigest,
}

impl TursoSemanticStore {
    /// Reads one durable snapshot and returns only the data authorized for a
    /// self-render client. This is deliberately separate from remote Turso
    /// replication so policy filtering happens before any bytes leave the
    /// server database.
    #[allow(dead_code)]
    pub(crate) async fn get_authorized_snapshot(
        &self,
        snapshot_id: &SnapshotId,
        policy: &AuthorizationPolicy,
    ) -> Result<Option<AuthorizedSemanticSnapshot>> {
        let Some(snapshot) = self.get_snapshot(snapshot_id).await? else {
            return Ok(None);
        };
        authorize_snapshot(&snapshot, policy).map(Some)
    }
}

/// Projects a complete server snapshot into the authorized client view.
pub(crate) fn authorize_snapshot(
    snapshot: &SemanticSnapshot,
    policy: &AuthorizationPolicy,
) -> Result<AuthorizedSemanticSnapshot> {
    policy
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid client-sync authorization policy: {error}"))?;
    if !policy.allows_self_render_delivery() {
        bail!("client semantic sync requires self-render delivery authorization");
    }
    if !policy.allows_model_download() {
        bail!("client semantic sync requires model-download authorization");
    }
    if matches!(&snapshot.source, SnapshotSource::GitCommit { .. }) && !policy.allows_history() {
        bail!("client semantic sync of committed snapshots requires history authorization");
    }

    let entities = snapshot
        .entities
        .values()
        .map(|entity| {
            let mut geometry = entity.geometry.clone();
            if let Some(geometry_signature) = geometry.as_mut()
                && geometry_signature
                    .render_blob
                    .as_ref()
                    .is_some_and(|blob_id| !policy.allows_runtime_blob(&blob_id.0))
            {
                geometry_signature.render_blob = None;
            }

            let properties = entity
                .properties
                .iter()
                .filter(|property| policy.allows_semantic_property(&property.name))
                .cloned()
                .collect();

            (
                entity.key.clone(),
                AuthorizedEntitySnapshot {
                    key: entity.key.clone(),
                    prim_path: entity.prim_path.clone(),
                    identity_source: entity.identity_source,
                    semantic: entity.semantic.clone(),
                    transform: entity.transform.clone(),
                    geometry,
                    properties,
                    source_metadata_hash: entity.metadata_hash,
                    source_full_hash: entity.full_hash,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut projected = AuthorizedSemanticSnapshot {
        source_snapshot_id: snapshot.snapshot_id.clone(),
        source: snapshot.source.clone(),
        config_hash: snapshot.config_hash,
        projection_hash: HashDigest::new([0; HashDigest::BYTE_LEN]),
        entities,
    };
    projected.projection_hash = projection_hash(&projected)?;
    Ok(projected)
}

pub(super) fn projection_hash(snapshot: &AuthorizedSemanticSnapshot) -> Result<HashDigest> {
    let mut canonical = snapshot.clone();
    canonical.projection_hash = HashDigest::new([0; HashDigest::BYTE_LEN]);
    let bytes = serde_json::to_vec(&canonical).context("serializing semantic client projection")?;
    Ok(HashDigest::new(*blake3::hash(&bytes).as_bytes()))
}

pub(super) fn verify_projection_hash(projection: &AuthorizedSemanticSnapshot) -> Result<()> {
    let expected = projection_hash(projection)?;
    if expected != projection.projection_hash {
        bail!(
            "client projection hash mismatch: expected {}, received {}",
            expected,
            projection.projection_hash
        );
    }
    Ok(())
}
