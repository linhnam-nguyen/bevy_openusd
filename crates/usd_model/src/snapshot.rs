//! Semantic snapshot containers.

use std::collections::HashMap;

use crate::hash::HashDigest;
use crate::identity::{EntityKey, IdentitySource};
use crate::semantic::SemanticInfo;
use crate::signature::{GeometrySignature, TransformSignature};
use crate::value::CanonicalValue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SemanticProperty {
    pub name: String,
    pub value: CanonicalValue,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SnapshotId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SnapshotSource {
    Working { session: String, live_revision: u64 },
    GitCommit { oid: String },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EntitySnapshot {
    pub key: EntityKey,
    pub prim_path: String,
    pub identity_source: IdentitySource,
    pub semantic: SemanticInfo,
    pub transform: TransformSignature,
    pub geometry: Option<GeometrySignature>,
    pub properties: Vec<SemanticProperty>,
    pub metadata_hash: HashDigest,
    pub full_hash: HashDigest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SemanticSnapshot {
    pub snapshot_id: SnapshotId,
    pub source: SnapshotSource,
    pub config_hash: HashDigest,
    pub entities: HashMap<EntityKey, EntitySnapshot>,
}
