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
    use std::fs;

    use anyhow::Result;
    use tempfile::tempdir;
    use usd_model::HashDigest;
    use usd_project::{SceneId, SceneMember, SceneMemberId, SceneMemberTarget, ScenePlacementTransform};
    use viewport_protocol::RuntimeProfile;

    use super::*;
    use crate::project::{
        blob_store::{BlobStore, FilesystemBlobStore},
        cache::{
            ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore,
            ProjectCacheTarget, SceneCacheStore,
        },
        cache_contract::{
            CacheObjectRef, CachedTransform, ProjectCacheLookup, SCENE_CACHE_DESCRIPTOR_SCHEMA_VERSION,
            SCENE_CACHE_INDEX_SCHEMA_VERSION, SceneCacheAddress, SceneCacheDescriptorV3,
            SceneCacheEntry, SceneCacheEntryKind, SceneCacheIndex, SceneCacheOccurrence,
            SceneCacheState, SceneObjectKey, SceneSourceStamp,
        },
        cache_warm_runtime::{
            build_and_publish_scene_cache_generation, publish_project_cache_lookup,
        },
        storage::ProjectStorageLayout,
    };

    #[test]
    fn runtime_contract_changes_identity() {
        let semantic = HashDigest::new([7; HashDigest::BYTE_LEN]);
        assert_ne!(project_runtime_cache_config_hash(semantic), semantic);
        assert_eq!(PROJECT_RUNTIME_CACHE_COMPATIBILITY_VERSION, 2);
    }

    #[test]
    fn scene_v3_storage_isolates_scene_ids_and_identical_blob_hashes() -> Result<()> {
        let directory = tempdir()?;
        let layout = ProjectStorageLayout::new(directory.path());
        let store = SceneCacheStore::new(directory.path());
        let scene_a = SceneId::new_v4();
        let scene_b = SceneId::new_v4();
        let descriptor_a = scene_descriptor(scene_a, 4);
        let descriptor_b = scene_descriptor(scene_b, 9);

        let path_a = store.publish_descriptor(&descriptor_a)?;
        let path_b = store.publish_descriptor(&descriptor_b)?;
        assert_eq!(path_a, layout.scene_cache_descriptor_path(scene_a));
        assert_eq!(path_b, layout.scene_cache_descriptor_path(scene_b));
        assert_eq!(store.load_descriptor(scene_a)?, Some(descriptor_a));
        assert_eq!(store.load_descriptor(scene_b)?, Some(descriptor_b));

        let scene_a_objects = store.object_store(scene_a)?;
        let scene_b_objects = store.object_store(scene_b)?;
        let blob_a = scene_a_objects.put(b"same-payload")?;
        let blob_a_repeat = scene_a_objects.put(b"same-payload")?;
        let blob_b = scene_b_objects.put(b"same-payload")?;
        assert_eq!(blob_a, blob_a_repeat);
        assert_eq!(blob_a, blob_b);

        let scene_a_root = layout.scene_cache_objects_dir(scene_a);
        let scene_b_root = layout.scene_cache_objects_dir(scene_b);
        assert_ne!(scene_a_root, scene_b_root);
        assert!(scene_a_root.is_dir());
        assert!(scene_b_root.is_dir());

        fs::remove_dir_all(&scene_a_root)?;
        assert!(scene_a_objects.get(&blob_a)?.is_none());
        assert_eq!(
            scene_b_objects.get(&blob_b)?.as_deref(),
            Some(b"same-payload".as_slice())
        );
        Ok(())
    }

    #[test]
    fn legacy_global_cache_is_a_scene_v3_miss() -> Result<()> {
        let directory = tempdir()?;
        let layout = ProjectStorageLayout::new(directory.path());
        let scene_id = SceneId::new_v4();
        let legacy_identity = ProjectCacheIdentity {
            target: ProjectCacheTarget::Scene {
                id: scene_id.to_string(),
            },
            target_content_hash: HashDigest::new([1; HashDigest::BYTE_LEN]),
            profile: RuntimeProfile::NativeMedium,
            config_hash: HashDigest::new([2; HashDigest::BYTE_LEN]),
        };
        ProjectCacheStore::new(directory.path()).publish(&ProjectCacheDescriptor::new(
            legacy_identity,
            ProjectCacheState::Partial,
            None,
        )?)?;

        let legacy_objects = FilesystemBlobStore::new(layout.cache_objects_dir())?;
        let legacy_blob = legacy_objects.put(b"legacy-global-payload")?;
        let scene_store = SceneCacheStore::new(directory.path());

        assert!(scene_store.load_descriptor(scene_id)?.is_none());
        assert!(scene_store.object_store(scene_id)?.get(&legacy_blob)?.is_none());
        assert_eq!(
            legacy_objects.get(&legacy_blob)?.as_deref(),
            Some(b"legacy-global-payload".as_slice())
        );
        assert!(!layout.scene_cache_dir(scene_id).exists());
        assert!(!layout.scene_cache_objects_dir(scene_id).exists());
        Ok(())
    }

    #[test]
    fn scene_owned_builder_prunes_child_and_publishes_payload_spatial_deterministically() -> Result<()> {
        use openusd::{gf::{Vec3d, Vec3f}, sdf::Value};
        use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneManifestEntry, StorageKey};

        let directory = tempdir()?;
        let parent_scene = SceneId::new_v4();
        let child_scene = SceneId::new_v4();
        let member_id = SceneMemberId::new_v4();

        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(), "Cache Builder", ProjectRoot::Scene(parent_scene),
            vec![
                SceneManifestEntry { id: parent_scene, storage_key: StorageKey::new("Parent")?, display_name: "Parent".to_owned() },
                SceneManifestEntry { id: child_scene, storage_key: StorageKey::new("Child")?, display_name: "Child".to_owned() },
            ],
            Vec::new(),
        );
        crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;

        crate::project::scene::authoring::author_scene_atomic(directory.path(), child_scene)?;
        let child_path = crate::project::scene::authoring::scene_path(directory.path(), child_scene);
        let child_stage = openusd::usd::Stage::open(child_path.to_string_lossy().as_ref())?;
        for index in 0..128 {
            child_stage.define_prim(format!("/SceneRoot/Heavy/Node{index:03}").as_str())?.set_type_name("Xform")?;
        }
        child_stage.root_layer().export(child_path.to_string_lossy().as_ref())?;
        crate::project::scene::authoring::author_scene_atomic_with_members(
            directory.path(),
            parent_scene,
            &[SceneMember {
                id: member_id,
                target: SceneMemberTarget::Scene(child_scene),
                name: Some("Child".to_owned()),
                transform: ScenePlacementTransform::from_translation([1.0, 2.0, 3.0]),
            }],
        )?;
        let parent_path = crate::project::scene::authoring::scene_path(directory.path(), parent_scene);
        let updated_parent_path = parent_path.with_file_name("Parent.updated.usda");
        {
            let parent_stage = openusd::usd::Stage::open(parent_path.to_string_lossy().as_ref())?;
            let mesh = parent_stage.define_prim("/SceneRoot/Owned")?.set_type_name("Mesh")?;
            mesh.create_attribute("points", "point3f[]")?.set(Value::Vec3fVec(vec![
                Vec3f::from([0.0, 0.0, 0.0]), Vec3f::from([1.0, 0.0, 0.0]), Vec3f::from([0.0, 1.0, 0.0]),
            ]))?;
            mesh.create_attribute("faceVertexCounts", "int[]")?.set(Value::IntVec(vec![3]))?;
            mesh.create_attribute("faceVertexIndices", "int[]")?.set(Value::IntVec(vec![0, 1, 2]))?;
            mesh.create_attribute("xformOp:translate", "double3")?.set(Value::Vec3d(Vec3d::from([1.0, 2.0, 3.0])))?;
            mesh.create_attribute("xformOpOrder", "token[]")?.set(Value::TokenVec(vec!["xformOp:translate".into()]))?;
            parent_stage.root_layer().export(updated_parent_path.to_string_lossy().as_ref())?;
        }
        fs::rename(&updated_parent_path, &parent_path)?;
        assert!(parent_path.exists());

        let descriptor = scene_descriptor(parent_scene, 12);
        let first = build_and_publish_scene_cache_generation(directory.path(), &descriptor)?;
        let (second, _) = crate::project::cache_warm_runtime::build_scene_cache_index(directory.path(), parent_scene, 12)?;
        assert_eq!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
        assert!(first.entries.len() < 16, "child descendants must be pruned, not composed into parent rows");
        let member_root = crate::project::scene::authoring::scene_member_path(member_id);
        assert!(!first.entries.iter().any(|entry| matches!(
            entry.kind, SceneCacheEntryKind::OwnedPrim { ref prim_path } if prim_path.starts_with(&member_root)
        )));
        let owned = first.entries.iter().find(|entry| matches!(
            entry.kind, SceneCacheEntryKind::OwnedPrim { ref prim_path } if prim_path == "/SceneRoot/Owned"
        )).expect("owned mesh entry");
        assert!(owned.geometry.is_some());
        assert!(owned.content_hash.is_some());
        assert_eq!(owned.bounds.expect("owned bounds").max, [1.0, 1.0, 0.0]);
        assert!(matches!(&owned.transform, CachedTransform::Prim(transform) if transform.translation_mm == [1000, 2000, 3000]));
        let geometry = owned.geometry.clone().expect("geometry payload");
        assert!(SceneCacheStore::new(directory.path()).object_store(parent_scene)?.contains(&geometry.blob_id)?);

        let store = SceneCacheStore::new(directory.path());
        let spatial = store.load_spatial(parent_scene)?.expect("spatial index");
        assert!(spatial.entries.iter().any(|entry| entry.address == owned.address));
        let descriptor = store.load_descriptor(parent_scene)?.expect("published descriptor");
        let layout = ProjectStorageLayout::new(directory.path());
        let spatial_bytes = fs::read(layout.scene_cache_spatial_path(parent_scene))?;
        assert_eq!(descriptor.spatial_digest, Some(HashDigest::new(*blake3::hash(&spatial_bytes).as_bytes())));

        let lookup = ProjectCacheLookup::from_scene_indexes(&[first.clone()])?;
        let member_address = SceneCacheAddress { scene_id: parent_scene, occurrence: SceneCacheOccurrence::Member(member_id) };
        assert_eq!(lookup.get(&member_address), Some(&CacheObjectRef::ChildScene { scene_id: child_scene, member_id }));
        assert_eq!(
            lookup.addresses_for_hash(SceneObjectKey { scene_id: parent_scene, content_hash: owned.content_hash.unwrap() }),
            std::slice::from_ref(&owned.address),
        );
        let first_bytes = publish_project_cache_lookup(directory.path(), &lookup)?;
        let second_bytes = publish_project_cache_lookup(directory.path(), &lookup)?;
        assert_eq!(first_bytes, second_bytes);
        Ok(())
    }

    #[test]
    fn project_cache_lookup_rejects_duplicate_scene_generations() -> Result<()> {
        let scene_id = SceneId::new_v4();
        let other_scene = SceneId::new_v4();
        let make_index = |generation, path: &str| SceneCacheIndex {
            schema_version: SCENE_CACHE_INDEX_SCHEMA_VERSION,
            scene_id,
            generation,
            entries: vec![SceneCacheEntry {
                address: SceneCacheAddress { scene_id, occurrence: SceneCacheOccurrence::PrimPath(path.to_owned()) },
                parent: None,
                transform: CachedTransform::Placement(ScenePlacementTransform::IDENTITY),
                bounds: None,
                cacheable: false,
                bim_enabled: false,
                geometry: None,
                material: None,
                animation: None,
                semantic_key: None,
                kind: SceneCacheEntryKind::OwnedPrim { prim_path: path.to_owned() },
                content_hash: None,
            }],
        };
        assert!(ProjectCacheLookup::from_scene_indexes(&[make_index(1, "/SceneRoot/A"), make_index(2, "/SceneRoot/B")]).is_err());
        let mut foreign = make_index(3, "/SceneRoot/C");
        foreign.entries[0].address.scene_id = other_scene;
        assert!(foreign.validate(scene_id, 3).is_err());
        Ok(())
    }

    #[test]
    fn scene_descriptor_index_or_spatial_mismatch_is_rejected() -> Result<()> {
        use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneManifestEntry, StorageKey};
        let directory = tempdir()?;
        let scene_id = SceneId::new_v4();
        let manifest = ProjectManifestV1::new(
            ProjectId::new_v4(), "Atomic Cache", ProjectRoot::Scene(scene_id),
            vec![SceneManifestEntry { id: scene_id, storage_key: StorageKey::new("Scene")?, display_name: "Scene".to_owned() }],
            Vec::new(),
        );
        crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(directory.path(), &manifest)?;
        crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;
        let descriptor = scene_descriptor(scene_id, 3);
        build_and_publish_scene_cache_generation(directory.path(), &descriptor)?;
        let layout = ProjectStorageLayout::new(directory.path());
        let store = SceneCacheStore::new(directory.path());
        fs::write(layout.scene_cache_index_path(scene_id), b"stale-index-generation")?;
        assert!(store.load_index(scene_id)?.is_none());
        build_and_publish_scene_cache_generation(directory.path(), &descriptor)?;
        fs::write(layout.scene_cache_spatial_path(scene_id), b"stale-spatial-generation")?;
        assert!(store.load_spatial(scene_id)?.is_none());
        Ok(())
    }

    fn scene_descriptor(scene_id: SceneId, generation: u64) -> SceneCacheDescriptorV3 {
        SceneCacheDescriptorV3 {
            schema_version: SCENE_CACHE_DESCRIPTOR_SCHEMA_VERSION,
            scene_id,
            generation,
            source_stamp: SceneSourceStamp::ManagedGeneration { generation },
            source_content_hash: None,
            config_hash: HashDigest::new([3; HashDigest::BYTE_LEN]),
            state: SceneCacheState::Partial,
            prim_count: 8,
            cacheable_count: 5,
            estimated_cpu_bytes: 1_024,
            estimated_gpu_bytes: 2_048,
            index_digest: HashDigest::new([4; HashDigest::BYTE_LEN]),
            spatial_digest: None,
        }
    }
}
