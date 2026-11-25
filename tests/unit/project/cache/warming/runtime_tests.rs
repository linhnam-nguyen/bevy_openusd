use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use image::RgbaImage;
use openusd::sdf;
use openusd::vt;
use openusd_schemas::shade::{Connectable, Material, MaterialBindingAPI, Shader};
use tempfile::tempdir;
use usd_model::HashDigest;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneId, SceneManifestEntry, StorageKey,
};

use super::{
    GenerationPublishObserver, SceneCacheDescriptorV3, SceneCacheStore,
    build_and_publish_managed_scene_cache_generation,
    build_and_publish_managed_scene_cache_generation_with_observer,
};
use crate::cache::blobs::BlobStore;
use crate::cache::contract::SceneCacheState;
use crate::cache::ProjectCacheTarget;
use crate::project::assets::closure::target_content_hash;

struct BarrierPublishObserver {
    built_tx: mpsc::Sender<()>,
    release_rx: Arc<Mutex<mpsc::Receiver<()>>>,
}

impl GenerationPublishObserver for BarrierPublishObserver {
    fn before_publish(&self, _project_root: &Path) {
        self.built_tx
            .send(())
            .expect("CAS test worker reached publish barrier");
        self.release_rx
            .lock()
            .expect("CAS test release lock is not poisoned")
            .recv()
            .expect("CAS test publish barrier released");
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
    let observer = BarrierPublishObserver {
        built_tx,
        release_rx: Arc::new(Mutex::new(release_rx)),
    };

    let root = directory.path().to_path_buf();
    let worker_descriptor = descriptor.clone();
    let worker = std::thread::spawn(move || {
        build_and_publish_managed_scene_cache_generation_with_observer(
            &root,
            &worker_descriptor,
            &observer,
        )
    });

    built_rx
        .recv()
        .expect("runtime publisher built from its snapshot");
    let mut winner = descriptor.clone();
    winner.state = SceneCacheState::Ready;
    store.publish_descriptor(&winner)?;
    release_tx.send(()).expect("CAS test worker can resume");

    let result = worker
        .join()
        .expect("runtime publisher thread did not panic")?;

    assert!(result.is_none(), "stale runtime build must lose the CAS");
    assert_eq!(
        store
            .load_descriptor(scene_id)?
            .expect("winner survives")
            .state,
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
    assert_eq!(
        published.source_content_hash,
        Some(caller_source_content_hash)
    );
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
    let scene_path =
        crate::project::scene::authoring::author_scene_atomic(directory.path(), scene_id)?;
    let texture_path = directory.path().join("scene-diffuse.png");
    RgbaImage::from_pixel(1, 1, image::Rgba([48, 96, 192, 255])).save(&texture_path)?;
    let stage = openusd::usd::Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(scene_path.to_string_lossy().as_ref())?;

    let material = Material::define(&stage, "/SceneRoot/Materials/Mat")?;
    let texture = Shader::define(&stage, "/SceneRoot/Materials/Mat/Texture")?;
    texture
        .node_def()
        .create_id_attr()?
        .set(vt::Value::token("UsdUVTexture"))?;
    texture
        .create_input("file", "asset")?
        .set(vt::Value::AssetPath(
            texture_path.to_string_lossy().into_owned().into(),
        ))?;
    texture.create_output("rgb", "float3")?;
    let surface = Shader::define(&stage, "/SceneRoot/Materials/Mat/Surface")?;
    surface
        .node_def()
        .create_id_attr()?
        .set(vt::Value::token("UsdPreviewSurface"))?;
    surface
        .create_input("diffuseColor", "color3f")?
        .set_connections([sdf::path("/SceneRoot/Materials/Mat/Texture.outputs:rgb")?])?;
    surface
        .create_input("roughness", "float")?
        .set(vt::Value::Float(0.35))?;
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
        .set(vt::Value::Vec3fVec(vec![
            openusd::gf::Vec3f::from([0.0, 0.0, 0.0]),
            openusd::gf::Vec3f::from([1.0, 0.0, 0.0]),
            openusd::gf::Vec3f::from([0.0, 1.0, 0.0]),
        ]))?;
    mesh.create_attribute("faceVertexCounts", "int[]")?
        .set(vt::Value::IntVec(vec![3]))?;
    mesh.create_attribute("faceVertexIndices", "int[]")?
        .set(vt::Value::IntVec(vec![0, 1, 2]))?;
    MaterialBindingAPI::apply(&mesh)?.bind(sdf::path("/SceneRoot/Materials/Mat")?)?;
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
        .find(|entry| {
            matches!(
                &entry.kind,
                crate::cache::contract::SceneCacheEntryKind::OwnedPrim { prim_path }
                    if prim_path == "/SceneRoot/Renderable"
            )
        })
        .expect("Scene mesh cache entry");
    let material_ref = entry.material.as_ref().expect("material blob reference");
    let store = SceneCacheStore::new(directory.path()).object_store(scene_id)?;
    let material_bytes = store
        .get(&material_ref.blob_id)?
        .expect("material blob persisted");
    let material: crate::document::runtime::payload::RuntimeMaterialBlob =
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
    assert_eq!(&texture_bytes[..4], b"NTEX");
    // The CPU warmer uses an edge-clamp sampler but stores the USD default
    // black-border intent for a cache reader on capable hardware.
    assert_eq!(texture_bytes[7], 16);
    let texture = crate::document::runtime::payload::RuntimeTexturePayload::decode(&texture_bytes)?;
    assert_eq!(texture.authored_black_axes(), [true, true]);
    assert_eq!(
        texture.clone().into_image(None)?.data,
        Some(vec![48, 96, 192, 255])
    );
    let mut images = bevy::asset::Assets::<bevy::image::Image>::default();
    let mut metadata = usd_bevy::route::material::UsdTextureCache::default();
    let mut previous_bytes = texture_bytes.clone();
    for _ in 0..3 {
        let decoded =
            crate::document::runtime::payload::RuntimeTexturePayload::decode(&previous_bytes)?;
        let axes = decoded.authored_black_axes();
        let handle = images.add(decoded.into_image(None)?);
        metadata.remember_authored_black_border(&handle, axes);
        // Activation invalidates path aliases, not metadata for a still-live image.
        metadata.replace_active_archives([]);
        let encoded = crate::document::runtime::payload::RuntimeTexturePayload::from_image(
            images.get(&handle).unwrap(),
            metadata.authored_black_border(&handle),
        )
        .expect("supported hydrated image")
        .encode()?;
        assert_eq!(encoded, texture_bytes);
        previous_bytes = encoded;
        let _ = images.remove(handle.id());
        metadata.prune_authored_black_borders(&images);
        assert_eq!(metadata.authored_black_border(&handle), [false; 2]);
    }
    Ok(())
}
