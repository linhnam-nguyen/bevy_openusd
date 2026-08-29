use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use tempfile::tempdir;
use usd_model::{Bounds3, HashDigest, TransformSignature};

use super::*;
use crate::project::blob_store::{BlobStore, PreparedMeshBlob, prepare_mesh_payload};
use crate::project::cache::{ProjectCacheDescriptor, ProjectCacheTarget};
use crate::project::runtime_delivery::{RuntimeHierarchyEntity, RuntimeHierarchyGeometry};
use crate::project::runtime_payload::{
    RUNTIME_MATERIAL_VERSION, RUNTIME_TEXTURE_VERSION, RuntimeMaterialTextures,
};

fn digest(value: u8) -> HashDigest {
    HashDigest::new([value; HashDigest::BYTE_LEN])
}

fn fixture() -> Result<(
    tempfile::TempDir,
    ActiveProjectCacheContext,
    RuntimeManifest,
    BlobId,
    BlobId,
)> {
    let project = tempdir()?;
    usd_git::Repository::init(project.path())?;
    let store = FilesystemBlobStore::new(project.path().join(OBJECTS_DIRECTORY))?;

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
    let PreparedMeshBlob {
        blob_id: mesh_id,
        bytes: mesh_bytes,
    } = prepare_mesh_payload(&mesh)?;
    assert_eq!(store.put(&mesh_bytes)?, mesh_id);

    let texture = RuntimeTextureBlob {
        version: RUNTIME_TEXTURE_VERSION,
        color_space: RuntimeTextureColorSpace::Srgb,
        width: 1,
        height: 1,
        rgba8: vec![255, 0, 0, 255],
    };
    let texture_bytes = serde_json::to_vec(&texture)?;
    let texture_id = store.put(&texture_bytes)?;

    let material = RuntimeMaterialBlob {
        version: RUNTIME_MATERIAL_VERSION,
        base_color: [0.2, 0.3, 0.4, 1.0],
        emissive: [0.0, 0.0, 0.0, 1.0],
        perceptual_roughness: 0.4,
        metallic: 0.1,
        ior: 1.5,
        alpha_mode: RuntimeAlphaMode::Opaque,
        double_sided: true,
        unlit: false,
        uv_transform: [[1.0, 0.0], [0.0, 1.0], [0.0, 0.0]],
        textures: RuntimeMaterialTextures {
            base_color: Some(texture_id.0.clone()),
            ..Default::default()
        },
    };
    let material_bytes = serde_json::to_vec(&material)?;
    let material_id = store.put(&material_bytes)?;

    let hierarchy = RuntimeHierarchyBlob {
        version: RUNTIME_HIERARCHY_VERSION,
        revision: "cache-revision".to_owned(),
        entities: vec![RuntimeHierarchyEntity {
            entity_key: "/World/Triangle".to_owned(),
            prim_path: "/World/Triangle".to_owned(),
            transform: TransformSignature {
                translation_mm: [0; 3],
                rotation_quantized: [0, 0, 0, 10_000],
                scale_quantized: [10_000; 3],
                hash: digest(1),
            },
            geometry: Some(RuntimeHierarchyGeometry {
                blob_id: mesh_id.0.clone(),
                vertex_count: 3,
                index_count: 3,
                local_bounds: Bounds3 {
                    min: [0.0; 3],
                    max: [1.0; 3],
                },
            }),
            material_blob_id: Some(material_id.0.clone()),
        }],
    };
    let hierarchy_bytes = serde_json::to_vec(&hierarchy)?;
    let hierarchy_id = store.put(&hierarchy_bytes)?;
    let manifest = RuntimeManifest {
        revision: "cache-revision".to_owned(),
        profile: RuntimeProfile::NativeMedium,
        hierarchy: RuntimeBlobReference {
            blob_id: hierarchy_id.0.clone(),
            payload_kind: RuntimePayloadKind::Hierarchy,
            payload_version: RUNTIME_HIERARCHY_VERSION,
            byte_size: hierarchy_bytes.len() as u64,
        },
        meshes: vec![RuntimeBlobReference {
            blob_id: mesh_id.0.clone(),
            payload_kind: RuntimePayloadKind::Mesh,
            payload_version: 1,
            byte_size: mesh_bytes.len() as u64,
        }],
        materials: vec![RuntimeBlobReference {
            blob_id: material_id.0.clone(),
            payload_kind: RuntimePayloadKind::Material,
            payload_version: RUNTIME_MATERIAL_VERSION,
            byte_size: material_bytes.len() as u64,
        }],
        textures: vec![RuntimeBlobReference {
            blob_id: texture_id.0.clone(),
            payload_kind: RuntimePayloadKind::Texture,
            payload_version: RUNTIME_TEXTURE_VERSION,
            byte_size: texture_bytes.len() as u64,
        }],
    };
    let target = ProjectCacheTarget::ProjectRoot;
    let identity = ProjectCacheIdentity::for_project(
        project.path(),
        target.clone(),
        RuntimeProfile::NativeMedium,
        default_project_cache_config_hash(),
    )?;
    ProjectCacheStore::new(project.path()).publish(&ProjectCacheDescriptor::new(
        identity.clone(),
        ProjectCacheState::Ready,
        Some(manifest.clone()),
    )?)?;
    let project_root = project.path().to_path_buf();
    Ok((
        project,
        ActiveProjectCacheContext {
            project_root,
            identity,
        },
        manifest,
        mesh_id,
        material_id,
    ))
}

#[test]
fn ready_cache_hydrates_mesh_material_texture_and_seed_maps() -> Result<()> {
    let (project, context, _manifest, _mesh_id, _material_id) = fixture()?;
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<Image>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<ProjectionSeed>();

    assert!(hydrate_project_cache(&mut world, &context)?);
    assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
    assert_eq!(world.resource::<Assets<Image>>().len(), 1);
    assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 1);
    let material = world
        .resource::<Assets<StandardMaterial>>()
        .iter()
        .next()
        .map(|(_, material)| material)
        .expect("hydrated material");
    assert!(material.base_color_texture.is_some());
    assert_eq!(world.resource::<ProjectionSeed>().pending_meshes(), 1);
    assert_eq!(world.resource::<ProjectionSeed>().pending_materials(), 1);
    drop(project);
    Ok(())
}

#[test]
fn corrupt_cache_is_rejected_before_seeds_are_published() -> Result<()> {
    let (project, context, manifest, mesh_id, _material_id) = fixture()?;
    let object = project
        .path()
        .join(OBJECTS_DIRECTORY)
        .join(&mesh_id.0[..2])
        .join(format!("{}.blob", mesh_id.0));
    std::fs::write(object, b"corrupt")?;

    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<Image>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<ProjectionSeed>();
    let error = hydrate_project_cache(&mut world, &context).expect_err("corrupt cache");
    assert!(error.to_string().contains("digest mismatch"));
    assert_eq!(world.resource::<ProjectionSeed>().pending_meshes(), 0);
    assert_eq!(world.resource::<ProjectionSeed>().pending_materials(), 0);
    assert_eq!(manifest.revision, "cache-revision");
    Ok(())
}

#[test]
fn missing_ready_descriptor_is_a_normal_cache_miss() -> Result<()> {
    let project = tempdir()?;
    usd_git::Repository::init(project.path())?;
    let context = ActiveProjectCacheContext::new(
        project.path().to_path_buf(),
        ProjectCacheTarget::ProjectRoot,
        RuntimeProfile::NativeMedium,
        default_project_cache_config_hash(),
    )?;

    let mut world = World::new();
    assert!(!hydrate_project_cache(&mut world, &context)?);
    Ok(())
}

#[test]
fn persistent_cache_timing_evidence_records_repeated_hydration_without_thresholds() -> Result<()> {
    let (project, context, _manifest, _mesh_id, _material_id) = fixture()?;
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<Image>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<ProjectionSeed>();

    let cold_start = Instant::now();
    assert!(hydrate_project_cache(&mut world, &context)?);
    let cold_ms = cold_start.elapsed().as_secs_f64() * 1_000.0;

    let repeated_start = Instant::now();
    for _ in 0..3 {
        assert!(hydrate_project_cache(&mut world, &context)?);
    }
    let repeated_ms = repeated_start.elapsed().as_secs_f64() * 1_000.0;
    eprintln!(
        "[owner-review-3-c8] persistent-cache hydration: cold_ms={cold_ms:.3}, repeated_ms={repeated_ms:.3}, meshes={}, materials={}, textures={}",
        world.resource::<Assets<Mesh>>().len(),
        world.resource::<Assets<StandardMaterial>>().len(),
        world.resource::<Assets<Image>>().len(),
    );
    drop(project);
    Ok(())
}
