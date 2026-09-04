//! Compatibility identity for renderer-neutral Project runtime caches.
//!
//! A cache descriptor is reusable only when the semantic configuration and all
//! runtime projection payload contracts agree. This keeps a change in material
//! provenance or cache policy from hydrating an older descriptor that happens
//! to have the same USD semantic configuration hash.

use usd_model::HashDigest;

/// Bumped whenever the meaning of a ready runtime cache changes, even if the
/// individual payload schemas remain decodable.
pub(crate) const PROJECT_RUNTIME_CACHE_COMPATIBILITY_VERSION: u16 = 2;

/// Explicit projection contract version included in Project cache identity.
pub(crate) const RUNTIME_PROJECTION_VERSION: u16 = 1;

/// Build the cache configuration identity from every renderer-neutral runtime
/// contract that can affect hydrated material, mesh, hierarchy, or texture
/// output.
pub(crate) fn project_runtime_cache_config_hash(semantic_config_hash: HashDigest) -> HashDigest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"usdhub-project-runtime-cache-config");
    hasher.update(&PROJECT_RUNTIME_CACHE_COMPATIBILITY_VERSION.to_le_bytes());
    hasher.update(&RUNTIME_PROJECTION_VERSION.to_le_bytes());
    hasher.update(&crate::project::runtime_delivery::RUNTIME_HIERARCHY_VERSION.to_le_bytes());
    hasher.update(&crate::project::runtime_delivery::RUNTIME_MESH_VERSION.to_le_bytes());
    hasher.update(&crate::project::runtime_payload::RUNTIME_MATERIAL_VERSION.to_le_bytes());
    hasher.update(&crate::project::runtime_payload::RUNTIME_TEXTURE_VERSION.to_le_bytes());
    hasher.update(semantic_config_bytes(semantic_config_hash).as_slice());
    HashDigest::new(*hasher.finalize().as_bytes())
}

fn semantic_config_bytes(hash: HashDigest) -> Vec<u8> {
    serde_json::to_vec(&hash).expect("semantic configuration hash is serializable")
}

#[cfg(test)]
mod tests {
    use usd_model::HashDigest;

    use super::*;

    #[test]
    fn runtime_contract_changes_identity() {
        let semantic = HashDigest::new([7; HashDigest::BYTE_LEN]);
        assert_ne!(project_runtime_cache_config_hash(semantic), semantic);
        assert_eq!(PROJECT_RUNTIME_CACHE_COMPATIBILITY_VERSION, 2);
    }
}
