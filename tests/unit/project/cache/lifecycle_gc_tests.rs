use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use openusd::{sdf, vt};
use tempfile::tempdir;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, SceneMember,
    SceneMemberId, SceneMemberTarget, StorageKey,
};
use viewport_protocol::RuntimeProfile;

use crate::cache::{
    ProjectCacheTarget, SceneCacheDescriptorV3, SceneCacheState, SceneCacheStore,
    blobs::BlobStore,
    gc::{GcPolicy, collect_garbage, safe_remove_cache_file},
    generic::{
        CacheIdentity, StandaloneCacheStore, compute_standalone_source_identity,
        default_standalone_recipe_digest,
    },
    hydration::default_project_cache_config_hash,
    identity::compute_scene_revision_identity,
    retention::{CachePinGuard, ImmutableRevisionStore},
    sync::{CacheLifecycleState, PendingIntentStore, TombstoneStore},
    warming::{
        ProjectCacheWarmQueue, expand_affected_targets,
        runtime::{
            build_and_publish_managed_scene_cache_generation,
            build_and_publish_scene_cache_generation,
        },
    },
};
use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::authoring::{author_scene_atomic, author_scene_atomic_with_members, scene_path},
    service::ProjectApplicationService,
    storage::ProjectStorageLayout,
};

fn create_mesh_scene_project(name: &str) -> Result<(tempfile::TempDir, PathBuf, SceneId)> {
    let dir = tempdir()?;
    let root = dir.path().join("project");
    fs::create_dir_all(&root)?;
    usd_git::Repository::init(&root)?;
    crate::project::storage::install_managed_ignore(&root)?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        name,
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("root-scene")?,
            display_name: "Root Scene".to_owned(),
        }],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&root, &manifest)?;
    let path = author_scene_atomic(&root, scene_id)?;
    write_triangle_mesh_to_scene(&path, [0.0, 0.0, 0.0])?;
    Ok((dir, root, scene_id))
}

fn write_triangle_mesh_to_scene(scene_file: &Path, offset: [f32; 3]) -> Result<()> {
    let stage = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(scene_file.to_string_lossy().as_ref())?;
    let mesh = stage
        .define_prim("/SceneRoot/Triangle")?
        .set_type_name("Mesh")?;
    mesh.create_attribute("points", "point3f[]")?
        .set(vt::Value::Vec3fVec(vec![
            openusd::gf::Vec3f::from(offset),
            openusd::gf::Vec3f::from([offset[0] + 1.0, offset[1], offset[2]]),
            openusd::gf::Vec3f::from([offset[0], offset[1] + 1.0, offset[2]]),
        ]))?;
    mesh.create_attribute("faceVertexCounts", "int[]")?
        .set(vt::Value::IntVec(vec![3]))?;
    mesh.create_attribute("faceVertexIndices", "int[]")?
        .set(vt::Value::IntVec(vec![0, 1, 2]))?;
    let tmp = scene_file.with_extension("edit.tmp.usda");
    stage
        .root_layer()
        .export(tmp.to_string_lossy().as_ref())?;
    fs::rename(&tmp, scene_file)?;
    Ok(())
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn project_creation_proactively_warms_root_scene_without_stage_activation() -> Result<()> {
    let dir = tempdir()?;
    let parent = dir.path().join("projects");
    fs::create_dir_all(&parent)?;
    let registry_path = dir.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path)?;
    let _summary = service.create_project(&parent, "Proactive Project")?;
    let project_root = parent.join("Proactive Project");
    let manifest = ManifestStore::read_validated(&project_root)?;
    let ProjectRoot::Scene(root_scene) = manifest.raw().root else {
        panic!("created Project must have a protected root Scene");
    };

    assert!(
        service.wait_for_cache_idle(&project_root),
        "proactive Project creation warm queue must drain cleanly"
    );
    let store = SceneCacheStore::new(&project_root);
    let descriptor = store
        .load_descriptor(root_scene)?
        .expect("root Scene descriptor must be proactively published on Project creation");
    assert!(matches!(
        descriptor.state,
        SceneCacheState::Ready | SceneCacheState::Partial
    ));
    assert!(descriptor.revision_identity.is_some());
    Ok(())
}

#[test]
fn nested_scene_mutation_invalidates_transitive_ancestors_and_project_root() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path().join("project");
    fs::create_dir_all(&root)?;
    usd_git::Repository::init(&root)?;

    let grandparent = SceneId::new_v4();
    let parent = SceneId::new_v4();
    let child = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Nested Hierarchy",
        ProjectRoot::Scene(grandparent),
        vec![
            SceneManifestEntry {
                id: grandparent,
                storage_key: StorageKey::new("grandparent")?,
                display_name: "Grandparent".to_owned(),
            },
            SceneManifestEntry {
                id: parent,
                storage_key: StorageKey::new("parent")?,
                display_name: "Parent".to_owned(),
            },
            SceneManifestEntry {
                id: child,
                storage_key: StorageKey::new("child")?,
                display_name: "Child".to_owned(),
            },
        ],
        Vec::new(),
    );
    ManifestStore::write_manifest_atomic(&root, &manifest)?;
    author_scene_atomic(&root, child)?;
    author_scene_atomic_with_members(
        &root,
        parent,
        &[SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Scene(child),
            name: Some("Child".to_owned()),
            transform: Default::default(),
        }],
    )?;
    author_scene_atomic_with_members(
        &root,
        grandparent,
        &[SceneMember {
            id: SceneMemberId::new_v4(),
            target: SceneMemberTarget::Scene(parent),
            name: Some("Parent".to_owned()),
            transform: Default::default(),
        }],
    )?;

    let expanded = expand_affected_targets(
        &root,
        vec![ProjectCacheTarget::Scene {
            id: child.to_string(),
        }],
    );
    assert!(expanded.contains(&ProjectCacheTarget::Scene {
        id: child.to_string()
    }));
    assert!(expanded.contains(&ProjectCacheTarget::Scene {
        id: parent.to_string()
    }));
    assert!(expanded.contains(&ProjectCacheTarget::Scene {
        id: grandparent.to_string()
    }));
    assert!(expanded.contains(&ProjectCacheTarget::ProjectRoot));
    Ok(())
}

#[test]
fn strong_revision_identity_tracks_bytes_variants_and_not_git_sha_alone() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Identity Check")?;
    run_git(&root, &["config", "user.name", "Test"]);
    run_git(&root, &["config", "user.email", "test@example.invalid"]);
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "baseline"]);

    let config = default_project_cache_config_hash();
    let baseline =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    let repeated =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    assert_eq!(baseline, repeated);
    assert_eq!(baseline.digest(), repeated.digest());

    // Edit canonical bytes WITHOUT creating a new Git commit (HEAD SHA unchanged).
    let path = scene_path(&root, scene_id);
    write_triangle_mesh_to_scene(&path, [5.0, 0.0, 0.0])?;
    let modified =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    assert_ne!(
        baseline.source_closure_digest, modified.source_closure_digest,
        "strong source validation must detect byte changes even when Git HEAD is unchanged"
    );
    assert_ne!(baseline.digest(), modified.digest());

    // Add an authored variant selection on a child prim inside SceneRoot.
    let mut raw = fs::read_to_string(&path)?;
    let insert_pos = raw.rfind('}').expect("SceneRoot closing brace");
    raw.insert_str(
        insert_pos,
        "\n    def Xform \"VariantScope\" (\n        variants = {\n            string lod = \"high\"\n        }\n        prepend variantSets = \"lod\"\n    )\n    {\n        variantSet \"lod\" = {\n            \"high\" {}\n            \"low\" {}\n        }\n    }\n",
    );
    fs::write(&path, raw)?;
    let variant_high =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    assert_ne!(
        modified.composition_context_digest,
        variant_high.composition_context_digest
    );
    assert_ne!(modified.digest(), variant_high.digest());
    Ok(())
}

#[test]
fn standalone_cache_identity_tracks_variants_and_supports_cache_disabled_fallback() -> Result<()> {
    let dir = tempdir()?;
    let usd_path = dir.path().join("standalone.usda");
    fs::write(
        &usd_path,
        r#"#usda 1.0
(
    defaultPrim = "World"
)
def Xform "World" (
    variants = {
        string quality = "high"
    }
    prepend variantSets = "quality"
)
{
    variantSet "quality" = {
        "high" {}
        "low" {}
    }
}
"#,
    )?;
    let id_high = CacheIdentity::new(
        compute_standalone_source_identity(&usd_path)?,
        default_standalone_recipe_digest(),
    );

    let store = StandaloneCacheStore::new(dir.path().join("standalone-cache"));
    assert_eq!(store.inspect_state(&id_high), CacheLifecycleState::Missing);

    let mut mesh = bevy::mesh::Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        bevy::mesh::Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(bevy::mesh::Indices::U32(vec![0, 1, 2]));
    let _handle = store.publish_meshes(&id_high, &[("/World/Box".to_owned(), &mesh)])?;
    assert_eq!(store.inspect_state(&id_high), CacheLifecycleState::Ready);

    // Corrupt descriptor -> fails closed to Failed state, canonical USD still opens cleanly.
    fs::write(store.descriptor_path(&id_high), b"{corrupt")?;
    assert_eq!(store.inspect_state(&id_high), CacheLifecycleState::Failed);
    assert!(store.hydrate_meshes(&id_high)?.is_none());
    let canonical_stage = openusd::usd::Stage::open(usd_path.to_string_lossy().as_ref())?;
    assert!(canonical_stage.prim(sdf::path("/World")?)?.is_defined()?);

    // Changing variant selection produces a distinct identity.
    let low_text = fs::read_to_string(&usd_path)?.replace("\"high\"", "\"low\"");
    fs::write(&usd_path, low_text)?;
    let id_low = CacheIdentity::new(
        compute_standalone_source_identity(&usd_path)?,
        default_standalone_recipe_digest(),
    );
    assert_ne!(id_high.storage_key(), id_low.storage_key());
    assert_eq!(store.inspect_state(&id_low), CacheLifecycleState::Missing);

    // Retiring removes descriptor cleanly.
    store.retire(&id_high)?;
    assert_eq!(store.inspect_state(&id_high), CacheLifecycleState::Missing);
    Ok(())
}

#[test]
fn lifecycle_states_cover_missing_building_ready_stale_failed_and_retired() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Lifecycle Six States")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let source_hash = crate::project::assets::closure::target_content_hash(&root, &target)?;

    // 1. Missing
    assert_eq!(
        store.lifecycle_state(scene_id, Some(source_hash), Some(config))?,
        CacheLifecycleState::Missing
    );

    // 2. Building
    let generation = store.advance_generation(scene_id, config)?;
    assert_eq!(
        store.lifecycle_state(scene_id, Some(source_hash), Some(config))?,
        CacheLifecycleState::Building
    );

    // 3. Ready
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, generation, config);
    descriptor.state = SceneCacheState::Ready;
    descriptor.source_content_hash = Some(source_hash);
    build_and_publish_managed_scene_cache_generation(&root, &descriptor)?
        .expect("managed Scene build publishes Ready");
    assert_eq!(
        store.lifecycle_state(scene_id, Some(source_hash), Some(config))?,
        CacheLifecycleState::Ready
    );

    // 4. Stale (external source change)
    write_triangle_mesh_to_scene(&scene_path(&root, scene_id), [9.0, 0.0, 0.0])?;
    let updated_source_hash = crate::project::assets::closure::target_content_hash(&root, &target)?;
    assert_eq!(
        store.lifecycle_state(scene_id, Some(updated_source_hash), Some(config))?,
        CacheLifecycleState::Stale
    );

    // 5. Failed (corrupt descriptor)
    let layout = ProjectStorageLayout::new(&root);
    fs::write(layout.scene_cache_descriptor_path(scene_id), b"corrupt-json")?;
    assert_eq!(
        store.lifecycle_state(scene_id, Some(updated_source_hash), Some(config))?,
        CacheLifecycleState::Failed
    );

    // 6. Retired (tombstoned)
    store.retire_scene(scene_id)?;
    assert_eq!(
        store.lifecycle_state(scene_id, Some(updated_source_hash), Some(config))?,
        CacheLifecycleState::Retired
    );
    Ok(())
}

#[test]
fn unavailable_queue_persists_pending_intent_and_recovers_on_restart() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Pending Intent Recovery")?;
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };

    // Simulate an unavailable warm queue during canonical publication.
    let offline_queue = ProjectCacheWarmQueue::default();
    offline_queue.shutdown_without_waiting();
    assert!(!offline_queue.enqueue_targets_for_mutation(&root, vec![target.clone()])?);

    // Durable pending intent must be persisted on disk.
    let pending_store = PendingIntentStore::new(&root);
    let intent = pending_store
        .load(&target)?
        .expect("durable pending intent must survive unavailable queue");
    assert_eq!(intent.generation, 1);

    // A fresh queue recovers and builds the pending intent to Ready, then acknowledges it.
    let recovered_queue = ProjectCacheWarmQueue::default();
    assert_eq!(recovered_queue.recover_pending_intents(&root), 1);
    assert!(recovered_queue.wait_for_project_idle(&root, Duration::from_secs(5)));

    let store = SceneCacheStore::new(&root);
    let published = store
        .load_descriptor(scene_id)?
        .expect("recovered Scene descriptor must be published");
    assert!(matches!(
        published.state,
        SceneCacheState::Ready | SceneCacheState::Partial
    ));
    assert!(
        pending_store.load(&target)?.is_none(),
        "pending intent must be acknowledged after successful publication"
    );
    Ok(())
}

#[test]
fn missing_blob_cannot_publish_pointer_and_interrupted_temp_is_ignored() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Missing Blob Gate")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let generation = store.advance_generation(scene_id, config)?;

    let (index, spatial) =
        crate::cache::warming::runtime::build_scene_cache_index(&root, scene_id, generation)?;
    let geom_blob = index
        .entries
        .iter()
        .find_map(|entry| entry.geometry.as_ref().map(|g| g.blob_id.clone()))
        .expect("built Scene index must reference a geometry blob");

    // Delete the referenced geometry blob before pointer publication.
    let layout = ProjectStorageLayout::new(&root);
    let blob_path = layout
        .scene_cache_objects_dir(scene_id)
        .join(&geom_blob.0[..2])
        .join(format!("{}.blob", geom_blob.0));
    fs::remove_file(&blob_path)?;

    // Leave an interrupted temporary file in the Scene cache directory.
    let interrupted_tmp = layout.scene_cache_dir(scene_id).join(".descriptor.interrupted.tmp");
    fs::write(&interrupted_tmp, b"{partial-write")?;

    let mut ready = SceneCacheDescriptorV3::invalidated(scene_id, generation, config);
    ready.state = SceneCacheState::Ready;
    let identity = compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    ready.source_content_hash = Some(identity.source_closure_digest);
    ready.revision_identity = Some(identity.digest());
    let publish_result = store.publish_generation(&ready, &index, &spatial);
    assert!(
        publish_result.is_err(),
        "store must refuse to publish a Ready pointer when a referenced blob is missing"
    );
    let current = store
        .load_descriptor(scene_id)?
        .expect("previous Building descriptor must remain intact");
    assert_eq!(current.state, SceneCacheState::Building);
    Ok(())
}

#[test]
fn managed_ready_requires_a_durable_desired_record() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Required Desired Record")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let target = ProjectCacheTarget::Scene { id: scene_id.to_string() };
    let pending = PendingIntentStore::new(&root);
    let generation = store.advance_generation(scene_id, config)?;
    let identity = compute_scene_revision_identity(
        &root, scene_id, RuntimeProfile::NativeMedium, config,
    )?;
    let (index, spatial) =
        crate::cache::warming::runtime::build_scene_cache_index(&root, scene_id, generation)?;
    let mut ready = SceneCacheDescriptorV3::invalidated(scene_id, generation, config);
    ready.state = SceneCacheState::Ready;
    ready.source_content_hash = Some(identity.source_closure_digest);
    ready.revision_identity = Some(identity.digest());
    store.verify_index_blobs(scene_id, &index)?;
    let before = store.load_descriptor(scene_id)?.context("Building pointer")?;

    pending.remove(&target)?;
    assert!(store.publish_descriptor(&ready).is_err());
    assert!(store.publish_generation(&ready, &index, &spatial).is_err());
    assert!(build_and_publish_managed_scene_cache_generation(&root, &ready)?.is_none());
    assert_eq!(store.load_descriptor(scene_id)?, Some(before));
    assert!(pending.load_record(&target)?.is_none());

    let next = store.advance_generation(scene_id, config)?;
    assert!(next > generation);
    let mut ready = SceneCacheDescriptorV3::invalidated(scene_id, next, config);
    ready.state = SceneCacheState::Ready;
    assert!(build_and_publish_managed_scene_cache_generation(&root, &ready)?.is_some());
    let committed = store.load_descriptor(scene_id)?.context("Ready pointer")?;
    let record = pending.load_record(&target)?.context("acknowledged desired record")?;
    assert!(!record.pending);
    assert_eq!(record.generation, next);
    assert_eq!(record.desired_revision_identity, committed.revision_identity);
    assert!(store.load_activation(scene_id)?.is_some());
    assert!(store.load_activation_for_repair(scene_id)?.is_some());

    // A corrupt/lost authorization record must not turn a valid payload
    // into an authorized current generation merely because bytes still match.
    pending.remove(&target)?;
    assert!(store.load_activation(scene_id)?.is_none());
    assert!(store.load_activation_for_repair(scene_id)?.is_none());
    assert_eq!(store.lifecycle_state(scene_id, None, None)?, CacheLifecycleState::Stale);
    assert_eq!(store.load_descriptor(scene_id)?, Some(committed));

    let queue = ProjectCacheWarmQueue::default();
    assert!(queue.recover_pending_intents(&root) > 0);
    assert!(queue.wait_for_project_idle(&root, Duration::from_secs(15)));
    let recovered = pending.load_record(&target)?.context("recovered desired record")?;
    assert!(recovered.generation > next);
    assert!(!recovered.pending);
    assert!(store.load_activation(scene_id)?.is_some());
    Ok(())
}

#[test]
fn external_modification_and_concurrent_save_reject_stale_build() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Concurrent Modification")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let gen1 = store.advance_generation(scene_id, config)?;
    let original_hash = crate::project::assets::closure::target_content_hash(&root, &target)?;

    // Externally modify canonical USD before the worker builds against original_hash.
    write_triangle_mesh_to_scene(&scene_path(&root, scene_id), [3.0, 4.0, 5.0])?;
    let mut stale_desc = SceneCacheDescriptorV3::invalidated(scene_id, gen1, config);
    stale_desc.state = SceneCacheState::Ready;
    stale_desc.source_content_hash = Some(original_hash);
    assert!(
        build_and_publish_managed_scene_cache_generation(&root, &stale_desc).is_err(),
        "external modification during build must fail validation and refuse publication"
    );

    // Concurrent save advances generation to gen2 -> gen1 worker is rejected as Stale (Ok(None)).
    let gen2 = store.advance_generation(scene_id, config)?;
    assert_eq!(gen2, gen1 + 1);
    let updated_hash = crate::project::assets::closure::target_content_hash(&root, &target)?;
    stale_desc.source_content_hash = Some(updated_hash);
    assert!(
        build_and_publish_managed_scene_cache_generation(&root, &stale_desc)?.is_none(),
        "superseded generation worker must be rejected by CAS"
    );
    assert_eq!(
        store
            .load_descriptor(scene_id)?
            .expect("gen2 remains current")
            .generation,
        gen2
    );
    Ok(())
}

#[test]
fn immutable_revision_retention_reuses_shared_blobs_across_branches() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Branch Retention")?;
    run_git(&root, &["config", "core.autocrlf", "false"]);
    run_git(&root, &["config", "user.name", "Test"]);
    run_git(&root, &["config", "user.email", "test@example.invalid"]);
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "branch A"]);
    let branch_a = String::from_utf8(
        std::process::Command::new("git")
            .current_dir(&root)
            .args(["branch", "--show-current"])
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();

    let config = default_project_cache_config_hash();
    let store = SceneCacheStore::new(&root);
    let revision_store = ImmutableRevisionStore::new(&root);
    let identity_a =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;

    // Build and publish branch A revision.
    let gen_a = store.advance_generation(scene_id, config)?;
    let mut desc_a = SceneCacheDescriptorV3::invalidated(scene_id, gen_a, config);
    desc_a.state = SceneCacheState::Ready;
    desc_a.source_content_hash = Some(identity_a.source_closure_digest);
    build_and_publish_managed_scene_cache_generation(&root, &desc_a)?
        .expect("branch A build publishes");

    let retained_a = revision_store
        .load_verified_revision(&identity_a)?
        .expect("branch A immutable revision must be retained");
    assert!(!retained_a.record.referenced_blobs.is_empty());
    let immutable_a = revision_store.revision_dir(&identity_a.storage_key());
    let original_files = ["record.json", "index.bin", "spatial.bin"]
        .map(|name| fs::read(immutable_a.join(name)));
    let [record_bytes, index_bytes, spatial_bytes] = original_files;
    let original_files = [record_bytes?, index_bytes?, spatial_bytes?];

    // Create branch B with modified geometry, commit, and build.
    run_git(&root, &["checkout", "-b", "branch-b"]);
    write_triangle_mesh_to_scene(&scene_path(&root, scene_id), [10.0, 20.0, 30.0])?;
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "branch B"]);

    let identity_b =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    assert_ne!(identity_a.digest(), identity_b.digest());

    let gen_b = store.advance_generation(scene_id, config)?;
    let mut desc_b = SceneCacheDescriptorV3::invalidated(scene_id, gen_b, config);
    desc_b.state = SceneCacheState::Ready;
    desc_b.source_content_hash = Some(identity_b.source_closure_digest);
    build_and_publish_managed_scene_cache_generation(&root, &desc_b)?
        .expect("branch B build publishes");

    // Switch back to branch A and prove immutable revision reuse without re-encoding.
    run_git(&root, &["checkout", &branch_a]);
    let gen_a2 = store.advance_generation(scene_id, config)?;
    let mut desc_a2 = SceneCacheDescriptorV3::invalidated(scene_id, gen_a2, config);
    desc_a2.state = SceneCacheState::Ready;
    desc_a2.source_content_hash = Some(identity_a.source_closure_digest);
    let promoted = build_and_publish_managed_scene_cache_generation(&root, &desc_a2)?
        .expect("returning to branch A must reuse retained immutable revision");
    assert_eq!(promoted.generation, gen_a2);
    for (name, original) in ["record.json", "index.bin", "spatial.bin"].into_iter().zip(original_files) {
        assert_eq!(fs::read(immutable_a.join(name))?, original, "retained revision must not be rewritten");
    }

    // Both branch A and branch B immutable revisions remain retained in the store.
    assert!(revision_store.load_verified_revision(&identity_a)?.is_some());
    assert!(revision_store.load_verified_revision(&identity_b)?.is_some());
    Ok(())
}

#[test]
fn deletion_during_in_flight_build_tombstones_target_and_prevents_resurrection() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Tombstone Anti-Resurrection")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };

    let gen1 = store.advance_generation(scene_id, config)?;
    let expected = store
        .load_descriptor(scene_id)?
        .expect("Building descriptor exists");
    let (index, spatial) =
        crate::cache::warming::runtime::build_scene_cache_index(&root, scene_id, gen1)?;

    // Target is deleted while the worker is mid-build.
    let retired_gen = store.retire_scene(scene_id)?;
    assert!(retired_gen > gen1);
    assert!(TombstoneStore::new(&root).is_retired_at(&target, gen1)?);
    assert!(store.load_descriptor(scene_id)?.is_none());

    // Worker finishes and attempts CAS publication -> MUST be rejected (zero resurrection).
    let mut ready = expected.clone();
    ready.state = SceneCacheState::Ready;
    let outcome = store.publish_generation_if_current(&expected, &ready, &index, &spatial)?;
    assert!(
        outcome.is_none(),
        "in-flight build for deleted target must be rejected by tombstone CAS"
    );
    assert!(
        store.load_descriptor(scene_id)?.is_none(),
        "deleted Scene descriptor must not be resurrected"
    );
    assert_eq!(
        store.lifecycle_state(scene_id, None, None)?,
        CacheLifecycleState::Retired
    );
    Ok(())
}

#[test]
fn gc_collects_unreferenced_artifacts_respects_pins_and_protects_canonical_files() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("GC Retention And Pins")?;
    let canonical_scene = scene_path(&root, scene_id);
    let canonical_manifest = ProjectStorageLayout::new(&root).canonical_manifest_path();
    let original_scene_bytes = fs::read(&canonical_scene)?;
    let original_manifest_bytes = fs::read(&canonical_manifest)?;

    let store = SceneCacheStore::new(&root);
    let revision_store = ImmutableRevisionStore::new(&root);
    let config = default_project_cache_config_hash();

    // Build revision 1.
    let id1 =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    let gen1 = store.advance_generation(scene_id, config)?;
    let mut d1 = SceneCacheDescriptorV3::invalidated(scene_id, gen1, config);
    d1.state = SceneCacheState::Ready;
    d1.source_content_hash = Some(id1.source_closure_digest);
    build_and_publish_managed_scene_cache_generation(&root, &d1)?;

    // Build revision 2 (unpinned historical revision that should be collected).
    write_triangle_mesh_to_scene(&canonical_scene, [1.0, 2.0, 3.0])?;
    let id2 =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    let gen2 = store.advance_generation(scene_id, config)?;
    let mut d2 = SceneCacheDescriptorV3::invalidated(scene_id, gen2, config);
    d2.state = SceneCacheState::Ready;
    d2.source_content_hash = Some(id2.source_closure_digest);
    build_and_publish_managed_scene_cache_generation(&root, &d2)?;

    // Build revision 3 (current).
    write_triangle_mesh_to_scene(&canonical_scene, [4.0, 5.0, 6.0])?;
    let id3 =
        compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    let gen3 = store.advance_generation(scene_id, config)?;
    let mut d3 = SceneCacheDescriptorV3::invalidated(scene_id, gen3, config);
    d3.state = SceneCacheState::Ready;
    d3.source_content_hash = Some(id3.source_closure_digest);
    build_and_publish_managed_scene_cache_generation(&root, &d3)?;
    let latest_scene_bytes = fs::read(&canonical_scene)?;

    // Plant an unreferenced blob in the shared CAS pool and an orphaned temp file.
    let shared_store = store.shared_object_store()?;
    let orphan_blob = shared_store.put(b"completely-unreferenced-derived-payload")?;
    let orphan_tmp = ProjectStorageLayout::new(&root).cache_revisions_dir()
        .join(format!(".revision.{}.tmp", uuid::Uuid::new_v4()));
    fs::create_dir(&orphan_tmp)?;
    fs::write(orphan_tmp.join("index.bin"), b"interrupted")?;

    // Pin revision 1 so GC MUST retain revision 1 and current revision 3, while collecting revision 2 and orphan_blob.
    let rev1 = revision_store
        .load_verified_revision(&id1)?
        .expect("revision 1 exists");
    let pin = CachePinGuard::acquire(
        &root,
        vec![rev1.record.revision_key.clone()],
        rev1.record.referenced_blobs.clone(),
    )?;

    let report = collect_garbage(
        &root,
        GcPolicy {
            max_retained_revisions_per_target: 1,
            collect_orphaned_temps: true,
        },
    )?;
    assert!(report.aborted_reason.is_none());
    assert!(report.removed_revisions >= 1);
    assert!(report.removed_blobs >= 1);
    assert!(
        shared_store.get(&orphan_blob)?.is_none(),
        "unreferenced blob must be collected"
    );
    assert!(
        revision_store.load_verified_revision(&id1)?.is_some(),
        "pinned historical revision must be protected from GC"
    );
    assert!(
        revision_store.load_verified_revision(&id3)?.is_some(),
        "current revision must be retained by GC"
    );
    assert!(
        revision_store.load_verified_revision(&id2)?.is_none(),
        "unpinned excess historical revision must be collected"
    );

    // Release pin and verify orphaned temp is cleaned on next GC pass.
    drop(pin);
    let second_report = collect_garbage(
        &root,
        GcPolicy {
            max_retained_revisions_per_target: 1,
            collect_orphaned_temps: true,
        },
    )?;
    assert!(second_report.aborted_reason.is_none());
    assert!(!orphan_tmp.exists());

    // Canonical Project files must be completely untouched.
    assert_eq!(fs::read(&canonical_scene)?, latest_scene_bytes);
    assert_ne!(latest_scene_bytes, original_scene_bytes);
    assert_eq!(fs::read(&canonical_manifest)?, original_manifest_bytes);
    Ok(())
}

#[test]
fn gc_aborts_conservatively_on_uncertain_reachability_and_protects_canonical_root() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("GC Conservative Abort")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let generation = store.advance_generation(scene_id, config)?;
    let mut desc = SceneCacheDescriptorV3::invalidated(scene_id, generation, config);
    desc.state = SceneCacheState::Ready;
    build_and_publish_scene_cache_generation(&root, &desc)?;

    let shared_store = store.shared_object_store()?;
    let orphan_blob = shared_store.put(b"should-survive-conservative-abort")?;

    // Corrupt a pin file so reachability is uncertain -> GC MUST abort and delete 0 objects.
    let pins_dir = ProjectStorageLayout::new(&root).cache_pins_dir();
    fs::create_dir_all(&pins_dir)?;
    let corrupt_pin = pins_dir.join("corrupt.json");
    fs::write(&corrupt_pin, b"not-valid-pin-json")?;

    let report = collect_garbage(&root, GcPolicy::default())?;
    assert!(
        report.aborted_reason.is_some(),
        "GC must abort conservatively when pin reachability is uncertain"
    );
    assert_eq!(report.removed_blobs, 0);
    assert_eq!(report.removed_revisions, 0);
    assert!(
        shared_store.get(&orphan_blob)?.is_some(),
        "aborted GC must not delete any blobs"
    );

    // Clean up corrupt pin and verify safe_remove_cache_file rejects paths outside cache root.
    fs::remove_file(&corrupt_pin)?;
    let canonical_cache_root = fs::canonicalize(ProjectStorageLayout::new(&root).cache_dir())?;
    let canonical_manifest = ProjectStorageLayout::new(&root).canonical_manifest_path();
    assert!(
        safe_remove_cache_file(&canonical_cache_root, &canonical_manifest).is_err(),
        "GC must refuse to delete any file outside verified cache root"
    );
    assert!(canonical_manifest.is_file());
    Ok(())
}

#[test]
fn branch_switch_during_preparation_cannot_overwrite_target_branch_cache() -> Result<()> {
    let dir = tempdir()?;
    let parent = dir.path().join("projects");
    fs::create_dir_all(&parent)?;
    let registry_path = dir.path().join("workspace.json");
    let mut service = ProjectApplicationService::open(&registry_path)?;
    let summary = service.create_project(&parent, "Branch Switch Race")?;
    let project_id = summary.id;
    let root = parent.join("Branch Switch Race");
    assert!(service.wait_for_cache_idle(&root));

    let manifest = ManifestStore::read_validated(&root)?;
    let ProjectRoot::Scene(scene_id) = manifest.raw().root else {
        panic!("created Project must have a protected root Scene");
    };
    write_triangle_mesh_to_scene(&scene_path(&root, scene_id), [0.0, 0.0, 0.0])?;
    service.commit(project_protocol::ProjectCommitRequest {
        project_id,
        target: project_protocol::ProjectCommitTarget::Project,
        message: "main baseline".to_owned(),
    })?;
    assert!(service.wait_for_cache_idle(&root));

    let base_branch = String::from_utf8(
        std::process::Command::new("git")
            .current_dir(&root)
            .args(["branch", "--show-current"])
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    run_git(&root, &["checkout", "-b", "feature"]);
    write_triangle_mesh_to_scene(&scene_path(&root, scene_id), [7.0, 8.0, 9.0])?;
    service.commit(project_protocol::ProjectCommitRequest {
        project_id,
        target: project_protocol::ProjectCommitTarget::Project,
        message: "feature update".to_owned(),
    })?;
    assert!(service.wait_for_cache_idle(&root));
    service.switch_branch(project_id, &base_branch)?;
    assert!(service.wait_for_cache_idle(&root));

    // Start preparing a generation on base_branch.
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let target = ProjectCacheTarget::Scene {
        id: scene_id.to_string(),
    };
    let gen_master = store.advance_generation(scene_id, config)?;
    let master_hash = crate::project::assets::closure::target_content_hash(&root, &target)?;

    // Switch to feature branch while base_branch preparation is still in flight.
    service.switch_branch(project_id, "feature")?;
    assert!(service.wait_for_cache_idle(&root));

    // Stale base_branch worker attempts to publish gen_master -> rejected by CAS.
    let mut stale_master = SceneCacheDescriptorV3::invalidated(scene_id, gen_master, config);
    stale_master.state = SceneCacheState::Ready;
    stale_master.source_content_hash = Some(master_hash);
    assert!(
        build_and_publish_managed_scene_cache_generation(&root, &stale_master)?.is_none(),
        "stale pre-branch-switch build must not overwrite target branch cache"
    );
    let current = store
        .load_descriptor(scene_id)?
        .expect("feature branch descriptor remains current");
    assert!(current.generation > gen_master);
    Ok(())
}


#[test]
fn older_acknowledgement_cannot_remove_newer_desired_generation() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Desired Generation Ordering")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let target = ProjectCacheTarget::Scene { id: scene_id.to_string() };
    let first = store.advance_generation(scene_id, config)?;
    let pending = PendingIntentStore::new(&root);
    let old = pending.load(&target)?.expect("first intent");
    let second = store.advance_generation(scene_id, config)?;
    assert!(pending.record(&old).is_err(), "late old intent must not overwrite newer desired work");
    pending.acknowledge_if_generation(&target, first, old.desired_revision_identity)?;
    assert_eq!(pending.load(&target)?.expect("newer intent survives").generation, second);
    Ok(())
}

#[test]
fn garbage_collection_refuses_aliased_cache_root_before_deletion() -> Result<()> {
    let dir = tempdir()?;
    let root = dir.path().join("project");
    fs::create_dir_all(&root)?;
    let layout = ProjectStorageLayout::new(&root);
    fs::create_dir_all(layout.metadata_dir())?;
    let sentinel = root.join(".unrelated.tmp");
    fs::write(&sentinel, b"canonical sentinel")?;
    let alias_path = layout.cache_dir();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, &alias_path)?;
    }
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                alias_path.to_string_lossy().as_ref(),
                root.to_string_lossy().as_ref(),
            ])
            .status()?;
        assert!(status.success(), "mklink /J must create directory junction");
    }
    assert!(collect_garbage(&root, GcPolicy::default()).is_err());
    assert_eq!(fs::read(sentinel)?, b"canonical sentinel");
    #[cfg(windows)]
    {
        let _ = fs::remove_dir(&alias_path);
    }
    Ok(())
}

#[test]
fn retired_generation_floor_survives_reactivation() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Retirement Floor")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let target = ProjectCacheTarget::Scene { id: scene_id.to_string() };
    let old = store.advance_generation(scene_id, config)?;
    let floor = store.retire_scene(scene_id)?;
    assert!(floor > old);
    let new = store.advance_generation(scene_id, config)?;
    assert!(new > floor);
    assert!(TombstoneStore::new(&root).is_retired_at(&target, old)?);
    assert!(!TombstoneStore::new(&root).is_retired_at(&target, new)?);
    Ok(())
}

#[test]
fn damaged_retained_revision_recovers_same_identity_after_leases_release() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Retained Revision Repair")?;
    let store = SceneCacheStore::new(&root);
    let retained = ImmutableRevisionStore::new(&root);
    let config = default_project_cache_config_hash();
    let identity = compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    let first = store.advance_generation(scene_id, config)?;
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, first, config);
    descriptor.state = SceneCacheState::Ready;
    descriptor.source_content_hash = Some(identity.source_closure_digest);
    assert!(build_and_publish_managed_scene_cache_generation(&root, &descriptor)?.is_some());
    let verified = retained.load_verified_revision(&identity)?.expect("first verified revision");
    let pin = CachePinGuard::acquire(&root, vec![identity.storage_key()], verified.record.referenced_blobs)?;
    let revision_path = retained.revision_dir(&identity.storage_key());
    let source_before = fs::read(scene_path(&root, scene_id))?;
    fs::write(revision_path.join("index.bin"), b"corrupt retained index")?;
    assert!(retained.load_verified_revision(&identity)?.is_none());
    assert!(collect_garbage(&root, GcPolicy::default())?.aborted_reason.is_some());
    let next = store.advance_generation(scene_id, config)?;
    let mut replacement = SceneCacheDescriptorV3::invalidated(scene_id, next, config);
    replacement.state = SceneCacheState::Ready;
    replacement.source_content_hash = Some(identity.source_closure_digest);
    assert!(build_and_publish_managed_scene_cache_generation(&root, &replacement).is_err());
    assert_eq!(fs::read(revision_path.join("index.bin"))?, b"corrupt retained index");
    drop(pin);
    assert!(build_and_publish_managed_scene_cache_generation(&root, &replacement)?.is_some());
    assert!(retained.load_verified_revision(&identity)?.is_some());
    assert!(store.load_activation(scene_id)?.is_some());
    assert!(collect_garbage(&root, GcPolicy::default())?.aborted_reason.is_none());
    let quarantines = fs::read_dir(ProjectStorageLayout::new(&root).cache_revisions_dir())?
        .collect::<std::io::Result<Vec<_>>>()?;
    assert!(quarantines.iter().any(|entry| entry.file_name().to_string_lossy().starts_with(".quarantine.")));
    assert_eq!(fs::read(scene_path(&root, scene_id))?, source_before);
    Ok(())
}

#[test]
fn pending_recovery_replaces_obsolete_source_identity_with_new_generation() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Obsolete Pending Identity")?;
    let target = ProjectCacheTarget::Scene { id: scene_id.to_string() };
    let queue = ProjectCacheWarmQueue::default();
    queue.shutdown_without_waiting();
    assert!(!queue.enqueue_targets_for_mutation(&root, vec![target.clone()])?);
    let pending = PendingIntentStore::new(&root);
    let old = pending.load(&target)?.expect("offline pending identity");
    write_triangle_mesh_to_scene(&scene_path(&root, scene_id), [11.0, 0.0, 0.0])?;
    let expected = compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, old.config_hash)?;
    assert_ne!(old.desired_revision_identity, Some(expected.digest()));
    let resumed = ProjectCacheWarmQueue::default();
    assert_eq!(resumed.recover_pending_intents(&root), 1);
    assert!(resumed.wait_for_project_idle(&root, Duration::from_secs(10)));
    let current = SceneCacheStore::new(&root).load_descriptor(scene_id)?.expect("recovered current descriptor");
    assert!(current.generation > old.generation);
    assert_eq!(current.revision_identity, Some(expected.digest()));
    assert!(pending.load(&target)?.is_none());
    Ok(())
}

#[test]
fn later_barrier_survives_earlier_reconciliation_transaction() -> Result<()> {
    use crate::cache::sync::{cache_invalidation_barrier, clear_cache_invalidation_barrier, set_cache_invalidation_barrier, with_cache_lock};
    let (_dir, root, _) = create_mesh_scene_project("Barrier Ordering")?;
    set_cache_invalidation_barrier(&root)?;
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let worker_root = root.clone();
    let handle = with_cache_lock(&root, "reconcile", || {
        let handle = std::thread::spawn(move || -> Result<()> {
            started_tx.send(())?;
            set_cache_invalidation_barrier(&worker_root)
        });
        started_rx.recv()?;
        clear_cache_invalidation_barrier(&root)?;
        Ok(handle)
    })?;
    handle.join().expect("barrier writer completes")?;
    assert!(cache_invalidation_barrier(&root)?);
    Ok(())
}

#[test]
fn logical_retirement_keeps_unleased_legacy_payloads_and_indexes() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Legacy Reader Retirement")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let generation = store.advance_generation(scene_id, config)?;
    let mut ready = SceneCacheDescriptorV3::invalidated(scene_id, generation, config);
    ready.state = SceneCacheState::Ready;
    let index = build_and_publish_managed_scene_cache_generation(&root, &ready)?.expect("ready generation");
    let blob = index.entries.iter().find_map(|entry| entry.geometry.as_ref().map(|g| g.blob_id.clone())).expect("geometry blob");
    let objects = store.object_store(scene_id)?;
    let before = objects.get(&blob)?.expect("legacy resource");
    let layout = ProjectStorageLayout::new(&root);
    let index_before = fs::read(layout.scene_cache_index_path(scene_id))?;
    let spatial_before = fs::read(layout.scene_cache_spatial_path(scene_id))?;
    store.retire_scene(scene_id)?;
    assert!(store.load_descriptor(scene_id)?.is_none());
    assert!(store.load_activation(scene_id)?.is_none());
    assert_eq!(objects.get(&blob)?, Some(before));
    assert_eq!(fs::read(layout.scene_cache_index_path(scene_id))?, index_before);
    assert_eq!(fs::read(layout.scene_cache_spatial_path(scene_id))?, spatial_before);
    Ok(())
}

#[test]
fn non_scene_desired_generation_prevents_stale_publication_and_recovers_offline_work() -> Result<()> {
    use crate::cache::{ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore};
    let (_dir, root, scene_id) = create_mesh_scene_project("Managed Root Generation")?;
    let target = ProjectCacheTarget::ProjectRoot;
    let config = default_project_cache_config_hash();
    let store = ProjectCacheStore::new(&root);
    let first = store.advance_generation(&target, config)?;
    let identity = ProjectCacheIdentity::for_project(&root, target.clone(), RuntimeProfile::NativeMedium, config)?;
    let digest = identity.revision_digest(&root)?;
    let building = ProjectCacheDescriptor::new(identity.clone(), ProjectCacheState::Building, None)?;
    assert!(store.publish_generation(&building, first, digest)?);
    store.retire_target(&target)?;
    assert!(!store.publish_generation(&building, first, digest)?);
    assert!(store.load(&identity)?.is_none());
    let floor = TombstoneStore::new(&root).reactivation_floor(&target)?;
    let offline = ProjectCacheWarmQueue::default();
    offline.shutdown_without_waiting();
    assert!(!offline.enqueue_targets_for_mutation(&root, vec![target.clone()])?);
    let pending = PendingIntentStore::new(&root);
    let old = pending.load(&target)?.expect("non-Scene offline intent");
    assert!(old.generation > floor);
    write_triangle_mesh_to_scene(&scene_path(&root, scene_id), [0.0, 7.0, 0.0])?;
    let resumed = ProjectCacheWarmQueue::default();
    assert_eq!(resumed.recover_pending_intents(&root), 1);
    assert!(resumed.wait_for_project_idle(&root, Duration::from_secs(10)));
    let now = ProjectCacheIdentity::for_project(&root, target.clone(), RuntimeProfile::NativeMedium, config)?;
    let current = store.load(&now)?.expect("current Root cache");
    assert_eq!(current.state, ProjectCacheState::Ready);
    assert!(pending.generation_floor(&target)? > old.generation);
    assert!(pending.load(&target)?.is_none());
    assert!(!store.publish_generation(&building, first, digest)?);
    Ok(())
}

#[cfg(any(unix, windows))]
#[test]
fn nested_managed_aliases_are_rejected_before_creating_descendants() -> Result<()> {
    use crate::cache::blobs::FilesystemBlobStore;
    use crate::cache::sync::{qualify_cache_directory, qualified_cache_root, write_bytes_atomic_durable};
    let dir = tempdir()?;
    let root = dir.path().join("project");
    let external = dir.path().join("external");
    fs::create_dir(&root)?;
    fs::create_dir(&external)?;
    fs::write(external.join("sentinel"), b"untouched")?;
    qualified_cache_root(&root, true)?;
    let cache = ProjectStorageLayout::new(&root).cache_dir();
    for namespace in ["scenes", "objects", "pending", "tombstones", "pins", "revisions", "assemblies", "descriptors"] {
        let alias = cache.join(namespace);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&external, &alias)?;
        #[cfg(windows)]
        assert!(std::process::Command::new("cmd").args(["/C", "mklink", "/J"])
            .arg(&alias).arg(&external).status()?.success());
        let missing = alias.join("missing-child");
        assert!(qualify_cache_directory(&missing, true).is_err());
        assert!(write_bytes_atomic_durable(&missing.join("owned.tmp"), &missing.join("record.json"), b"data").is_err());
        assert!(FilesystemBlobStore::new(&missing).is_err());
        assert!(!external.join("missing-child").exists());
        assert_eq!(fs::read(external.join("sentinel"))?, b"untouched");
        #[cfg(unix)]
        fs::remove_file(&alias)?;
        #[cfg(windows)]
        fs::remove_dir(&alias)?;
        assert!(qualify_cache_directory(&missing, true)?);
        assert!(missing.is_dir());
    }
    Ok(())
}

#[test]
fn captured_scene_geometry_and_identity_survive_external_aba_without_mislabeling() -> Result<()> {
    use crate::cache::identity::scene_revision_identity_from_snapshot;
    use crate::cache::scene::build::build_scene_cache_index_from_snapshot;
    let (_dir, root, scene_id) = create_mesh_scene_project("Sealed Scene Source")?;
    let path = scene_path(&root, scene_id);
    let original = fs::read(&path)?;
    let config = default_project_cache_config_hash();
    let a = usd_bevy::LiveStage::open_document(&path)?;
    let identity_a = scene_revision_identity_from_snapshot(&root, scene_id, RuntimeProfile::NativeMedium, config, &a)?;
    write_triangle_mesh_to_scene(&path, [21.0, 0.0, 0.0])?;
    let b = usd_bevy::LiveStage::open_document(&path)?;
    let (a_index, _) = build_scene_cache_index_from_snapshot(&root, scene_id, 1, &a)?;
    let (b_index, _) = build_scene_cache_index_from_snapshot(&root, scene_id, 2, &b)?;
    fs::write(&path, &original)?;
    let a_again = usd_bevy::LiveStage::open_document(&path)?;
    let (again_index, _) = build_scene_cache_index_from_snapshot(&root, scene_id, 3, &a_again)?;
    let geometry = |index: &crate::cache::contract::SceneCacheIndex| {
        index.entries.iter().find_map(|entry| entry.geometry.as_ref().map(|g| g.blob_id.clone()))
            .expect("triangle geometry is present")
    };
    assert_eq!(geometry(&a_index), geometry(&again_index), "A snapshot cannot follow live B bytes");
    assert_ne!(geometry(&a_index), geometry(&b_index));
    assert_eq!(compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?, identity_a);
    // Even when the owned source hash is again A, captured B's full closure
    // must not acquire A's immutable identity.
    let b_after_restore = scene_revision_identity_from_snapshot(&root, scene_id, RuntimeProfile::NativeMedium, config, &b)?;
    assert_ne!(b_after_restore.digest(), identity_a.digest());
    assert_eq!(fs::read(&path)?, original);
    Ok(())
}

#[test]
fn standalone_dependency_capture_and_managed_root_publication_bind_exact_source() -> Result<()> {
    use crate::cache::{ProjectCacheDescriptor, ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore};
    use crate::cache::generic::source_identity_from_opened_stage;
    let dir = tempdir()?;
    let entry = dir.path().join("root.usda");
    let child = dir.path().join("child.usda");
    fs::write(&entry, b"#usda 1.0\n(\n    subLayers = [@./child.usda@]\n)\n")?;
    let child_a = b"#usda 1.0\ndef Xform \"A\" {}\n";
    let child_b = b"#usda 1.0\ndef Xform \"B\" {}\n";
    fs::write(&child, child_a)?;
    let a = usd_bevy::LiveStage::open_document(&entry)?;
    let a_id = source_identity_from_opened_stage(&a)?;
    fs::write(&child, child_b)?;
    let b = usd_bevy::LiveStage::open_document(&entry)?;
    fs::write(&child, child_a)?;
    assert_eq!(a_id, compute_standalone_source_identity(&entry)?);
    assert_ne!(source_identity_from_opened_stage(&b)?, a_id);
    assert_eq!(source_identity_from_opened_stage(&a)?, a_id);

    let (_dir, root, scene_id) = create_mesh_scene_project("Sealed Project Root")?;
    let source = scene_path(&root, scene_id);
    let original = fs::read(&source)?;
    let config = default_project_cache_config_hash();
    let target = ProjectCacheTarget::ProjectRoot;
    let store = ProjectCacheStore::new(&root);
    let generation = store.advance_generation(&target, config)?;
    let expected = ProjectCacheIdentity::for_project(&root, target.clone(), RuntimeProfile::NativeMedium, config)?;
    let desired = expected.revision_digest(&root)?;
    write_triangle_mesh_to_scene(&source, [25.0, 0.0, 0.0])?;
    let capture_b = usd_bevy::LiveStage::open_document(&source)?;
    fs::write(&source, original)?;
    let revision_b = expected.revision_digest_from_snapshot(&root, Some(&capture_b))?;
    assert_ne!(revision_b, desired);
    let building = ProjectCacheDescriptor::new(expected, ProjectCacheState::Building, None)?;
    assert!(!store.publish_generation(&building, generation, revision_b)?);
    assert_eq!(PendingIntentStore::new(&root).load(&target)?.expect("desired obligation remains").desired_revision_identity, Some(desired));
    Ok(())
}

#[test]
fn corrupt_optional_lod_catalogue_aborts_collection_before_any_sweep() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("LOD Reachability")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let generation = store.advance_generation(scene_id, config)?;
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, generation, config);
    descriptor.state = SceneCacheState::Ready;
    assert!(build_and_publish_managed_scene_cache_generation(&root, &descriptor)?.is_some());
    let shared = store.shared_object_store()?;
    let orphan = shared.put(b"orphan protected by uncertain LOD root")?;
    let layout = ProjectStorageLayout::new(&root);
    let path = layout.scene_cache_dir(scene_id).join(format!("generated-lod-v1-{generation}.json"));
    for corrupt in [b"not json".as_slice(), b"{\"schema\":1,\"recipe\":\"wrong\",\"generation\":0,\"candidates\":{}}".as_slice()] {
        fs::write(&path, corrupt)?;
        let report = collect_garbage(&root, GcPolicy::default())?;
        assert!(report.aborted_reason.is_some());
        assert_eq!((report.removed_revisions, report.removed_blobs, report.removed_temps), (0, 0, 0));
        assert!(shared.get(&orphan)?.is_some());
    }
    fs::remove_file(&path)?;
    let report = collect_garbage(&root, GcPolicy::default())?;
    assert!(report.aborted_reason.is_none(), "qualified optional absence is not corruption");
    assert!(shared.get(&orphan)?.is_none());
    Ok(())
}

#[test]
fn production_warm_and_retirement_queue_runs_gc_without_stage_activation() -> Result<()> {
    let (_dir, root, scene_id) = create_mesh_scene_project("Production Cache Maintenance")?;
    let store = SceneCacheStore::new(&root);
    let shared = store.shared_object_store()?;
    let orphan = shared.put(b"unreferenced shared object")?;
    let protected = shared.put(b"active shared lease")?;
    let pin = CachePinGuard::acquire(&root, Vec::new(), vec![protected.clone()])?;
    let target = ProjectCacheTarget::Scene { id: scene_id.to_string() };
    let queue = ProjectCacheWarmQueue::default();
    let canonical_before = fs::read(scene_path(&root, scene_id))?;
    assert!(queue.enqueue_targets_for_mutation(&root, vec![target.clone()])?);
    assert!(queue.wait_for_project_idle(&root, Duration::from_secs(30)));
    assert!(shared.get(&orphan)?.is_none(), "real warm completion must service maintenance");
    assert!(shared.get(&protected)?.is_some(), "worker collection must preserve active pins");
    drop(pin);
    assert!(queue.remove_target_descriptors(&root, &target));
    assert!(queue.wait_for_project_idle(&root, Duration::from_secs(30)));
    assert!(shared.get(&protected)?.is_none(), "real retirement trigger must collect unleased orphan");
    assert_eq!(fs::read(scene_path(&root, scene_id))?, canonical_before);
    Ok(())
}

#[test]
fn malformed_active_pin_roots_abort_before_any_sweep_and_recover() -> Result<()> {
    let (_directory, root, scene_id) = create_mesh_scene_project("Pin Reference Integrity")?;
    let store = SceneCacheStore::new(&root);
    let config = default_project_cache_config_hash();
    let generation = store.advance_generation(scene_id, config)?;
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, generation, config);
    descriptor.state = SceneCacheState::Ready;
    assert!(build_and_publish_managed_scene_cache_generation(&root, &descriptor)?.is_some());
    let layout = ProjectStorageLayout::new(&root);
    let retained = ImmutableRevisionStore::new(&root);
    let revision = compute_scene_revision_identity(&root, scene_id, RuntimeProfile::NativeMedium, config)?;
    let revision_path = retained.revision_dir(&revision.storage_key());
    store.retire_scene(scene_id)?;
    let shared = store.shared_object_store()?;
    let protected = shared.put(b"protected direct reader")?;
    let orphan = shared.put(b"unreferenced object")?;
    let staging = layout.cache_revisions_dir().join(format!(".revision.{}.tmp", uuid::Uuid::new_v4()));
    fs::create_dir(&staging)?;
    fs::write(staging.join("marker"), b"staged")?;
    let reservation = "a".repeat(64);
    assert!(CachePinGuard::acquire(&root, vec!["not-a-key".to_owned()], Vec::new()).is_err());
    assert!(CachePinGuard::acquire(&root, Vec::new(), vec![usd_model::BlobId("not-a-digest".to_owned())]).is_err());
    let pin = CachePinGuard::acquire(&root, vec![reservation], vec![protected.clone()])?;
    let record_path = layout.cache_pins_dir().join(format!("{}.json", pin.pin_id()));
    let original = fs::read(&record_path)?;
    let original_value: serde_json::Value = serde_json::from_slice(&original)?;
    let policy = GcPolicy { max_retained_revisions_per_target: 0, collect_orphaned_temps: true };
    let assert_aborted = || -> Result<()> {
        let report = collect_garbage(&root, policy)?;
        assert!(report.aborted_reason.is_some());
        assert_eq!((report.removed_revisions, report.removed_blobs, report.removed_temps), (0, 0, 0));
        assert!(shared.get(&protected)?.is_some());
        assert!(shared.get(&orphan)?.is_some());
        assert!(revision_path.is_dir());
        assert!(staging.is_dir());
        Ok(())
    };
    for (field, invalid) in [
        ("blob_ids", serde_json::json!(["not-a-digest"])),
        ("revision_keys", serde_json::json!(["not-a-key"])),
        ("blob_ids", serde_json::to_value(vec![orphan.clone()])?),
        ("blob_ids", serde_json::json!([])),
        ("integrity", serde_json::Value::Null),
    ] {
        let mut damaged = original_value.clone();
        damaged[field] = invalid;
        fs::write(&record_path, serde_json::to_vec(&damaged)?)?;
        assert_aborted()?;
    }
    fs::remove_file(&record_path)?;
    assert_aborted()?;
    // Reinstating the exact original bytes restores normal collection. The
    // valid uncommitted reservation does not require a committed revision.
    fs::write(&record_path, &original)?;
    let report = collect_garbage(&root, policy)?;
    assert!(report.aborted_reason.is_none());
    assert!(!revision_path.exists());
    assert!(shared.get(&protected)?.is_some());
    assert!(shared.get(&orphan)?.is_none());
    assert!(staging.is_dir());
    drop(pin);
    let report = collect_garbage(&root, policy)?;
    assert!(report.aborted_reason.is_none());
    assert!(shared.get(&protected)?.is_none());
    assert!(!staging.exists());
    let repeat = collect_garbage(&root, policy)?;
    assert_eq!((repeat.removed_revisions, repeat.removed_blobs, repeat.removed_temps), (0, 0, 0));
    Ok(())
}

#[test]
fn legacy_pin_defers_gc_until_the_exact_lease_is_inactive() -> Result<()> {
    let (_directory, root, _) = create_mesh_scene_project("Legacy Pin Liveness")?;
    let store = SceneCacheStore::new(&root);
    let shared = store.shared_object_store()?;
    let blob = shared.put(b"legacy protected object")?;
    let pin = CachePinGuard::acquire(&root, Vec::new(), vec![blob.clone()])?;
    let path = ProjectStorageLayout::new(&root).cache_pins_dir().join(format!("{}.json", pin.pin_id()));
    let mut legacy: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    legacy["schema_version"] = serde_json::json!(1);
    legacy.as_object_mut().context("pin JSON object")?.remove("integrity");
    let bytes = serde_json::to_vec(&legacy)?;
    fs::write(&path, &bytes)?;
    let report = collect_garbage(&root, GcPolicy::default())?;
    assert!(report.aborted_reason.is_some());
    assert_eq!((report.removed_revisions, report.removed_blobs, report.removed_temps), (0, 0, 0));
    assert!(shared.get(&blob)?.is_some());
    drop(pin);
    let report = collect_garbage(&root, GcPolicy::default())?;
    assert!(report.aborted_reason.is_none());
    assert!(shared.get(&blob)?.is_none());
    assert_eq!(fs::read(path)?, bytes, "legacy record is never rewritten by GC");
    Ok(())
}

#[test]
fn committed_digest_named_file_is_uncertain_revision_reachability() -> Result<()> {
    let (_directory, root, _) = create_mesh_scene_project("Malformed Revision Entry")?;
    let store = SceneCacheStore::new(&root);
    let shared = store.shared_object_store()?;
    let orphan = shared.put(b"must survive uncertain committed root")?;
    let revisions = ProjectStorageLayout::new(&root).cache_revisions_dir();
    fs::create_dir_all(&revisions)?;
    let bad_root = revisions.join("c".repeat(64));
    fs::write(&bad_root, b"not a committed revision directory")?;
    let report = collect_garbage(&root, GcPolicy::default())?;
    assert!(report.aborted_reason.is_some());
    assert_eq!((report.removed_revisions, report.removed_blobs, report.removed_temps), (0, 0, 0));
    assert!(shared.get(&orphan)?.is_some());
    assert!(bad_root.is_file());
    fs::remove_file(bad_root)?;
    let report = collect_garbage(&root, GcPolicy::default())?;
    assert!(report.aborted_reason.is_none());
    assert!(shared.get(&orphan)?.is_none());
    Ok(())
}
