use anyhow::{Context, Result};
use openusd::usd::Stage;
use tempfile::tempdir;
use usd_model::BlobId;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneId};

use super::preparation::wait_for;
use super::*;
use crate::project::blob_store::{BlobStore, FilesystemBlobStore};

#[test]
fn migrated_project_root_cache_contains_legacy_composed_content() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let legacy_manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Cached Legacy Project",
        ProjectRoot::Scene(scene_id),
        vec![usd_project::SceneManifestEntry {
            id: scene_id,
            storage_key: usd_project::StorageKey::new("Legacy Scene")?,
        }],
        Vec::new(),
    );
    let legacy_path = crate::project::scene::authoring::scene_path(directory.path(), scene_id);
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;
    let legacy_stage = Stage::open(legacy_path.to_string_lossy().as_ref())?;
    let content = legacy_stage
        .define_prim("/SceneRoot/LegacyCached/Content")?
        .set_type_name("Mesh")?;
    content
        .create_attribute("points", "point3f[]")?
        .set(openusd::sdf::Value::Vec3fVec(vec![
            openusd::gf::Vec3f::from([0.0, 0.0, 0.0]),
            openusd::gf::Vec3f::from([1.0, 0.0, 0.0]),
            openusd::gf::Vec3f::from([0.0, 1.0, 0.0]),
        ]))?;
    content
        .create_attribute("faceVertexCounts", "int[]")?
        .set(openusd::sdf::Value::IntVec(vec![3]))?;
    content
        .create_attribute("faceVertexIndices", "int[]")?
        .set(openusd::sdf::Value::IntVec(vec![0, 1, 2]))?;
    legacy_stage.set_default_prim("SceneRoot")?;
    legacy_stage
        .root_layer()
        .export(legacy_path.to_string_lossy().as_ref())?;

    let migrated = crate::project::scene::root::ensure_protected_root_scene_atomic(
        directory.path(),
        &legacy_manifest,
    )?;
    let ProjectRoot::Scene(root_scene_id) = migrated.root else {
        anyhow::bail!("legacy Project must receive a protected Root Scene");
    };
    assert_ne!(root_scene_id, scene_id);
    let root_path = crate::project::scene::authoring::scene_path(directory.path(), root_scene_id);
    let root_stage = Stage::open(root_path.to_string_lossy().as_ref())?;
    let member = crate::project::scene::authoring::read_scene_members(&root_path, root_scene_id)?
        .into_iter()
        .find(|member| member.target == usd_project::SceneMemberTarget::Scene(scene_id))
        .context("migrated legacy Scene placement")?;
    let member_path = crate::project::scene::authoring::scene_member_path(member.id);
    assert!(
        root_stage
            .prim(member_path.as_str())
            .child_names()?
            .iter()
            .any(|name| name.as_str() == "LegacyCached"),
        "migrated root Stage must expose the legacy descendant"
    );

    let queue = ProjectCacheWarmQueue::default();
    let target = ProjectCacheTarget::ProjectRoot;
    assert!(queue.enqueue(directory.path(), target.clone()));
    let descriptor = wait_for(&queue, directory.path(), &target)?.context("cache warm result")?;
    assert_eq!(descriptor.state, ProjectCacheState::Ready);
    let runtime = descriptor.runtime.context("Ready runtime manifest")?;
    let store = FilesystemBlobStore::new(
        directory
            .path()
            .join(crate::project::storage::CACHE_OBJECTS_RELATIVE_PATH),
    )?;
    let hierarchy_bytes = store
        .get(&BlobId(runtime.hierarchy.blob_id.clone()))?
        .context("warmed hierarchy blob")?;
    let hierarchy: crate::project::runtime_delivery::RuntimeHierarchyBlob =
        serde_json::from_slice(&hierarchy_bytes)?;
    assert!(
        hierarchy
            .entities
            .iter()
            .any(|entity| entity.prim_path == member_path)
    );
    Ok(())
}
