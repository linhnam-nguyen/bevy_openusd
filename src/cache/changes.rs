//! Conservative USD change classification for incremental Scene cache updates.
//!
//! Routes high-value non-geometry and localized edits (BIM/semantic metadata,
//! transform-only, localized geometry, material/texture and animation) to
//! incremental cache updates. Structural resyncs and uncertain source intervals
//! use a clean full build: a resulting subtree cannot prove old dependency safety.

use std::{collections::{BTreeSet, HashSet}, path::Path};

use anyhow::{Context, Result, ensure};
use usd_model::HashDigest;
use usd_project::SceneId;
use viewport_protocol::RuntimeProfile;

use super::contract::{SceneCacheEntryKind, SceneCacheIndex};
use super::identity::{CacheRevisionIdentity, compute_stage_composition_context, scene_revision_identity_from_snapshot};

const MAX_HINT_CHANGES: usize = 4096;

/// Persisted change hint passed from a canonical save to the Scene cache builder.
///
/// Always optional and advisory: if the base revision is missing, incompatible,
/// or corrupt, or if the change chain is truncated/uncertain, the builder falls
/// back to a clean full build producing identical canonical output.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SceneDeltaHint {
    pub(crate) base_revision_identity: Option<HashDigest>,
    pub(crate) result_revision_identity: Option<HashDigest>,
    pub(crate) changes: Vec<usd_bevy::StageChange>,
    pub(crate) truncated: bool,
}

impl SceneDeltaHint {
    pub(crate) fn from_persisted(
        base_revision_identity: Option<HashDigest>,
        result_revision_identity: Option<HashDigest>,
        snapshot: usd_bevy::PersistedChangeSnapshot,
    ) -> Self {
        Self {
            base_revision_identity,
            result_revision_identity,
            changes: snapshot.changes,
            truncated: snapshot.truncated,
        }
    }

    /// Combine hints of two existing queued jobs. Unknown history cannot be
    /// silently discarded, and the earlier result must equal the later base.
    pub(crate) fn coalesce(earlier: Option<Self>, later: Option<Self>) -> Option<Self> {
        let mut earlier = earlier?;
        let later = later?;
        if !earlier.has_complete_interval()
            || !later.has_complete_interval()
            || earlier.result_revision_identity != later.base_revision_identity
            || earlier.changes.len().saturating_add(later.changes.len()) > MAX_HINT_CHANGES
        {
            return None;
        }
        earlier.changes.extend(later.changes);
        earlier.result_revision_identity = later.result_revision_identity;
        Some(earlier)
    }

    fn has_complete_interval(&self) -> bool {
        !self.truncated
            && !self.changes.is_empty()
            && self.changes.len() <= MAX_HINT_CHANGES
            && self.base_revision_identity.is_some()
            && self.result_revision_identity.is_some()
    }

    pub(crate) fn targets_revision(&self, target: &CacheRevisionIdentity) -> bool {
        self.has_complete_interval() && self.result_revision_identity == Some(target.digest())
    }

    /// Exact interval endpoints and compatible interpretation of retained bytes.
    pub(crate) fn matches_revisions(
        &self,
        base: &CacheRevisionIdentity,
        target: &CacheRevisionIdentity,
    ) -> bool {
        self.targets_revision(target)
            && self.base_revision_identity == Some(base.digest())
            && base.target == target.target
            && base.projection_recipe_digest == target.projection_recipe_digest
            && base.schema_digest == target.schema_digest
    }
}

/// The verified canonical pre-save endpoint, never a mutable cache pointer.
pub(crate) struct SceneDeltaBase {
    revision: CacheRevisionIdentity,
    stage_context: HashDigest,
}

fn managed_scene_revision(
    project_root: &Path,
    scene_id: SceneId,
    snapshot: &usd_bevy::LiveStage,
) -> Result<CacheRevisionIdentity> {
    let config_hash = super::hydration::default_project_cache_config_hash();
    scene_revision_identity_from_snapshot(
        project_root, scene_id, RuntimeProfile::NativeMedium, config_hash, snapshot,
    )
}

/// Qualify all non-root inputs against the original sealed live source. Root
/// bytes are qualified separately against the last successful persisted save.
/// A changed/missing dependency or resolver association must use full rebuild.
fn matches_opened_dependencies(
    live: &usd_bevy::LiveStage,
    snapshot: &usd_bevy::LiveStage,
) -> Result<bool> {
    let entry = snapshot.opened_source_path().context("missing saved source entry")?;
    if live.opened_source_path().as_ref() != Some(&entry) {
        return Ok(false);
    }
    let mut original = live.opened_source_files().context("missing live source closure")?;
    let mut current = snapshot.opened_source_files().context("missing saved source closure")?;
    if original.remove(&entry).is_none() || current.remove(&entry).is_none() {
        return Ok(false);
    }
    if original.len() != current.len() {
        return Ok(false);
    }
    for (path, bytes) in &original {
        let Some(other) = current.get(path) else { return Ok(false); };
        let left: &[u8] = bytes.as_ref();
        let right: &[u8] = other.as_ref();
        if left != right {
            return Ok(false);
        }
    }
    let original = live.opened_resolver_bindings()?.context("missing live resolver bindings")?;
    let current = snapshot.opened_resolver_bindings()?.context("missing saved resolver bindings")?;
    Ok(original == current)
}

/// Capture the actual source revision before canonical save, while authoring
/// is leased. This remains advisory: cache qualification may never reject save.
pub(crate) fn capture_scene_delta_base(
    project_root: &Path,
    scene_id: SceneId,
    live: &usd_bevy::LiveStage,
) -> Option<SceneDeltaBase> {
    let result = (|| -> Result<SceneDeltaBase> {
        let path = crate::project::scene::authoring::scene_path(project_root, scene_id);
        let snapshot = usd_bevy::LiveStage::open_document(&path)?;
        let baseline = live.persisted_source_root_bytes().context("no persisted notice baseline")?;
        let current = snapshot.opened_root_bytes().context("missing canonical root bytes")?;
        let baseline: &[u8] = baseline.as_ref();
        let current: &[u8] = current.as_ref();
        ensure!(baseline == current && matches_opened_dependencies(live, &snapshot)?,
            "canonical source changed outside the persisted notice interval");
        Ok(SceneDeltaBase {
            revision: managed_scene_revision(project_root, scene_id, &snapshot)?,
            stage_context: compute_stage_composition_context(&snapshot.stage)?,
        })
    })();
    match result {
        Ok(base) => Some(base),
        Err(error) => {
            log::debug!("Scene delta base unavailable; using full cache rebuild: {error:#}");
            None
        }
    }
}

/// Called only after canonical save succeeds, before releasing the authoring
/// lease. Include generated timeline notices emitted during save. The new
/// snapshot must match the saved live root and the original non-root closure.
/// A changed stage context uses full rebuild (including generated time-range
/// changes which may not have a precise sparse notice).
pub(crate) fn complete_scene_delta_save(
    project_root: &Path,
    scene_id: SceneId,
    live: &usd_bevy::LiveStage,
    base: Option<SceneDeltaBase>,
) -> Option<SceneDeltaHint> {
    let changes = live.snapshot_persisted_changes();
    let result = (|| -> Result<(Option<SceneDeltaHint>, openusd::ar::SharedBuffer)> {
        let base = base.context("no qualified pre-save endpoint")?;
        ensure!(live.persisted_source_root_bytes().is_some(), "notice baseline invalidated during save");
        let path = crate::project::scene::authoring::scene_path(project_root, scene_id);
        let snapshot = usd_bevy::LiveStage::open_document(&path)?;
        ensure!(matches_opened_dependencies(live, &snapshot)?, "saved dependency closure changed");
        ensure!(usd_bevy::authoring::export_stage_string(&live.stage)?
            == usd_bevy::authoring::export_stage_string(&snapshot.stage)?,
            "saved canonical root differs from live authored root");
        let result_context = compute_stage_composition_context(&snapshot.stage)?;
        ensure!(result_context == compute_stage_composition_context(&live.stage)?,
            "saved canonical stage context differs from live stage");
        let target = managed_scene_revision(project_root, scene_id, &snapshot)?;
        let root = snapshot.opened_root_bytes().context("missing saved root bytes")?;
        let hint = SceneDeltaHint::from_persisted(
            Some(base.revision.digest()), Some(target.digest()), changes,
        );
        Ok(((base.stage_context == result_context && hint.has_complete_interval()).then_some(hint), root))
    })();
    match result {
        Ok((hint, root)) => {
            live.complete_persisted_save(Some(root));
            hint
        }
        Err(error) => {
            log::debug!("Scene delta result unavailable; using full cache rebuild: {error:#}");
            live.complete_persisted_save(None);
            None
        }
    }
}

/// Reason why incremental classification or execution fell back to a clean full build.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FullRebuildReason {
    MissingBaseRevision,
    BaseRevisionCorruptOrIncompatible,
    UnqualifiedChangeChain,
    TruncatedOrEmptyChangeChain,
    InvalidPath,
    RootResync,
    MemberCompositionChanged,
    ExternalOrRootCompositionChanged,
    StageRootMetadataChanged,
    UnknownPrimInfoChange,
    UnknownTransformTarget,
    UnknownGeometryTarget,
    UnrecognizedProperty,
    StructuralResync,
    MemberEntriesChanged,
}

/// Deterministic incremental update plan derived from persisted USD notices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum IncrementalUpdatePlan {
    FullRebuild {
        reason: FullRebuildReason,
    },
    Incremental(IncrementalWorkSet),
}

/// Minimal sets of owned prims requiring targeted updates.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct IncrementalWorkSet {
    pub(crate) metadata_prims: BTreeSet<String>,
    pub(crate) transform_prims: BTreeSet<String>,
    pub(crate) geometry_prims: BTreeSet<String>,
    pub(crate) material_prims: BTreeSet<String>,
    pub(crate) refresh_all_materials: bool,
    pub(crate) animation_prims: BTreeSet<String>,
    pub(crate) refresh_all_animations: bool,
}

impl IncrementalWorkSet {
    pub(crate) fn is_empty(&self) -> bool {
        self.metadata_prims.is_empty()
            && self.transform_prims.is_empty()
            && self.geometry_prims.is_empty()
            && self.material_prims.is_empty()
            && !self.refresh_all_materials
            && self.animation_prims.is_empty()
            && !self.refresh_all_animations
    }
}

/// Coarse property impact class used by [`classify_scene_changes`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PropertyChangeClass {
    MetadataOnly,
    TransformOnly,
    Geometry,
    Material,
    Animation,
    Unknown,
}

/// Classify a single USD property name into its conservative cache impact class.
///
/// Any property not recognized as a known safe category returns
/// [`PropertyChangeClass::Unknown`] so callers fall back to a full rebuild
/// rather than risking stale derived artifacts.
pub(crate) fn classify_property_name(property: &str) -> PropertyChangeClass {
    // BIM / semantic metadata and authored presentation labels.
    if property.starts_with("bim:")
        || property.starts_with("ifc:")
        || property.starts_with("custom:")
        || property.starts_with("usdhub:")
        || property.starts_with("classification:")
        || property.starts_with("pset:")
        || property.starts_with("customData:")
        || matches!(
            property,
            "ui:displayName"
                | "customData"
                | "comment"
                | "documentation"
                | "visibility"
                | "purpose"
        )
    {
        return PropertyChangeClass::MetadataOnly;
    }

    // Transform-only attributes (occurrence transform without geometry re-encoding).
    if property == "xformOpOrder" || property.starts_with("xformOp:") {
        return PropertyChangeClass::TransformOnly;
    }

    // Material / shader / texture / display-color attributes.
    if property == "material:binding"
        || property.starts_with("material:binding:")
        || property.starts_with("inputs:")
        || property.starts_with("outputs:")
        || matches!(
            property,
            "primvars:displayColor"
                | "primvars:displayOpacity"
                | "info:id"
                | "info:implementationSource"
                | "info:sourceAsset"
        )
    {
        return PropertyChangeClass::Material;
    }

    // Skeletal / blend-shape / stage-time animation attributes.
    if matches!(
        property,
        "translations"
            | "rotations"
            | "scales"
            | "blendShapeWeights"
            | "skel:animationSource"
            | "skel:skeleton"
            | "skel:joints"
            | "skel:blendShapes"
            | "skel:blendShapeTargets"
            | "primvars:skel:jointIndices"
            | "primvars:skel:jointWeights"
            | "primvars:skel:geomBindTransform"
            | "startTimeCode"
            | "endTimeCode"
            | "timeCodesPerSecond"
            | "framesPerSecond"
    ) {
        return PropertyChangeClass::Animation;
    }

    // Geometry vertices, topology, normals, and render primvars.
    if property.starts_with("primvars:st")
        || property.starts_with("primvars:uv")
        || property.starts_with("primvars:normals")
        || property.starts_with("primvars:tangents")
        || matches!(
            property,
            "points"
                | "faceVertexCounts"
                | "faceVertexIndices"
                | "normals"
                | "extent"
                | "subdivisionScheme"
                | "orientation"
                | "doubleSided"
                | "cornerIndices"
                | "cornerSharpnesses"
                | "creaseIndices"
                | "creaseLengths"
                | "creaseSharpnesses"
                | "holeIndices"
                | "radius"
                | "size"
                | "height"
                | "axis"
        )
    {
        return PropertyChangeClass::Geometry;
    }

    PropertyChangeClass::Unknown
}

/// Classify a persisted [`SceneDeltaHint`] against a verified base [`SceneCacheIndex`]
/// and current Scene member roots.
pub(crate) fn classify_scene_changes(
    hint: &SceneDeltaHint,
    base_index: &SceneCacheIndex,
    member_roots: &HashSet<String>,
) -> IncrementalUpdatePlan {
    if hint.truncated || hint.changes.is_empty() {
        return IncrementalUpdatePlan::FullRebuild {
            reason: FullRebuildReason::TruncatedOrEmptyChangeChain,
        };
    }

    let owned_prims: HashSet<&str> = base_index
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { prim_path } => Some(prim_path.as_str()),
            SceneCacheEntryKind::ChildScene { .. } | SceneCacheEntryKind::ChildModel { .. } => None,
        })
        .collect();

    let owned_mesh_prims: HashSet<&str> = base_index
        .entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            SceneCacheEntryKind::OwnedPrim { prim_path } if entry.geometry.is_some() => {
                Some(prim_path.as_str())
            }
            _ => None,
        })
        .collect();

    let mut work = IncrementalWorkSet::default();

    // A structural resync does not reveal the old dependency role of a
    // removed/replaced prim. A clean full build is the bounded safe policy
    // until old AND new cross-subtree dependencies can be qualified.
    for change in &hint.changes {
        for resync in &change.resynced {
            if usd_bevy::validate_prim_path(resync).is_err() {
                return IncrementalUpdatePlan::FullRebuild {
                    reason: FullRebuildReason::InvalidPath,
                };
            }
            let normalized = usd_bevy::normalize_prim_path(resync);
            let reason = if normalized == "/" || normalized == "/SceneRoot" {
                FullRebuildReason::RootResync
            } else if touches_member_root(&normalized, member_roots) {
                FullRebuildReason::MemberCompositionChanged
            } else if !usd_bevy::is_descendant_or_self("/SceneRoot", &normalized) {
                FullRebuildReason::ExternalOrRootCompositionChanged
            } else {
                FullRebuildReason::StructuralResync
            };
            return IncrementalUpdatePlan::FullRebuild { reason };
        }
    }

    for change in &hint.changes {
        for info_path in &change.changed_info {
            let prim = usd_bevy::normalize_prim_path(usd_bevy::prim_of(info_path));
            let Some(property) = usd_bevy::property_of(info_path) else {
                return IncrementalUpdatePlan::FullRebuild {
                    reason: FullRebuildReason::UnknownPrimInfoChange,
                };
            };
            if touches_member_root(&prim, member_roots) {
                return IncrementalUpdatePlan::FullRebuild {
                    reason: FullRebuildReason::MemberCompositionChanged,
                };
            }
            let class = classify_property_name(property);
            if prim == "/" || prim == "/SceneRoot" {
                if class == PropertyChangeClass::Animation {
                    work.refresh_all_animations = true;
                    continue;
                }
                return IncrementalUpdatePlan::FullRebuild {
                    reason: FullRebuildReason::StageRootMetadataChanged,
                };
            }
            if !usd_bevy::is_descendant_or_self("/SceneRoot", &prim) {
                match class {
                    PropertyChangeClass::Material => {
                        work.refresh_all_materials = true;
                        continue;
                    }
                    PropertyChangeClass::Animation => {
                        work.refresh_all_animations = true;
                        continue;
                    }
                    _ => {
                        return IncrementalUpdatePlan::FullRebuild {
                            reason: FullRebuildReason::ExternalOrRootCompositionChanged,
                        };
                    }
                }
            }

            match class {
                PropertyChangeClass::MetadataOnly => {
                    if owned_prims.contains(prim.as_str()) {
                        work.metadata_prims.insert(prim);
                    }
                }
                PropertyChangeClass::TransformOnly => {
                    if owned_prims.contains(prim.as_str()) {
                        work.transform_prims.insert(prim);
                    } else {
                        return IncrementalUpdatePlan::FullRebuild {
                            reason: FullRebuildReason::UnknownTransformTarget,
                        };
                    }
                }
                PropertyChangeClass::Geometry => {
                    if owned_prims.contains(prim.as_str()) {
                        work.geometry_prims.insert(prim);
                    } else {
                        return IncrementalUpdatePlan::FullRebuild {
                            reason: FullRebuildReason::UnknownGeometryTarget,
                        };
                    }
                }
                PropertyChangeClass::Material => {
                    if owned_mesh_prims.contains(prim.as_str()) {
                        work.material_prims.insert(prim);
                    } else {
                        // A Material or Shader prim (or ancestor binding) changed:
                        // refresh material payloads without re-extracting geometry.
                        work.refresh_all_materials = true;
                    }
                }
                PropertyChangeClass::Animation => {
                    if owned_mesh_prims.contains(prim.as_str()) {
                        work.animation_prims.insert(prim);
                    } else {
                        // A SkelAnimation or Skeleton prim changed: refresh animation
                        // blobs without re-extracting static geometry.
                        work.refresh_all_animations = true;
                    }
                }
                PropertyChangeClass::Unknown => {
                    return IncrementalUpdatePlan::FullRebuild {
                        reason: FullRebuildReason::UnrecognizedProperty,
                    };
                }
            }
        }
    }

    IncrementalUpdatePlan::Incremental(work)
}

fn touches_member_root(path: &str, member_roots: &HashSet<String>) -> bool {
    member_roots.iter().any(|member| {
        usd_bevy::is_descendant_or_self(member, path)
            || usd_bevy::is_descendant_or_self(path, member)
    })
}
