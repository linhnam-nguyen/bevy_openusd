use std::fs;
use std::path::Path;

use anyhow::Result;
use openusd::sdf;
use openusd::usd::TimeCode;
use openusd::vt::Value;
use tempfile::tempdir;
use usd_model::HashDigest;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, SceneMember,
    SceneMemberId, SceneMemberTarget, ScenePlacementTransform, StorageKey,
};
use viewport_protocol::RuntimeProfile;

use crate::cache::changes::{
    FullRebuildReason, SceneDeltaHint, capture_scene_delta_base, complete_scene_delta_save,
};
use crate::cache::contract::{
    ProjectCacheTarget, SceneCacheDescriptorV3, SceneCacheEntry, SceneCacheEntryKind,
    SceneCacheIndex, SceneCacheState,
};
use crate::cache::identity::compute_scene_revision_identity;
use crate::cache::retention::ImmutableRevisionStore;
use crate::cache::scene::build::build_scene_cache_index_from_snapshot;
use crate::cache::scene::store::SceneCacheStore;
use crate::cache::scene::update::IncrementalBuildStats;
use crate::cache::warming::runtime::build_and_publish_managed_scene_cache_generation_with_delta;
use crate::project::catalog::manifest_store::ManifestStore;
use crate::project::scene::authoring::{
    author_scene_atomic, replace_scene_members_atomic, scene_path,
};

fn vec3f(x: f32, y: f32, z: f32) -> openusd::gf::Vec3f {
    openusd::gf::Vec3f { x, y, z }
}

fn vec3d(x: f64, y: f64, z: f64) -> openusd::gf::Vec3d {
    openusd::gf::Vec3d { x, y, z }
}

fn init_managed_scene_project(usda: &str) -> Result<(tempfile::TempDir, SceneId, std::path::PathBuf)> {
    let dir = tempdir()?;
    usd_git::Repository::init(dir.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Incremental Cache Test Project",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("main-scene").expect("valid storage key"),
            display_name: "Main Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(dir.path(), &manifest)?;
    let path = scene_path(dir.path(), scene_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let rendered = usda.replace("__SCENE_ID__", &scene_id.to_string());
    fs::write(&path, rendered)?;
    Ok((dir, scene_id, path))
}

fn default_config_hash() -> HashDigest {
    crate::cache::compatibility::project_runtime_cache_config_hash(
        usd_semantic::SemanticConfig::default().hash(),
    )
}

fn build_and_publish(
    project_root: &Path,
    scene_id: SceneId,
    delta_hint: Option<&SceneDeltaHint>,
) -> Result<(SceneCacheDescriptorV3, SceneCacheIndex, IncrementalBuildStats)> {
    let config = default_config_hash();
    let store = SceneCacheStore::new(project_root);
    let generation = store.advance_generation(scene_id, config)?;
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, generation, config);
    descriptor.state = SceneCacheState::Ready;
    let (index, stats) =
        build_and_publish_managed_scene_cache_generation_with_delta(project_root, &descriptor, delta_hint)?
            .expect("managed Scene cache generation must commit");
    let committed = store
        .load_descriptor(scene_id)?
        .expect("committed descriptor must exist");
    Ok((committed, index, stats))
}

fn revision_digest(project_root: &Path, scene_id: SceneId) -> Result<HashDigest> {
    Ok(compute_scene_revision_identity(
        project_root, scene_id, RuntimeProfile::NativeMedium, default_config_hash(),
    )?.digest())
}

fn save_with_qualified_delta(
    project_root: &Path,
    scene_id: SceneId,
    path: &Path,
    live: &usd_bevy::LiveStage,
) -> Result<Option<SceneDeltaHint>> {
    let base = capture_scene_delta_base(project_root, scene_id, live);
    usd_bevy::authoring::save_stage_as(&live.stage, path.to_str().expect("utf8 path"))?;
    Ok(complete_scene_delta_save(project_root, scene_id, live, base))
}

fn assert_equivalent_to_full_build(
    project_root: &Path,
    scene_id: SceneId,
    generation: u64,
    incremental_index: &SceneCacheIndex,
) -> Result<()> {
    let path = scene_path(project_root, scene_id);
    let live = usd_bevy::LiveStage::open_document(&path)?;
    let (full_index, full_spatial) =
        build_scene_cache_index_from_snapshot(project_root, scene_id, generation, &live)?;
    let incremental_spatial = SceneCacheStore::new(project_root)
        .load_spatial(scene_id)?
        .expect("published spatial index must exist");
    assert_eq!(
        *incremental_index, full_index,
        "incremental SceneCacheIndex must match clean full build"
    );
    assert_eq!(
        incremental_spatial, full_spatial,
        "incremental SceneSpatialIndex must match clean full build"
    );
    Ok(())
}

fn owned_entry<'a>(index: &'a SceneCacheIndex, path: &str) -> &'a SceneCacheEntry {
    index
        .entries
        .iter()
        .find(|entry| {
            matches!(
                &entry.kind,
                SceneCacheEntryKind::OwnedPrim { prim_path } if prim_path == path
            )
        })
        .unwrap_or_else(|| panic!("missing owned entry {path}"))
}

const TWO_BOX_SCENE_USDA: &str = r#"#usda 1.0
(
    defaultPrim = "SceneRoot"
    metersPerUnit = 1
    upAxis = "Y"
)

def Xform "SceneRoot" (
    customData = {
        string "usdhub:sceneId" = "__SCENE_ID__"
        int "usdhub:schemaVersion" = 1
    }
)
{
    def Scope "Looks"
    {
        def Material "MatA"
        {
            token outputs:surface.connect = </SceneRoot/Looks/MatA/Shader.outputs:surface>
            def Shader "Shader"
            {
                uniform token info:id = "UsdPreviewSurface"
                color3f inputs:diffuseColor = (0.8, 0.2, 0.2)
                token outputs:surface
            }
        }
    }

    def Xform "GroupA"
    {
        def Mesh "BoxA" (
            prepend apiSchemas = ["MaterialBindingAPI"]
        )
        {
            float3[] points = [(-1, -1, 0), (1, -1, 0), (1, 1, 0), (-1, 1, 0)]
            int[] faceVertexCounts = [4]
            int[] faceVertexIndices = [0, 1, 2, 3]
            rel material:binding = </SceneRoot/Looks/MatA>
            double3 xformOp:translate = (0, 0, 0)
            uniform token[] xformOpOrder = ["xformOp:translate"]
            custom string bim:fireRating = "60min"
        }
    }

    def Xform "GroupB"
    {
        def Mesh "BoxB"
        {
            float3[] points = [(2, -1, 0), (4, -1, 0), (4, 1, 0), (2, 1, 0)]
            int[] faceVertexCounts = [4]
            int[] faceVertexIndices = [0, 1, 2, 3]
            double3 xformOp:translate = (5, 0, 0)
            uniform token[] xformOpOrder = ["xformOp:translate"]
        }
    }
}
"#;

const SKINNED_ANIM_SCENE_USDA: &str = r#"#usda 1.0
(
    defaultPrim = "SceneRoot"
    metersPerUnit = 1
    upAxis = "Y"
    startTimeCode = 0
    endTimeCode = 10
    timeCodesPerSecond = 24
)

def Xform "SceneRoot" (
    customData = {
        string "usdhub:sceneId" = "__SCENE_ID__"
        int "usdhub:schemaVersion" = 1
    }
)
{
    def SkelRoot "ArmRoot"
    {
        def Skeleton "Skel"
        {
            uniform token[] joints = ["Root"]
            uniform matrix4d[] bindTransforms = [( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )]
            uniform matrix4d[] restTransforms = [( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )]
            rel skel:animationSource = </SceneRoot/ArmRoot/Anim>
        }

        def SkelAnimation "Anim"
        {
            uniform token[] joints = ["Root"]
            float3[] translations.timeSamples = {
                0: [(0, 0, 0)],
                10: [(0, 2, 0)],
            }
            quatf[] rotations = [(1, 0, 0, 0)]
            half3[] scales = [(1, 1, 1)]
        }

        def Mesh "ArmMesh" (
            prepend apiSchemas = ["SkelBindingAPI"]
        )
        {
            float3[] points = [(-1, -1, 0), (1, -1, 0), (1, 1, 0)]
            int[] faceVertexCounts = [3]
            int[] faceVertexIndices = [0, 1, 2]
            int[] primvars:skel:jointIndices = [0, 0, 0] (
                elementSize = 1
                interpolation = "vertex"
            )
            float[] primvars:skel:jointWeights = [1, 1, 1] (
                elementSize = 1
                interpolation = "vertex"
            )
            rel skel:skeleton = </SceneRoot/ArmRoot/Skel>
        }
    }
}
"#;

#[test]
fn bim_metadata_edit_updates_semantic_fields_without_rebuilding_geometry() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (_base_desc, base_index, _) = build_and_publish(dir.path(), scene_id, None)?;

    let live = usd_bevy::LiveStage::open_document(&path)?;
    // Drain per-frame projection queue to verify persisted delta log is independent.
    let _ = live.drain_change_batch();
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxA",
        "bim:fireRating",
        "string",
        Value::String("120min".to_owned()),
    )?;
    let _ = live.drain_change_batch();
    let snapshot = live.snapshot_persisted_changes();
    assert!(!snapshot.is_empty(), "persisted changes must survive frame drains");
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("BIM metadata interval must qualify");
    let (next_desc, next_index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;

    assert!(stats.used_incremental, "BIM metadata edit must use incremental path");
    assert_eq!(stats.metadata_prims_updated, 1);
    assert_eq!(stats.geometry_prims_rebuilt, 0, "BIM edit must never rebuild geometry");
    assert_eq!(stats.material_prims_rebuilt, 0, "BIM edit must never rebuild materials");
    assert_eq!(stats.animation_prims_rebuilt, 0, "BIM edit must never rebuild animations");
    assert_eq!(stats.geometry_blobs_reused, 2);
    assert_eq!(
        owned_entry(&next_index, "/SceneRoot/GroupA/BoxA").geometry,
        owned_entry(&base_index, "/SceneRoot/GroupA/BoxA").geometry
    );
    assert_eq!(
        owned_entry(&next_index, "/SceneRoot/GroupB/BoxB").geometry,
        owned_entry(&base_index, "/SceneRoot/GroupB/BoxB").geometry
    );
    assert_equivalent_to_full_build(dir.path(), scene_id, next_desc.generation, &next_index)?;
    Ok(())
}

#[test]
fn transform_only_edit_updates_occurrence_transform_and_reuses_geometry() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (_base_desc, base_index, _) = build_and_publish(dir.path(), scene_id, None)?;

    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxA",
        "xformOp:translate",
        "double3",
        Value::Vec3d(vec3d(12.5, -3.0, 4.25)),
    )?;
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("ordinary persisted change must qualify");
    let (next_desc, next_index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;

    assert!(stats.used_incremental, "transform-only edit must use incremental path");
    assert_eq!(stats.transform_prims_updated, 1);
    assert_eq!(stats.geometry_prims_rebuilt, 0, "transform edit must never rebuild geometry");
    assert_eq!(stats.geometry_blobs_reused, 2);
    assert_ne!(
        owned_entry(&next_index, "/SceneRoot/GroupA/BoxA").transform,
        owned_entry(&base_index, "/SceneRoot/GroupA/BoxA").transform
    );
    assert_eq!(
        owned_entry(&next_index, "/SceneRoot/GroupA/BoxA").geometry,
        owned_entry(&base_index, "/SceneRoot/GroupA/BoxA").geometry
    );
    assert_equivalent_to_full_build(dir.path(), scene_id, next_desc.generation, &next_index)?;
    Ok(())
}

#[test]
fn localized_geometry_edit_rebuilds_only_affected_mesh_and_reuses_unchanged_blobs() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (_base_desc, base_index, _) = build_and_publish(dir.path(), scene_id, None)?;

    let live = usd_bevy::LiveStage::open_document(&path)?;
    let new_points = vec![
        vec3f(-2.0, -2.0, 0.0),
        vec3f(2.0, -2.0, 0.0),
        vec3f(2.0, 2.0, 0.0),
        vec3f(-2.0, 2.0, 0.0),
    ];
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxA",
        "points",
        "point3f[]",
        Value::Vec3fVec(new_points),
    )?;
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("ordinary persisted change must qualify");
    let (next_desc, next_index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;

    assert!(stats.used_incremental, "single-prim geometry edit must use incremental path");
    assert_eq!(stats.geometry_prims_rebuilt, 1, "only edited mesh must be re-extracted");
    assert_eq!(stats.geometry_blobs_reused, 1, "unchanged mesh blob must be reused");
    assert_ne!(
        owned_entry(&next_index, "/SceneRoot/GroupA/BoxA").geometry,
        owned_entry(&base_index, "/SceneRoot/GroupA/BoxA").geometry
    );
    assert_eq!(
        owned_entry(&next_index, "/SceneRoot/GroupB/BoxB").geometry,
        owned_entry(&base_index, "/SceneRoot/GroupB/BoxB").geometry
    );
    assert_equivalent_to_full_build(dir.path(), scene_id, next_desc.generation, &next_index)?;
    Ok(())
}

#[test]
fn material_shader_edit_rebuilds_material_without_reextracting_geometry() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (_base_desc, base_index, _) = build_and_publish(dir.path(), scene_id, None)?;

    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/Looks/MatA/Shader",
        "inputs:diffuseColor",
        "color3f",
        Value::Vec3f(vec3f(0.1, 0.9, 0.2)),
    )?;
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("ordinary persisted change must qualify");
    let (next_desc, next_index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;

    assert!(stats.used_incremental, "material shader edit must use incremental path");
    assert_eq!(stats.geometry_prims_rebuilt, 0, "material edit must never rebuild geometry");
    assert_eq!(stats.geometry_blobs_reused, 2);
    assert!(stats.material_prims_rebuilt >= 1);
    assert_ne!(
        owned_entry(&next_index, "/SceneRoot/GroupA/BoxA").material,
        owned_entry(&base_index, "/SceneRoot/GroupA/BoxA").material
    );
    assert_eq!(
        owned_entry(&next_index, "/SceneRoot/GroupA/BoxA").geometry,
        owned_entry(&base_index, "/SceneRoot/GroupA/BoxA").geometry
    );
    assert_equivalent_to_full_build(dir.path(), scene_id, next_desc.generation, &next_index)?;
    Ok(())
}

#[test]
fn animation_only_edit_rebuilds_animation_blob_and_reuses_static_geometry() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(SKINNED_ANIM_SCENE_USDA)?;
    let (_base_desc, base_index, _) = build_and_publish(dir.path(), scene_id, None)?;
    assert!(
        owned_entry(&base_index, "/SceneRoot/ArmRoot/ArmMesh")
            .animation
            .is_some(),
        "base skinned mesh must have an animation blob"
    );

    let live = usd_bevy::LiveStage::open_document(&path)?;
    let attr = live
        .stage
        .attribute(sdf::path("/SceneRoot/ArmRoot/Anim.translations")?)?;
    attr.set_at(Value::Vec3fVec(vec![vec3f(0.0, 7.5, 0.0)]), TimeCode::new(10.0))?;
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("ordinary persisted change must qualify");
    let (next_desc, next_index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;

    assert!(stats.used_incremental, "animation-only edit must use incremental path");
    assert_eq!(
        stats.geometry_prims_rebuilt, 0,
        "animation edit must never re-extract static geometry"
    );
    assert_eq!(stats.geometry_blobs_reused, 1);
    assert_eq!(stats.animation_prims_rebuilt, 1);
    assert_ne!(
        owned_entry(&next_index, "/SceneRoot/ArmRoot/ArmMesh").animation,
        owned_entry(&base_index, "/SceneRoot/ArmRoot/ArmMesh").animation
    );
    assert_eq!(
        owned_entry(&next_index, "/SceneRoot/ArmRoot/ArmMesh").geometry,
        owned_entry(&base_index, "/SceneRoot/ArmRoot/ArmMesh").geometry
    );
    assert_equivalent_to_full_build(dir.path(), scene_id, next_desc.generation, &next_index)?;
    Ok(())
}

#[test]
fn structural_add_uses_conservative_full_build_with_identical_output() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (base_desc, base_index, _) = build_and_publish(dir.path(), scene_id, None)?;

    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::define_prim(&live.stage, "/SceneRoot/GroupA/BoxC", "Mesh")?;
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxC",
        "points",
        "point3f[]",
        Value::Vec3fVec(vec![
            vec3f(0.0, 0.0, 0.0),
            vec3f(1.0, 0.0, 0.0),
            vec3f(0.0, 1.0, 0.0),
        ]),
    )?;
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxC",
        "faceVertexCounts",
        "int[]",
        Value::IntVec(vec![3]),
    )?;
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxC",
        "faceVertexIndices",
        "int[]",
        Value::IntVec(vec![0, 1, 2]),
    )?;
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?;
    let structural = SceneDeltaHint {
        base_revision_identity: base_desc.revision_identity,
        result_revision_identity: Some(revision_digest(dir.path(), scene_id)?),
        changes: vec![usd_bevy::StageChange {
            resynced: vec!["/SceneRoot/GroupA/BoxC".to_owned()], changed_info: vec![],
        }],
        truncated: false,
    };
    let hint = hint.unwrap_or(structural);
    let (next_desc, next_index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;

    assert!(!stats.used_incremental, "unqualified structural edit requires full rebuild");
    assert_eq!(stats.fallback_reason, Some(FullRebuildReason::StructuralResync));
    assert!(owned_entry(&next_index, "/SceneRoot/GroupA/BoxC").geometry.is_some());
    assert_eq!(
        owned_entry(&next_index, "/SceneRoot/GroupB/BoxB").geometry,
        owned_entry(&base_index, "/SceneRoot/GroupB/BoxB").geometry
    );
    assert_equivalent_to_full_build(dir.path(), scene_id, next_desc.generation, &next_index)?;
    Ok(())
}

#[test]
fn unknown_property_and_root_resync_fall_back_conservatively_to_full_build() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (base_desc, _, _) = build_and_publish(dir.path(), scene_id, None)?;

    // 1. Unrecognized property falls back cleanly to full rebuild.
    let mut hint = SceneDeltaHint {
        base_revision_identity: base_desc.revision_identity,
        result_revision_identity: None,
        changes: vec![usd_bevy::StageChange {
            resynced: Vec::new(),
            changed_info: vec!["/SceneRoot/GroupA/BoxA.mysteryOpaqueField".to_owned()],
        }],
        truncated: false,
    };
    // Edit file slightly so revision_identity differs and doesn't hit exact-revision reuse.
    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxA",
        "bim:fireRating",
        "string",
        Value::String("90min".to_owned()),
    )?;
    usd_bevy::authoring::save_stage_as(&live.stage, path.to_str().expect("utf8 path"))?;

    hint.result_revision_identity = Some(revision_digest(dir.path(), scene_id)?);
    let (next_desc, next_index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;
    assert!(!stats.used_incremental);
    assert_eq!(
        stats.fallback_reason,
        Some(FullRebuildReason::UnrecognizedProperty)
    );
    assert_equivalent_to_full_build(dir.path(), scene_id, next_desc.generation, &next_index)?;

    // 2. Root resync "/" falls back cleanly to full rebuild.
    let live2 = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(
        &live2.stage,
        "/SceneRoot/GroupA/BoxA",
        "bim:fireRating",
        "string",
        Value::String("180min".to_owned()),
    )?;
    usd_bevy::authoring::save_stage_as(&live2.stage, path.to_str().expect("utf8 path"))?;
    let root_hint = SceneDeltaHint {
        base_revision_identity: next_desc.revision_identity,
        result_revision_identity: Some(revision_digest(dir.path(), scene_id)?),
        changes: vec![usd_bevy::StageChange {
            resynced: vec!["/".to_owned()],
            changed_info: Vec::new(),
        }],
        truncated: false,
    };
    let (third_desc, third_index, third_stats) =
        build_and_publish(dir.path(), scene_id, Some(&root_hint))?;
    assert!(!third_stats.used_incremental);
    assert_eq!(third_stats.fallback_reason, Some(FullRebuildReason::RootResync));
    assert_equivalent_to_full_build(dir.path(), scene_id, third_desc.generation, &third_index)?;
    Ok(())
}

#[test]
fn unsaved_live_edits_and_transient_state_do_not_mutate_persistent_cache() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (base_desc, base_index, _) = build_and_publish(dir.path(), scene_id, None)?;

    let live = usd_bevy::LiveStage::open_document(&path)?;
    // Apply an unsaved live edit in memory only.
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxA",
        "xformOp:translate",
        "double3",
        Value::Vec3d(vec3d(99.0, 0.0, 0.0)),
    )?;
    // Persisted descriptor & index on disk must be completely untouched.
    let store = SceneCacheStore::new(dir.path());
    assert_eq!(store.load_descriptor(scene_id)?, Some(base_desc));
    assert_eq!(store.load_index(scene_id)?, Some(base_index));

    // Simulating a failed save (not clearing persisted changes) preserves the chain.
    let first_snapshot = live.snapshot_persisted_changes();
    assert!(!first_snapshot.is_empty());
    let second_snapshot = live.snapshot_persisted_changes();
    assert_eq!(first_snapshot, second_snapshot);
    Ok(())
}

#[test]
fn missing_or_corrupt_base_revision_falls_back_to_full_build_cleanly() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (base_desc, _, _) = build_and_publish(dir.path(), scene_id, None)?;

    // Corrupt the retained base revision index on disk.
    let base_key = base_desc
        .revision_identity
        .expect("base revision identity")
        .to_hex();
    let revision_store = ImmutableRevisionStore::new(dir.path());
    let corrupt_index_path = revision_store.revision_dir(&base_key).join("index.bin");
    fs::write(&corrupt_index_path, b"corrupted-json-payload")?;

    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxA",
        "bim:fireRating",
        "string",
        Value::String("240min".to_owned()),
    )?;
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("ordinary persisted change must qualify");
    let (next_desc, next_index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;

    assert!(!stats.used_incremental, "corrupt base revision must fall back to full build");
    assert_eq!(
        stats.fallback_reason,
        Some(FullRebuildReason::MissingBaseRevision)
    );
    assert_eq!(next_desc.state, SceneCacheState::Ready);
    assert_equivalent_to_full_build(dir.path(), scene_id, next_desc.generation, &next_index)?;
    Ok(())
}

#[test]
fn parent_scene_dependency_propagation_does_not_copy_child_geometry() -> Result<()> {
    let dir = tempdir()?;
    usd_git::Repository::init(dir.path())?;
    let parent_id = SceneId::new_v4();
    let child_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Parent Child Incremental Project",
        ProjectRoot::Scene(parent_id),
        vec![
            SceneManifestEntry {
                id: parent_id,
                storage_key: StorageKey::new("parent-scene").expect("valid key"),
                display_name: "Parent Scene".to_owned(),
            },
            SceneManifestEntry {
                id: child_id,
                storage_key: StorageKey::new("child-scene").expect("valid key"),
                display_name: "Child Scene".to_owned(),
            },
        ],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(dir.path(), &manifest)?;
    author_scene_atomic(dir.path(), parent_id)?;
    let child_path = scene_path(dir.path(), child_id);
    if let Some(parent) = child_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &child_path,
        TWO_BOX_SCENE_USDA.replace("__SCENE_ID__", &child_id.to_string()),
    )?;

    let parent_path = scene_path(dir.path(), parent_id);
    let member = SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Scene(child_id),
        transform: ScenePlacementTransform::identity(),
        name: Some("Child Scene".to_owned()),
    };
    replace_scene_members_atomic(&parent_path, dir.path(), parent_id, &[member])?;

    let (_base_child_desc, base_child_index, _) = build_and_publish(dir.path(), child_id, None)?;
    let (_, base_parent_index, _) = build_and_publish(dir.path(), parent_id, None)?;
    // Parent Scene must reference Child Scene via ChildScene member entry without copying child geometry.
    assert!(
        base_parent_index
            .entries
            .iter()
            .all(|entry| entry.geometry.is_none()),
        "parent Scene must not duplicate child Scene geometry blobs"
    );

    // Verify expand_affected_targets propagates from child -> parent + ProjectRoot.
    let expanded = crate::cache::warming::expand_affected_targets(
        dir.path(),
        vec![ProjectCacheTarget::Scene {
            id: child_id.to_string(),
        }],
    );
    assert!(expanded.contains(&ProjectCacheTarget::Scene {
        id: child_id.to_string()
    }));
    assert!(expanded.contains(&ProjectCacheTarget::Scene {
        id: parent_id.to_string()
    }));
    assert!(expanded.contains(&ProjectCacheTarget::ProjectRoot));

    // Edit child BIM metadata, verify child updates incrementally and parent revision advances cleanly.
    let live = usd_bevy::LiveStage::open_document(&child_path)?;
    usd_bevy::authoring::set_attribute(
        &live.stage,
        "/SceneRoot/GroupA/BoxA",
        "bim:fireRating",
        "string",
        Value::String("90min".to_owned()),
    )?;
    let hint = save_with_qualified_delta(dir.path(), child_id, &child_path, &live)?
        .expect("child persisted change must qualify");
    let (next_child_desc, next_child_index, child_stats) =
        build_and_publish(dir.path(), child_id, Some(&hint))?;
    assert!(child_stats.used_incremental);
    assert_eq!(child_stats.geometry_prims_rebuilt, 0);
    assert_eq!(
        owned_entry(&next_child_index, "/SceneRoot/GroupA/BoxA").geometry,
        owned_entry(&base_child_index, "/SceneRoot/GroupA/BoxA").geometry
    );
    assert_equivalent_to_full_build(dir.path(), child_id, next_child_desc.generation, &next_child_index)?;

    let parent_identity = compute_scene_revision_identity(
        dir.path(),
        parent_id,
        RuntimeProfile::NativeMedium,
        default_config_hash(),
    )?;
    let (next_parent_desc, next_parent_index, _) = build_and_publish(dir.path(), parent_id, None)?;
    assert_eq!(next_parent_desc.revision_identity, Some(parent_identity.digest()));
    assert!(
        next_parent_index
            .entries
            .iter()
            .all(|entry| entry.geometry.is_none()),
        "updated parent Scene must not duplicate child Scene geometry blobs"
    );
    Ok(())
}

#[test]
fn external_change_before_open_uses_actual_base_not_retained_pointer() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (a, a_index, _) = build_and_publish(dir.path(), scene_id, None)?;
    let external = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(&external.stage, "/SceneRoot/GroupB/BoxB", "points",
        "point3f[]", Value::Vec3fVec(vec![vec3f(2.0, -2.0, 0.0), vec3f(6.0, -2.0, 0.0),
            vec3f(6.0, 2.0, 0.0), vec3f(2.0, 2.0, 0.0)]))?;
    usd_bevy::authoring::save_stage_as(&external.stage, path.to_str().expect("utf8 path"))?;
    let b = revision_digest(dir.path(), scene_id)?;
    assert_ne!(a.revision_identity, Some(b));
    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(&live.stage, "/SceneRoot/GroupA/BoxA", "bim:fireRating",
        "string", Value::String("120min".to_owned()))?;
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("B is the actual persisted notice base");
    assert_eq!(hint.base_revision_identity, Some(b));
    let (c, index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;
    assert!(!stats.used_incremental, "no retained B artifact exists");
    assert_ne!(owned_entry(&index, "/SceneRoot/GroupB/BoxB").geometry,
        owned_entry(&a_index, "/SceneRoot/GroupB/BoxB").geometry);
    assert_equivalent_to_full_build(dir.path(), scene_id, c.generation, &index)
}

#[test]
fn consecutive_saves_merge_exact_intervals_and_reject_wrong_result() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (a, a_index, _) = build_and_publish(dir.path(), scene_id, None)?;
    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(&live.stage, "/SceneRoot/GroupA/BoxA", "points",
        "point3f[]", Value::Vec3fVec(vec![vec3f(-2.0, -2.0, 0.0), vec3f(2.0, -2.0, 0.0),
            vec3f(2.0, 2.0, 0.0), vec3f(-2.0, 2.0, 0.0)]))?;
    let ab = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("first persisted interval");
    usd_bevy::authoring::set_attribute(&live.stage, "/SceneRoot/GroupB/BoxB", "xformOp:translate",
        "double3", Value::Vec3d(vec3d(20.0, 0.0, 0.0)))?;
    let bc = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("second persisted interval on the same live stage");
    assert_eq!(ab.base_revision_identity, a.revision_identity);
    assert_eq!(ab.result_revision_identity, bc.base_revision_identity);
    assert!(SceneDeltaHint::coalesce(None, Some(bc.clone())).is_none());
    assert!(SceneDeltaHint::coalesce(Some(ab.clone()), None).is_none());
    assert!(SceneDeltaHint::coalesce(Some(bc.clone()), Some(ab.clone())).is_none());
    let ac = SceneDeltaHint::coalesce(Some(ab.clone()), Some(bc.clone()))
        .expect("adjacent qualified intervals merge");
    assert_eq!(ac.base_revision_identity, a.revision_identity);
    assert_eq!(ac.result_revision_identity, bc.result_revision_identity);
    // Wrong result must be rejected before the incremental builder. Do this
    // first so exact C reuse cannot mask the mismatch test.
    let (c, index, stats) = build_and_publish(dir.path(), scene_id, Some(&ab))?;
    assert!(!stats.used_incremental);
    assert_eq!(stats.fallback_reason, Some(FullRebuildReason::UnqualifiedChangeChain));
    assert_ne!(owned_entry(&index, "/SceneRoot/GroupA/BoxA").geometry,
        owned_entry(&a_index, "/SceneRoot/GroupA/BoxA").geometry);
    assert_ne!(owned_entry(&index, "/SceneRoot/GroupB/BoxB").transform,
        owned_entry(&a_index, "/SceneRoot/GroupB/BoxB").transform);
    assert_equivalent_to_full_build(dir.path(), scene_id, c.generation, &index)?;
    Ok(())
}

#[test]
fn consecutive_save_chain_reuses_original_base_without_intermediate_build() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (_, a, _) = build_and_publish(dir.path(), scene_id, None)?;
    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(&live.stage, "/SceneRoot/GroupA/BoxA", "points",
        "point3f[]", Value::Vec3fVec(vec![vec3f(-2.0, -2.0, 0.0), vec3f(2.0, -2.0, 0.0),
            vec3f(2.0, 2.0, 0.0), vec3f(-2.0, 2.0, 0.0)]))?;
    let ab = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?;
    usd_bevy::authoring::set_attribute(&live.stage, "/SceneRoot/GroupB/BoxB", "xformOp:translate",
        "double3", Value::Vec3d(vec3d(25.0, 0.0, 0.0)))?;
    let bc = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?;
    let ac = SceneDeltaHint::coalesce(ab, bc).expect("adjacent intervals");
    let (c, index, stats) = build_and_publish(dir.path(), scene_id, Some(&ac))?;
    assert!(stats.used_incremental);
    assert_eq!(stats.geometry_prims_rebuilt, 1);
    assert_eq!(stats.geometry_blobs_reused, 1);
    assert_eq!(owned_entry(&index, "/SceneRoot/GroupB/BoxB").geometry,
        owned_entry(&a, "/SceneRoot/GroupB/BoxB").geometry);
    assert_equivalent_to_full_build(dir.path(), scene_id, c.generation, &index)
}

#[test]
fn external_change_after_open_invalidates_persisted_notice_baseline() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (_, _, _) = build_and_publish(dir.path(), scene_id, None)?;
    let live = usd_bevy::LiveStage::open_document(&path)?;
    let external = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(&external.stage, "/SceneRoot/GroupB/BoxB", "xformOp:translate",
        "double3", Value::Vec3d(vec3d(42.0, 0.0, 0.0)))?;
    usd_bevy::authoring::save_stage_as(&external.stage, path.to_str().expect("utf8 path"))?;
    usd_bevy::authoring::set_attribute(&live.stage, "/SceneRoot/GroupA/BoxA", "bim:fireRating",
        "string", Value::String("180min".to_owned()))?;
    assert!(capture_scene_delta_base(dir.path(), scene_id, &live).is_none());
    // This test checks cache admission, not policy for resolving canonical
    // editor write conflicts. Preserve the existing save behavior.
    Ok(())
}

#[test]
fn removed_shared_material_subtree_cannot_reuse_old_consumer_payload() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (a, index_a, _) = build_and_publish(dir.path(), scene_id, None)?;
    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::remove_prim(&live.stage, "/SceneRoot/Looks/MatA")?;
    let base = capture_scene_delta_base(dir.path(), scene_id, &live);
    usd_bevy::authoring::save_stage_as(&live.stage, path.to_str().expect("utf8 path"))?;
    let _hint = complete_scene_delta_save(dir.path(), scene_id, &live, base);
    // Isolate structural classification even if composition changes already
    // made the production hint unavailable.
    let structural = SceneDeltaHint {
        base_revision_identity: a.revision_identity,
        result_revision_identity: Some(revision_digest(dir.path(), scene_id)?),
        changes: vec![usd_bevy::StageChange {
            resynced: vec!["/SceneRoot/Looks/MatA".to_owned()], changed_info: vec![],
        }],
        truncated: false,
    };
    let (b, index_b, stats) = build_and_publish(dir.path(), scene_id, Some(&structural))?;
    assert!(!stats.used_incremental);
    assert!(owned_entry(&index_a, "/SceneRoot/GroupA/BoxA").material.is_some());
    assert_equivalent_to_full_build(dir.path(), scene_id, b.generation, &index_b)
}

#[test]
fn managed_timeline_save_qualifies_persisted_metadata_interval() -> Result<()> {
    // Explicit user range is preserved by the real canonical Scene save owner.
    let source = TWO_BOX_SCENE_USDA.replace("    upAxis = \"Y\"", "    upAxis = \"Y\"\n    startTimeCode = 0\n    endTimeCode = 10");
    let (dir, scene_id, path) = init_managed_scene_project(&source)?;
    let (a, _, _) = build_and_publish(dir.path(), scene_id, None)?;
    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(&live.stage, "/SceneRoot/GroupA/BoxA", "bim:fireRating",
        "string", Value::String("120min".to_owned()))?;
    assert!(live.try_freeze_authoring());
    let base = capture_scene_delta_base(dir.path(), scene_id, &live);
    let manifest = ManifestStore::read_validated(dir.path())?;
    let result = crate::project::scene::timeline::save_scene_with_timelines_atomic(
        &live.stage, dir.path(), manifest.raw(), scene_id, &path,
    );
    // The production helper is deliberately called only after canonical success.
    let hint = match result {
        Ok(_) => complete_scene_delta_save(dir.path(), scene_id, &live, base),
        Err(error) => { live.unfreeze_authoring(); return Err(error); }
    };
    live.unfreeze_authoring();
    let hint = hint.expect("metadata save with preserved source interval must qualify");
    assert_eq!(hint.base_revision_identity, a.revision_identity);
    let (b, index, stats) = build_and_publish(dir.path(), scene_id, Some(&hint))?;
    assert!(stats.used_incremental);
    assert_eq!(stats.geometry_prims_rebuilt, 0);
    assert_equivalent_to_full_build(dir.path(), scene_id, b.generation, &index)
}

#[test]
fn incompatible_or_missing_interval_is_never_used_for_sparse_build() -> Result<()> {
    let (dir, scene_id, path) = init_managed_scene_project(TWO_BOX_SCENE_USDA)?;
    let (a, _, _) = build_and_publish(dir.path(), scene_id, None)?;
    let live = usd_bevy::LiveStage::open_document(&path)?;
    usd_bevy::authoring::set_attribute(&live.stage, "/SceneRoot/GroupA/BoxA", "bim:fireRating",
        "string", Value::String("120min".to_owned()))?;
    let hint = save_with_qualified_delta(dir.path(), scene_id, &path, &live)?
        .expect("ordinary persisted interval");
    let target = compute_scene_revision_identity(dir.path(), scene_id,
        RuntimeProfile::NativeMedium, default_config_hash())?;
    let base = ImmutableRevisionStore::new(dir.path())
        .load_verified_revision_by_key(&a.revision_identity.expect("base digest").to_hex(), None)?
        .expect("verified retained base");
    assert!(hint.matches_revisions(&base.record.identity, &target));
    let mut incompatible = base.record.identity.clone();
    incompatible.projection_recipe_digest = HashDigest::new([0x19; 32]);
    let mut forged = hint.clone();
    forged.base_revision_identity = Some(incompatible.digest());
    assert!(!forged.matches_revisions(&incompatible, &target));
    let mut unknown = hint;
    unknown.result_revision_identity = None;
    let (b, index, stats) = build_and_publish(dir.path(), scene_id, Some(&unknown))?;
    assert!(!stats.used_incremental);
    assert_eq!(stats.fallback_reason, Some(FullRebuildReason::UnqualifiedChangeChain));
    assert_equivalent_to_full_build(dir.path(), scene_id, b.generation, &index)
}
