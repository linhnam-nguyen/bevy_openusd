//! Cache-first Project projection hydration.
//!
//! A cache hit is only an acceleration of the normal stage lifecycle. The
//! caller opens the canonical USD stage first; this module then validates the
//! exact source/profile/config descriptor and prepares renderer-neutral blobs
//! as Bevy assets for the existing projection routes. Any miss or corruption
//! returns control to source projection without replacing LiveStage authority.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::math::{Affine2, Mat2, Vec2};
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::{AlphaMode, Assets, Color, LinearRgba, Resource, World};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use usd_bevy::ProjectionSeed;
use usd_model::BlobId;
use viewport_protocol::{
    RuntimeBlobReference, RuntimeManifest, RuntimePayloadKind, RuntimeProfile,
};

use super::blob_store::{BlobStore, FilesystemBlobStore, OBJECTS_DIRECTORY, get_mesh};
use super::cache::{
    ProjectCacheIdentity, ProjectCacheState, ProjectCacheStore, ProjectCacheTarget,
};
use super::runtime_delivery::{RUNTIME_HIERARCHY_VERSION, RuntimeHierarchyBlob};
use super::runtime_payload::{
    RuntimeAlphaMode, RuntimeMaterialBlob, RuntimeTextureBlob, RuntimeTextureColorSpace,
};

/// The exact cache identity attached to the currently active Project stage.
/// It is backend-only and is never exposed to the frontend or renderer crate.
#[derive(Clone, Debug, Resource)]
pub(crate) struct ActiveProjectCacheContext {
    pub(crate) project_root: PathBuf,
    pub(crate) identity: ProjectCacheIdentity,
}

impl ActiveProjectCacheContext {
    pub(crate) fn new(
        project_root: PathBuf,
        target: ProjectCacheTarget,
        profile: RuntimeProfile,
        config_hash: usd_model::HashDigest,
    ) -> Result<Self> {
        let identity =
            ProjectCacheIdentity::for_project(&project_root, target, profile, config_hash)?;
        Ok(Self {
            project_root,
            identity,
        })
    }
}

/// The application-wide semantic configuration used for runtime cache
/// identity. Keeping this in one helper prevents warm and delivery paths from
/// accidentally publishing different configuration hashes.
pub(crate) fn default_project_cache_config_hash() -> usd_model::HashDigest {
    usd_semantic::SemanticConfig::default().hash()
}

/// Try to hydrate the current Project from its exact ready descriptor.
/// `Ok(false)` is a normal cache miss; errors describe corruption or an
/// unsupported payload and are deliberately handled as source fallback by the
/// stage lifecycle.
pub(crate) fn hydrate_project_cache(
    world: &mut World,
    context: &ActiveProjectCacheContext,
) -> Result<bool> {
    let store = FilesystemBlobStore::new(context.project_root.join(OBJECTS_DIRECTORY))?;
    let descriptor_store = ProjectCacheStore::new(&context.project_root);
    let Some(descriptor) = descriptor_store.load(&context.identity)? else {
        return Ok(false);
    };
    if descriptor.state != ProjectCacheState::Ready {
        return Ok(false);
    }
    let manifest = descriptor
        .runtime
        .as_ref()
        .context("ready Project cache descriptor has no runtime manifest")?;
    validate_manifest_identity(manifest, &context.identity)?;

    let hierarchy_reference = &manifest.hierarchy;
    let hierarchy_bytes = read_blob(
        &store,
        hierarchy_reference,
        RuntimePayloadKind::Hierarchy,
        RUNTIME_HIERARCHY_VERSION,
    )?;
    let hierarchy: RuntimeHierarchyBlob =
        serde_json::from_slice(&hierarchy_bytes).context("decode cached runtime hierarchy")?;
    ensure!(
        hierarchy.version == RUNTIME_HIERARCHY_VERSION,
        "unsupported cached runtime hierarchy version {}",
        hierarchy.version
    );
    ensure!(
        hierarchy.revision == manifest.revision,
        "cached runtime hierarchy revision does not match its manifest"
    );

    let mesh_references = references_by_id(&manifest.meshes)?;
    let material_references = references_by_id(&manifest.materials)?;
    let texture_references = references_by_id(&manifest.textures)?;

    let mut meshes = Vec::new();
    let mut required_meshes = HashSet::new();
    for entity in &hierarchy.entities {
        let Some(geometry) = &entity.geometry else {
            continue;
        };
        if !required_meshes.insert(geometry.blob_id.clone()) {
            continue;
        }
        let reference = mesh_references.get(&geometry.blob_id).with_context(|| {
            format!(
                "cached hierarchy references unknown mesh {}",
                geometry.blob_id
            )
        })?;
        let mesh = get_mesh(&store, &BlobId(geometry.blob_id.clone()))?
            .with_context(|| format!("cached mesh {} is missing", geometry.blob_id))?;
        meshes.push((geometry.blob_id.clone(), mesh, reference.byte_size));
    }

    let mut required_materials = HashSet::new();
    for entity in &hierarchy.entities {
        if let Some(material_id) = &entity.material_blob_id {
            required_materials.insert(material_id.clone());
        }
    }
    let mut texture_payloads = HashMap::new();
    let mut material_payloads = HashMap::new();
    for material_id in &required_materials {
        let reference = material_references.get(material_id).with_context(|| {
            format!("cached hierarchy references unknown material {material_id}")
        })?;
        let bytes = read_blob(
            &store,
            reference,
            RuntimePayloadKind::Material,
            super::runtime_payload::RUNTIME_MATERIAL_VERSION,
        )?;
        let material: RuntimeMaterialBlob = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode cached runtime material {material_id}"))?;
        material.validate()?;
        for texture_id in material_texture_ids(&material).into_iter().flatten() {
            if texture_payloads.contains_key(texture_id) {
                continue;
            }
            let reference = texture_references.get(texture_id).with_context(|| {
                format!("cached material references unknown texture {texture_id}")
            })?;
            let bytes = read_blob(
                &store,
                reference,
                RuntimePayloadKind::Texture,
                super::runtime_payload::RUNTIME_TEXTURE_VERSION,
            )?;
            let texture: RuntimeTextureBlob = serde_json::from_slice(&bytes)
                .with_context(|| format!("decode cached runtime texture {texture_id}"))?;
            texture.validate()?;
            texture_payloads.insert(texture_id.to_owned(), texture);
        }
        material_payloads.insert(material_id.clone(), material);
    }

    let mut texture_assets = HashMap::with_capacity(texture_payloads.len());
    for (texture_id, texture) in texture_payloads {
        let format = match texture.color_space {
            RuntimeTextureColorSpace::Srgb => TextureFormat::Rgba8UnormSrgb,
            RuntimeTextureColorSpace::Linear => TextureFormat::Rgba8Unorm,
        };
        texture_assets.insert(
            texture_id,
            Image::new(
                Extent3d {
                    width: texture.width,
                    height: texture.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                texture.rgba8,
                format,
                RenderAssetUsages::default(),
            ),
        );
    }

    let Some(_) = world.get_resource::<Assets<Mesh>>() else {
        bail!("render mesh assets are unavailable for Project cache hydration");
    };
    let Some(_) = world.get_resource::<Assets<bevy::image::Image>>() else {
        bail!("render image assets are unavailable for Project cache hydration");
    };
    let Some(_) = world.get_resource::<Assets<StandardMaterial>>() else {
        bail!("render material assets are unavailable for Project cache hydration");
    };

    let texture_handles = {
        let mut assets = world.resource_mut::<Assets<bevy::image::Image>>();
        texture_assets
            .into_iter()
            .map(|(id, image)| (id, assets.add(image)))
            .collect::<HashMap<_, _>>()
    };
    let material_handles = {
        let mut assets = world.resource_mut::<Assets<StandardMaterial>>();
        material_payloads
            .into_iter()
            .map(|(id, material)| {
                let hydrated = standard_material(&material, &texture_handles)?;
                Ok((id, assets.add(hydrated)))
            })
            .collect::<Result<HashMap<_, _>>>()?
    };
    let mesh_handles = {
        let mut assets = world.resource_mut::<Assets<Mesh>>();
        meshes
            .into_iter()
            .map(|(id, mesh, _byte_size)| (id, assets.add(mesh)))
            .collect::<HashMap<_, _>>()
    };

    let Some(mut seed) = world.get_resource_mut::<ProjectionSeed>() else {
        bail!("renderer projection seed resource is unavailable");
    };
    for entity in &hierarchy.entities {
        if let Some(geometry) = &entity.geometry {
            let handle = mesh_handles
                .get(&geometry.blob_id)
                .with_context(|| format!("hydrated mesh {} was not allocated", geometry.blob_id))?;
            seed.insert_mesh(
                entity.prim_path.clone(),
                handle.clone(),
                Some((
                    geometry.local_bounds.min.map(|value| value as f32),
                    geometry.local_bounds.max.map(|value| value as f32),
                )),
            );
        }
        if let Some(material_id) = &entity.material_blob_id {
            let handle = material_handles
                .get(material_id)
                .with_context(|| format!("hydrated material {material_id} was not allocated"))?;
            seed.insert_material(entity.prim_path.clone(), handle.clone());
        }
    }
    Ok(true)
}

fn validate_manifest_identity(
    manifest: &RuntimeManifest,
    identity: &ProjectCacheIdentity,
) -> Result<()> {
    manifest
        .validate()
        .map_err(|error| anyhow::anyhow!(error))?;
    ensure!(
        manifest.profile == identity.profile,
        "cached runtime profile mismatch"
    );
    Ok(())
}

fn references_by_id(
    references: &[RuntimeBlobReference],
) -> Result<HashMap<String, RuntimeBlobReference>> {
    references
        .iter()
        .map(|reference| {
            ensure!(
                reference.blob_id == reference.blob_id.trim(),
                "cached runtime blob id contains surrounding whitespace"
            );
            Ok((reference.blob_id.clone(), reference.clone()))
        })
        .collect()
}

fn read_blob(
    store: &FilesystemBlobStore,
    reference: &RuntimeBlobReference,
    expected_kind: RuntimePayloadKind,
    expected_version: u16,
) -> Result<Vec<u8>> {
    ensure!(
        reference.payload_kind == expected_kind,
        "cached runtime blob {} has kind {:?}, expected {:?}",
        reference.blob_id,
        reference.payload_kind,
        expected_kind
    );
    ensure!(
        reference.payload_version == expected_version,
        "cached runtime blob {} has version {}, expected {}",
        reference.blob_id,
        reference.payload_version,
        expected_version
    );
    let id = BlobId(reference.blob_id.clone());
    let bytes = store
        .get(&id)?
        .with_context(|| format!("cached runtime blob {} is missing", reference.blob_id))?;
    ensure!(
        bytes.len() as u64 == reference.byte_size,
        "cached runtime blob {} has unexpected byte size",
        reference.blob_id
    );
    Ok(bytes)
}

fn material_texture_ids(material: &RuntimeMaterialBlob) -> [Option<&str>; 5] {
    [
        material.textures.base_color.as_deref(),
        material.textures.normal.as_deref(),
        material.textures.metallic_roughness.as_deref(),
        material.textures.emissive.as_deref(),
        material.textures.occlusion.as_deref(),
    ]
}

fn standard_material(
    material: &RuntimeMaterialBlob,
    texture_handles: &HashMap<String, bevy::asset::Handle<bevy::image::Image>>,
) -> Result<StandardMaterial> {
    let texture = |id: &Option<String>| {
        id.as_ref()
            .map(|id| {
                texture_handles
                    .get(id)
                    .cloned()
                    .with_context(|| format!("hydrated texture {id} was not allocated"))
            })
            .transpose()
    };
    Ok(StandardMaterial {
        base_color: Color::srgba(
            material.base_color[0],
            material.base_color[1],
            material.base_color[2],
            material.base_color[3],
        ),
        base_color_texture: texture(&material.textures.base_color)?,
        normal_map_texture: texture(&material.textures.normal)?,
        metallic_roughness_texture: texture(&material.textures.metallic_roughness)?,
        emissive: LinearRgba::new(
            material.emissive[0],
            material.emissive[1],
            material.emissive[2],
            material.emissive[3],
        ),
        emissive_texture: texture(&material.textures.emissive)?,
        metallic: material.metallic,
        perceptual_roughness: material.perceptual_roughness,
        ior: material.ior,
        alpha_mode: alpha_mode(material.alpha_mode),
        double_sided: material.double_sided,
        unlit: material.unlit,
        uv_transform: Affine2 {
            matrix2: Mat2::from_cols(
                Vec2::new(material.uv_transform[0][0], material.uv_transform[0][1]),
                Vec2::new(material.uv_transform[1][0], material.uv_transform[1][1]),
            ),
            translation: Vec2::new(material.uv_transform[2][0], material.uv_transform[2][1]),
        },
        occlusion_texture: texture(&material.textures.occlusion)?,
        ..Default::default()
    })
}

fn alpha_mode(mode: RuntimeAlphaMode) -> AlphaMode {
    match mode {
        RuntimeAlphaMode::Opaque => AlphaMode::Opaque,
        RuntimeAlphaMode::Mask { cutoff } => AlphaMode::Mask(cutoff),
        RuntimeAlphaMode::Blend => AlphaMode::Blend,
        RuntimeAlphaMode::Premultiplied => AlphaMode::Premultiplied,
        RuntimeAlphaMode::AlphaToCoverage => AlphaMode::AlphaToCoverage,
        RuntimeAlphaMode::Add => AlphaMode::Add,
        RuntimeAlphaMode::Multiply => AlphaMode::Multiply,
    }
}

#[cfg(test)]
#[path = "cache_benchmark_tests.rs"]
mod benchmark_tests;
#[cfg(test)]
#[path = "cache_hydration_tests.rs"]
mod tests;
