use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::mpsc;

use anyhow::Result;
use image::RgbaImage;
use openusd::schemas::shade::{Connectable, Material, MaterialBindingAPI, Shader};
use openusd::sdf;
use tempfile::tempdir;
use usd_model::HashDigest;
use usd_project::{ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, StorageKey};

use super::{
    SceneCacheDescriptorV3, SceneCacheStore, build_and_publish_managed_scene_cache_generation,
};
use crate::project::blob_store::BlobStore;
use crate::project::cache::{ProjectCacheTarget, target_content_hash};
use crate::project::cache_contract::SceneCacheState;

struct BuildPublishHook {
    project_root: PathBuf,
    callback: Arc<dyn Fn() + Send + Sync + 'static>,
}

static BUILD_PUBLISH_HOOK: OnceLock<Mutex<Option<BuildPublishHook>>> = OnceLock::new();

pub(super) fn run_build_publish_hook(project_root: &Path) {
    let hook = BUILD_PUBLISH_HOOK.get().and_then(|hooks| {
        let hooks = hooks.lock().ok()?;
        let hook = hooks.as_ref()?;
        (hook.project_root == project_root).then(|| Arc::clone(&hook.callback))
    });
    if let Some(hook) = hook {
        hook();
    }
}

#[test]
fn managed_runtime_publisher_rejects_intervening_same_generation_update() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Runtime CAS snapshot",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene")?,
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        directory.path(),
        &manifest,
    )?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;

    let config_hash = HashDigest::new([9; HashDigest::BYTE_LEN]);
    let mut descriptor = SceneCacheDescriptorV3::invalidated(scene_id, 7, config_hash);
    descriptor.state = SceneCacheState::Partial;
    let store = SceneCacheStore::new(directory.path());
    store.publish_descriptor(&descriptor)?;

    let (built_tx, built_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let hook_release = Arc::clone(&release_rx);
    let hooks = BUILD_PUBLISH_HOOK.get_or_init(|| Mutex::new(None));
    *hooks.lock().expect("CAS test hook lock is not poisoned") = Some(BuildPublishHook {
        project_root: directory.path().to_path_buf(),
        callback: Arc::new(move || {
            built_tx.send(()).expect("CAS test worker reached publish barrier");
            hook_release
                .lock()
                .expect("CAS test release lock is not poisoned")
                .recv()
                .expect("CAS test publish barrier released");
        }),
    });

    let root = directory.path().to_path_buf();
    let worker_descriptor = descriptor.clone();
    let worker = std::thread::spawn(move || {
        build_and_publish_managed_scene_cache_generation(&root, &worker_descriptor)
    });

    built_rx.recv().expect("runtime publisher built from its snapshot");
    let mut winner = descriptor.clone();
    winner.state = SceneCacheState::Ready;
    store.publish_descriptor(&winner)?;
    release_tx.send(()).expect("CAS test worker can resume");

    let result = worker.join().expect("runtime publisher thread did not panic")?;
    *hooks.lock().expect("CAS test hook lock is not poisoned") = None;

    assert!(result.is_none(), "stale runtime build must lose the CAS");
    assert_eq!(
        store.load_descriptor(scene_id)?.expect("winner survives").state,
        SceneCacheState::Ready
    );
    Ok(())
}

#[test]
fn managed_runtime_publisher_preserves_pre_capture_same_generation_evolution() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Runtime CAS merge",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene")?,
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        directory.path(),
        &manifest,
    )?;
    crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;

    let config_hash = HashDigest::new([7; HashDigest::BYTE_LEN]);
    let mut stale = SceneCacheDescriptorV3::invalidated(scene_id, 9, config_hash);
    stale.state = SceneCacheState::Partial;
    let store = SceneCacheStore::new(directory.path());
    store.publish_descriptor(&stale)?;

    let mut evolved = stale.clone();
    evolved.state = SceneCacheState::Ready;
    evolved.prim_count = 4;
    evolved.prim_count_ready = true;
    evolved.cacheable_count = 2;
    store.publish_descriptor(&evolved)?;

    let caller_source_content_hash = target_content_hash(
        directory.path(),
        &ProjectCacheTarget::Scene {
            id: scene_id.to_string(),
        },
    )?;
    let mut caller_descriptor = stale;
    caller_descriptor.source_content_hash = Some(caller_source_content_hash);
    build_and_publish_managed_scene_cache_generation(directory.path(), &caller_descriptor)?
        .expect("compatible same-generation evolution remains publishable");

    let published = store
        .load_descriptor(scene_id)?
        .expect("rebased descriptor is published");
    assert_eq!(published.state, SceneCacheState::Ready);
    assert!(published.prim_count_ready);
    assert_eq!(published.source_content_hash, Some(caller_source_content_hash));
    Ok(())
}

#[test]
fn scene_owned_builder_persists_material_and_texture_blob_references() -> Result<()> {
    let directory = tempdir()?;
    usd_git::Repository::init(directory.path())?;
    let scene_id = SceneId::new_v4();
    let manifest = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Scene payload coverage",
        ProjectRoot::Scene(scene_id),
        vec![SceneManifestEntry {
            id: scene_id,
            storage_key: StorageKey::new("scene")?,
            display_name: "Scene".to_owned(),
        }],
        Vec::new(),
    );
    crate::project::catalog::manifest_store::ManifestStore::write_manifest_atomic(
        directory.path(),
        &manifest,
    )?;
    let scene_path = crate::project::scene::authoring::author_scene_atomic(
        directory.path(),
        scene_id,
    )?;
    let texture_path = directory.path().join("scene-diffuse.png");
    RgbaImage::from_pixel(1, 1, image::Rgba([48, 96, 192, 255])).save(&texture_path)?;
    let stage = openusd::usd::Stage::open(scene_path.to_string_lossy().as_ref())?;

    let material = Material::define(&stage, "/SceneRoot/Materials/Mat")?;
    let texture = Shader::define(&stage, "/SceneRoot/Materials/Mat/Texture")?;
    texture
        .create_id_attr()?
        .set(sdf::Value::token("UsdUVTexture"))?;
    texture
        .create_input("file", "asset")?
        .set(sdf::Value::AssetPath(texture_path.to_string_lossy().into_owned().into()))?;
    texture.create_output("rgb", "float3")?;
    let surface = Shader::define(&stage, "/SceneRoot/Materials/Mat/Surface")?;
    surface
        .create_id_attr()?
        .set(sdf::Value::token("UsdPreviewSurface"))?;
    surface
        .create_input("diffuseColor", "color3f")?
        .set_connections([sdf::path(
            "/SceneRoot/Materials/Mat/Texture.outputs:rgb",
        )?])?;
    surface
        .create_input("roughness", "float")?
        .set(sdf::Value::Float(0.35))?;
    surface.create_output("surface", "token")?;
    material
        .create_surface_output()?
        .set_connections([sdf::path(
            "/SceneRoot/Materials/Mat/Surface.outputs:surface",
        )?])?;

    let mesh = stage
        .define_prim("/SceneRoot/Renderable")?
        .set_type_name("Mesh")?;
    mesh.create_attribute("points", "point3f[]")?
        .set(sdf::Value::Vec3fVec(vec![
            openusd::gf::Vec3f::from([0.0, 0.0, 0.0]),
            openusd::gf::Vec3f::from([1.0, 0.0, 0.0]),
            openusd::gf::Vec3f::from([0.0, 1.0, 0.0]),
        ]))?;
    mesh.create_attribute("faceVertexCounts", "int[]")?
        .set(sdf::Value::IntVec(vec![3]))?;
    mesh.create_attribute("faceVertexIndices", "int[]")?
        .set(sdf::Value::IntVec(vec![0, 1, 2]))?;
    MaterialBindingAPI::apply(&stage, sdf::path("/SceneRoot/Renderable")?)?
        .bind(sdf::path("/SceneRoot/Materials/Mat")?)?;
    let temporary = scene_path.with_extension("payload.usda");
    stage
        .root_layer()
        .export(temporary.to_string_lossy().as_ref())?;
    fs::rename(temporary, &scene_path)?;

    let descriptor = SceneCacheDescriptorV3::invalidated(
        scene_id,
        1,
        HashDigest::new([11; HashDigest::BYTE_LEN]),
    );
    let index = super::build_and_publish_scene_cache_generation(directory.path(), &descriptor)?;
    let entry = index
        .entries
        .iter()
        .find(|entry| matches!(
            &entry.kind,
            crate::project::cache_contract::SceneCacheEntryKind::OwnedPrim { prim_path }
                if prim_path == "/SceneRoot/Renderable"
        ))
        .expect("Scene mesh cache entry");
    let material_ref = entry.material.as_ref().expect("material blob reference");
    let store = SceneCacheStore::new(directory.path()).object_store(scene_id)?;
    let material_bytes = store
        .get(&material_ref.blob_id)?
        .expect("material blob persisted");
    let material: crate::project::runtime_payload::RuntimeMaterialBlob =
        serde_json::from_slice(&material_bytes)?;
    material.validate()?;
    let texture_id = material
        .textures
        .base_color
        .as_ref()
        .expect("base-color texture reference");
    let texture_bytes = store
        .get(&usd_model::BlobId(texture_id.clone()))?
        .expect("texture blob persisted");
    let texture: crate::project::runtime_payload::RuntimeTextureBlob =
        serde_json::from_slice(&texture_bytes)?;
    texture.validate()?;
    assert_eq!(texture.rgba8, vec![48, 96, 192, 255]);
    Ok(())
}
