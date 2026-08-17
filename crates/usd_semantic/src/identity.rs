//! Configurable source-identity resolution.

use anyhow::{Result, bail};
use openusd::sdf::{Path, Value};
use openusd::usd::Stage;
use usd_model::{EntityKey, IdentitySource};

use crate::config::IdentityConfig;

/// Resolve an entity key using the configured source-identity priority.
///
/// Candidate names are read as authored/composed prim attributes. The value
/// is kept opaque: in particular, Revit UniqueIds are not parsed or converted
/// into UUIDs. Source prefixes keep equal text values from different identity
/// systems from colliding in one semantic snapshot.
pub fn resolve_identity(
    stage: &Stage,
    path: &Path,
    config: &IdentityConfig,
) -> Result<(EntityKey, IdentitySource)> {
    let prim = stage.prim(path.clone());
    for (source, candidates) in [
        (
            IdentitySource::RevitUniqueId,
            &config.revit_unique_id_candidates,
        ),
        (IdentitySource::IfcGuid, &config.ifc_guid_candidates),
        (
            IdentitySource::ApplicationGuid,
            &config.application_guid_candidates,
        ),
        (
            IdentitySource::AssetIdentifier,
            &config.asset_identifier_candidates,
        ),
    ] {
        for candidate in candidates {
            let Some(value) = prim.attribute(candidate).get::<Value>()? else {
                continue;
            };
            let Some(value) = identity_text(value) else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            return Ok((prefixed_key(source, value), source));
        }
    }

    let prim_path = path.as_str();
    if config.allow_prim_path_fallback {
        return Ok((EntityKey::from(prim_path), IdentitySource::PrimPath));
    }
    if config.allow_synthetic_fallback {
        let digest = blake3::hash(prim_path.as_bytes()).to_hex();
        return Ok((
            EntityKey::from(format!("synthetic:{digest}")),
            IdentitySource::Synthetic,
        ));
    }
    bail!("no configured source identity for {prim_path} and both fallback modes are disabled")
}

fn identity_text(value: Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value),
        Value::Token(value) => Some(value.as_str().to_owned()),
        Value::AssetPath(value) => Some(value.as_str().to_owned()),
        _ => None,
    }
}

fn prefixed_key(source: IdentitySource, value: &str) -> EntityKey {
    let prefix = match source {
        IdentitySource::RevitUniqueId => "revit",
        IdentitySource::IfcGuid => "ifc",
        IdentitySource::ApplicationGuid => "application",
        IdentitySource::AssetIdentifier => "asset",
        IdentitySource::PrimPath | IdentitySource::Synthetic => unreachable!(),
    };
    EntityKey::from(format!("{prefix}:{value}"))
}
