//! Strong, deterministic cache revision identity.
//!
//! Combines canonical source closure bytes, composition context (occurrence
//! placement overrides and authored variant selections), projection recipe,
//! and renderer/cache schema versions. Never relies on Git SHA alone or
//! machine-specific absolute paths.

use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use openusd::usd::{PrimPredicate, Stage};
use serde::{Deserialize, Serialize};
use usd_model::HashDigest;
use usd_project::{SceneId, SceneMemberTarget};
use viewport_protocol::RuntimeProfile;

use super::contract::{
    PROJECT_CACHE_INDEX_SCHEMA_VERSION, ProjectCacheTarget, SCENE_CACHE_DESCRIPTOR_SCHEMA_VERSION,
    SCENE_CACHE_INDEX_SCHEMA_VERSION, SCENE_SPATIAL_INDEX_SCHEMA_VERSION,
};
use crate::project::assets::closure::target_content_hash;

const CACHE_REVISION_IDENTITY_DOMAIN: &[u8] = b"nemeton.cache-revision-identity.v1\0";
const COMPOSITION_CONTEXT_DOMAIN: &[u8] = b"nemeton.composition-context.v1\0";
const SCHEMA_DIGEST_DOMAIN: &[u8] = b"nemeton.cache-schema-digest.v1\0";

/// Strong cache revision identity binding canonical source closure,
/// composition context/occurrence overrides, projection recipe, and schema.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CacheRevisionIdentity {
    pub(crate) target: ProjectCacheTarget,
    pub(crate) source_closure_digest: HashDigest,
    pub(crate) composition_context_digest: HashDigest,
    pub(crate) projection_recipe_digest: HashDigest,
    pub(crate) schema_digest: HashDigest,
}

impl CacheRevisionIdentity {
    /// Deterministic BLAKE3 digest of all identity components.
    pub(crate) fn digest(&self) -> HashDigest {
        let mut hasher = blake3::Hasher::new();
        hasher.update(CACHE_REVISION_IDENTITY_DOMAIN);
        let target_key = self.target.key();
        hasher.update(&(target_key.len() as u64).to_le_bytes());
        hasher.update(target_key.as_bytes());
        hasher.update(self.source_closure_digest.as_bytes());
        hasher.update(self.composition_context_digest.as_bytes());
        hasher.update(self.projection_recipe_digest.as_bytes());
        hasher.update(self.schema_digest.as_bytes());
        HashDigest::new(*hasher.finalize().as_bytes())
    }

    pub(crate) fn storage_key(&self) -> String {
        self.digest().to_hex()
    }
}

/// Deterministic digest of all persistent Scene cache schema versions.
pub(crate) fn current_cache_schema_digest() -> HashDigest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SCHEMA_DIGEST_DOMAIN);
    hasher.update(b"sealed-source-provenance-v2\0");
    hasher.update(&SCENE_CACHE_DESCRIPTOR_SCHEMA_VERSION.to_le_bytes());
    hasher.update(&SCENE_CACHE_INDEX_SCHEMA_VERSION.to_le_bytes());
    hasher.update(&SCENE_SPATIAL_INDEX_SCHEMA_VERSION.to_le_bytes());
    hasher.update(&PROJECT_CACHE_INDEX_SCHEMA_VERSION.to_le_bytes());
    hasher.update(&super::compatibility::PROJECT_RUNTIME_CACHE_COMPATIBILITY_VERSION.to_le_bytes());
    hasher.update(&super::compatibility::RUNTIME_PROJECTION_VERSION.to_le_bytes());
    HashDigest::new(*hasher.finalize().as_bytes())
}

/// Combine runtime profile and configuration hash into a deterministic
/// projection recipe digest.
pub(crate) fn projection_recipe_digest(
    profile: RuntimeProfile,
    config_hash: HashDigest,
) -> HashDigest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"nemeton.projection-recipe.v1\0");
    let profile_bytes = serde_json::to_vec(&profile).expect("RuntimeProfile is serializable");
    hasher.update(&(profile_bytes.len() as u64).to_le_bytes());
    hasher.update(&profile_bytes);
    hasher.update(config_hash.as_bytes());
    HashDigest::new(*hasher.finalize().as_bytes())
}

/// Compute a strong, deterministic [`CacheRevisionIdentity`] for a managed
/// Project Scene.
///
/// Fails closed if the canonical Scene layer, its imported closure, or its
/// authored composition context cannot be read.
pub(crate) fn compute_scene_revision_identity(
    project_root: &Path,
    scene_id: SceneId,
    profile: RuntimeProfile,
    config_hash: HashDigest,
) -> Result<CacheRevisionIdentity> {
    let scene = crate::project::scene::authoring::scene_path(project_root, scene_id);
    let live = usd_bevy::LiveStage::open_document(&scene)
        .context("capture composed Scene dependency closure")?;
    scene_revision_identity_from_snapshot(project_root, scene_id, profile, config_hash, &live)
}

/// Derive the full revision from the sealed source actually consumed by the builder.
pub(crate) fn scene_revision_identity_from_snapshot(
    project_root: &Path,
    scene_id: SceneId,
    profile: RuntimeProfile,
    config_hash: HashDigest,
    live: &usd_bevy::LiveStage,
) -> Result<CacheRevisionIdentity> {
    let target = ProjectCacheTarget::Scene { id: scene_id.to_string() };
    let source_closure_digest = target_content_hash(project_root, &target)?;
    let scene = crate::project::scene::authoring::scene_path(project_root, scene_id);
    let complete = super::generic::source_identity_from_opened_stage(live)?;
    anyhow::ensure!(complete.entry_path == std::fs::canonicalize(scene)?, "sealed Scene source is not the selected canonical entry");
    let authored_context = compute_scene_composition_context(&live.stage, scene_id)?;
    let mut context = blake3::Hasher::new();
    context.update(b"nemeton.qualified-composition.v2\0");
    context.update(authored_context.as_bytes());
    context.update(complete.closure_digest.as_bytes());
    context.update(complete.resolver_digest.as_bytes());
    let composition_context_digest = HashDigest::new(*context.finalize().as_bytes());
    Ok(CacheRevisionIdentity {
        target,
        source_closure_digest,
        composition_context_digest,
        projection_recipe_digest: projection_recipe_digest(profile, config_hash),
        schema_digest: current_cache_schema_digest(),
    })
}

/// Hash the deterministic composition context for one managed Scene:
/// - ordered occurrence placements & transforms (`SceneMember`),
/// - authored variant selections on owned prims,
/// - stage metrics (`metersPerUnit`, `upAxis`, `startTimeCode`, `endTimeCode`,
///   `timeCodesPerSecond`).
fn compute_scene_composition_context(stage: &Stage, scene_id: SceneId) -> Result<HashDigest> {
    let mut members = crate::project::scene::authoring::read_scene_members_from_stage(stage, scene_id)
        .with_context(|| format!("read sealed Scene members for composition identity {scene_id}"))?;
    members.sort_by_key(|member| member.id);
    let mut hasher = blake3::Hasher::new();
    hasher.update(COMPOSITION_CONTEXT_DOMAIN);
    hasher.update(&(members.len() as u64).to_le_bytes());
    for member in &members {
        let member_id = member.id.to_string();
        hasher.update(&(member_id.len() as u64).to_le_bytes());
        hasher.update(member_id.as_bytes());
        let (target_kind, target_id) = match member.target {
            SceneMemberTarget::Scene(id) => (b"scene".as_slice(), id.to_string()),
            SceneMemberTarget::Model(id) => (b"model".as_slice(), id.to_string()),
        };
        hasher.update(target_kind);
        hasher.update(&(target_id.len() as u64).to_le_bytes());
        hasher.update(target_id.as_bytes());
        let transform_bytes = serde_json::to_vec(&member.transform)
            .context("serialize Scene member placement transform")?;
        hasher.update(&(transform_bytes.len() as u64).to_le_bytes());
        hasher.update(&transform_bytes);
    }

    hash_stage_composition_context(stage, &mut hasher)?;
    Ok(HashDigest::new(*hasher.finalize().as_bytes()))
}

/// Compute a deterministic, path-independent composition context digest for
/// any OpenUSD [`Stage`] (used by standalone USD caching as well as managed
/// Scenes).
pub(crate) fn compute_stage_composition_context(stage: &Stage) -> Result<HashDigest> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(COMPOSITION_CONTEXT_DOMAIN);
    hash_stage_composition_context(stage, &mut hasher)?;
    Ok(HashDigest::new(*hasher.finalize().as_bytes()))
}

fn hash_stage_composition_context(stage: &Stage, hasher: &mut blake3::Hasher) -> Result<()> {
    let mut variants = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut prim_paths = Vec::new();
    stage.traverse(PrimPredicate::DEFAULT, |path| {
        if !path.is_abs_root() {
            prim_paths.push(path.clone());
        }
    })?;
    prim_paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    for path in prim_paths {
        let prim = stage.prim(path.clone())?;
        let mut set_names = prim.variant_sets().names()?;
        set_names.sort();
        for name in set_names {
            if let Some(selected) = prim.variant_set(&name).selection()? {
                variants
                    .entry(path.as_str().to_owned())
                    .or_default()
                    .insert(name, selected);
            }
        }
    }

    hasher.update(&(variants.len() as u64).to_le_bytes());
    for (prim_path, sets) in variants {
        hasher.update(&(prim_path.len() as u64).to_le_bytes());
        hasher.update(prim_path.as_bytes());
        hasher.update(&(sets.len() as u64).to_le_bytes());
        for (set_name, selection) in sets {
            hasher.update(&(set_name.len() as u64).to_le_bytes());
            hasher.update(set_name.as_bytes());
            hasher.update(&(selection.len() as u64).to_le_bytes());
            hasher.update(selection.as_bytes());
        }
    }

    hasher.update(&stage.time_codes_per_second().to_bits().to_le_bytes());
    hasher.update(&stage.frames_per_second().to_bits().to_le_bytes());
    let has_range = stage.has_authored_time_code_range();
    hasher.update(&[u8::from(has_range)]);
    if has_range {
        hasher.update(&stage.start_time_code().to_bits().to_le_bytes());
        hasher.update(&stage.end_time_code().to_bits().to_le_bytes());
    }
    Ok(())
}
