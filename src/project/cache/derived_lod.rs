//! Opt-in, content-addressed *visual* LOD candidates for static triangle meshes.
//!
//! The caller must prove eligibility from the authoritative USD source:
//! no animated/skinned mesh, topology-critical/precision-required object, or
//! unsupported material binding. This function never substitutes the original
//! in engineering queries and never modifies canonical USD layers.
use std::collections::{BTreeMap, BTreeSet};

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use openusd::usd::Prim;
use usd_project::SceneId;

use crate::project::storage::ProjectStorageLayout;
use meshopt::{SimplifyOptions, VertexDataAdapter};
use serde::{Deserialize, Serialize};
use usd_model::BlobId;

use super::blobs::{BlobStore, MeshBlob};
use super::contract::{SceneCacheAddress, SceneCacheEntry, SceneCacheOccurrence};
use super::mesh_codec;

const DERIVED_LOD_SCHEMA: u16 = 1;
const MAX_CATALOGUE_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_CANDIDATE_BYTES: usize = 8 * 1024 * 1024;
const DERIVED_RECIPE: &str = "meshopt-rs-0.7.0-border-safe-v1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LODSettings {
    pub(crate) target_ratio: f32,
    pub(crate) max_relative_error: f32,
}

impl LODSettings {
    fn validate(self) -> Result<()> {
        ensure!(
            self.target_ratio.is_finite() && (0.05..1.0).contains(&self.target_ratio),
            "target_ratio must be [0.05,1.0)"
        );
        ensure!(
            self.max_relative_error.is_finite() && (0.0..=0.1).contains(&self.max_relative_error),
            "relative error must be [0,0.1]"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct DerivedLodManifest {
    pub(crate) schema: u16,
    pub(crate) recipe: String,
    pub(crate) source_anchor: String,
    pub(crate) source_blob: BlobId,
    pub(crate) source_digest: String,
    pub(crate) visual_blob: BlobId,
    pub(crate) ratio: f32,
    pub(crate) allowed_error: f32,
    pub(crate) measured_error: f32,
    pub(crate) original_triangles: usize,
    pub(crate) derived_triangles: usize,
    pub(crate) original_bytes: usize,
    pub(crate) derived_bytes: usize,
    /// The reduced index buffer points to original vertex rows; no source
    /// vertex data, UVs, colors or normals are rewritten or reordered.
    pub(crate) source_vertex_rows: Vec<u32>,
    pub(crate) original_bounds: ([f32; 3], [f32; 3]),
    pub(crate) derived_bounds: ([f32; 3], [f32; 3]),
}

/// A *rejected* candidate is not a degraded success; callers must retain the
/// untouched source blob as the full-resolution visual and exact fallback.
pub(crate) fn generate_static_candidate(
    store: &impl BlobStore,
    source_anchor: &str,
    source_blob: &BlobId,
    source_bytes: &[u8],
    settings: LODSettings,
) -> Result<DerivedLodManifest> {
    settings.validate()?;
    ensure!(
        source_anchor.starts_with('/') && !source_anchor.contains('{'),
        "invalid source prim identity"
    );
    ensure!(
        blake3::hash(source_bytes).to_hex().to_string() == source_blob.0,
        "source bytes/digest do not match"
    );
    let mesh = mesh_codec::decode(source_bytes)?;
    mesh.validate()?;
    ensure!(
        mesh.indices.len() >= 12 && mesh.indices.len() % 3 == 0,
        "LOD requires >=4 source triangles"
    );
    ensure!(
        mesh.positions.iter().flatten().all(|p| p.is_finite()),
        "nonfinite engineering vertices"
    );

    let boundary = verified_boundary_edges(&mesh)?;
    let seams = seam_vertices(&mesh);
    // The library retains the exact original vertex rows. Without explicit
    // attribute streams, post-validate UV/normal/color seams and the topology
    // boundary; never silently turn a rejected topology into a lower LOD.
    let positions = mesh
        .positions
        .iter()
        .flat_map(|position| {
            position
                .iter()
                .flat_map(|component| component.to_le_bytes())
        })
        .collect::<Vec<u8>>();
    let adapter = VertexDataAdapter::new(&positions, 12, 0)?;
    let desired = ((mesh.indices.len() as f32 * settings.target_ratio) as usize / 3 * 3)
        .max(3)
        .min(mesh.indices.len() - 3);
    let mut measured_error = 0.0f32;
    let simplified = meshopt::simplify(
        &mesh.indices,
        &adapter,
        desired,
        settings.max_relative_error,
        SimplifyOptions::LockBorder,
        Some(&mut measured_error),
    );
    ensure!(
        simplified.len() < mesh.indices.len() && simplified.len() >= 3 && simplified.len() % 3 == 0,
        "topology cannot be safely simplified"
    );
    ensure!(
        measured_error.is_finite() && measured_error <= settings.max_relative_error,
        "meshopt error exceeds declared tolerance"
    );
    let used = simplified.iter().copied().collect::<BTreeSet<_>>();
    let mut simplified_mesh = MeshBlob {
        indices: simplified,
        ..mesh.clone()
    };
    ensure!(
        boundary == verified_boundary_edges(&simplified_mesh)?,
        "simplification changed a boundary/hole edge"
    );
    ensure!(
        seams.is_subset(&used),
        "simplification removed a UV/normal/color seam vertex"
    );
    for triangle in simplified_mesh.indices.chunks_exact(3) {
        let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
        ensure!(
            a != b && a != c && b != c,
            "simplification created degenerate triangle"
        );
    }

    let original_bounds = bounds(&mesh.positions, &mesh.indices)?;
    let derived_bounds = bounds(&mesh.positions, &simplified_mesh.indices)?;
    let source_diagonal = original_bounds
        .0
        .into_iter()
        .zip(original_bounds.1)
        .map(|(lo, hi)| (hi - lo).powi(2))
        .sum::<f32>()
        .sqrt()
        .max(1e-6);
    let epsilon = source_diagonal * settings.max_relative_error.max(1e-6);
    for axis in 0..3 {
        ensure!(
            (original_bounds.0[axis] - derived_bounds.0[axis]).abs() <= epsilon
                && (original_bounds.1[axis] - derived_bounds.1[axis]).abs() <= epsilon,
            "LOD changed source bounds beyond error tolerance"
        );
    }
    let rows: Vec<_> = used.into_iter().collect();
    let remap: BTreeMap<_, _> = rows
        .iter()
        .copied()
        .enumerate()
        .map(|(slot, row)| (row, slot as u32))
        .collect();
    for index in &mut simplified_mesh.indices {
        *index = remap[index];
    }
    fn compact<T: Copy>(values: &[T], rows: &[u32]) -> Vec<T> {
        rows.iter().map(|&row| values[row as usize]).collect()
    }
    simplified_mesh.positions = compact(&mesh.positions, &rows);
    simplified_mesh.normals = mesh.normals.as_ref().map(|values| compact(values, &rows));
    simplified_mesh.uvs = mesh.uvs.as_ref().map(|values| compact(values, &rows));
    simplified_mesh.colors = mesh.colors.as_ref().map(|values| compact(values, &rows));
    let bytes = mesh_codec::encode(&simplified_mesh)?;
    ensure!(
        bytes.len() < source_bytes.len(),
        "derived object does not reduce render bytes"
    );
    let visual_blob = store
        .put(&bytes)
        .context("publish derived render-only mesh")?;
    ensure!(
        store.get(&visual_blob)?.as_deref() == Some(bytes.as_slice()),
        "derived object failed readback"
    );
    Ok(DerivedLodManifest {
        schema: DERIVED_LOD_SCHEMA,
        recipe: DERIVED_RECIPE.to_owned(),
        source_anchor: source_anchor.to_owned(),
        source_blob: source_blob.clone(),
        source_digest: blake3::hash(source_bytes).to_hex().to_string(),
        visual_blob,
        ratio: settings.target_ratio,
        allowed_error: settings.max_relative_error,
        measured_error,
        original_triangles: mesh.indices.len() / 3,
        derived_triangles: simplified_mesh.indices.len() / 3,
        original_bytes: source_bytes.len(),
        derived_bytes: bytes.len(),
        source_vertex_rows: rows,
        original_bounds,
        derived_bounds,
    })
}

fn bounds(vertices: &[[f32; 3]], indices: &[u32]) -> Result<([f32; 3], [f32; 3])> {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for &index in indices {
        let vertex = vertices
            .get(index as usize)
            .context("out-of-range vertex index")?;
        for axis in 0..3 {
            lo[axis] = lo[axis].min(vertex[axis]);
            hi[axis] = hi[axis].max(vertex[axis]);
        }
    }
    ensure!(
        lo.iter().chain(&hi).all(|v| v.is_finite()),
        "invalid geometry bounds"
    );
    Ok((lo, hi))
}

/// A manifold edge has one or two adjacent faces. Edges seen once are the
/// exact boundary/hole closure, which the output must not remove.
fn verified_boundary_edges(mesh: &MeshBlob) -> Result<BTreeSet<(u32, u32)>> {
    let mut edges = BTreeMap::<(u32, u32), u8>::new();
    for triangle in mesh.indices.chunks_exact(3) {
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            ensure!(a != b, "degenerate source face");
            let edge = (a.min(b), a.max(b));
            let value = edges.entry(edge).or_default();
            *value = value.saturating_add(1);
            ensure!(*value <= 2, "nonmanifold triangle boundary");
        }
    }
    Ok(edges
        .into_iter()
        .filter(|(_, count)| *count == 1)
        .map(|(edge, _)| edge)
        .collect())
}

fn seam_vertices(mesh: &MeshBlob) -> BTreeSet<u32> {
    // Vertex rows with shared geometric position but different attributes
    // represent a discontinuity: they must survive intact in the output.
    let mut groups = BTreeMap::<[u32; 3], Vec<usize>>::new();
    for (index, position) in mesh.positions.iter().enumerate() {
        groups
            .entry(position.map(f32::to_bits))
            .or_default()
            .push(index);
    }
    let mut seams = BTreeSet::new();
    for group in groups.into_values() {
        if group.len() < 2 {
            continue;
        }
        let value = |index: usize| {
            (
                mesh.normals
                    .as_ref()
                    .and_then(|n| n.get(index))
                    .map(|v| v.map(f32::to_bits)),
                mesh.uvs
                    .as_ref()
                    .and_then(|u| u.get(index))
                    .map(|v| v.map(f32::to_bits)),
                mesh.colors
                    .as_ref()
                    .and_then(|c| c.get(index))
                    .map(|v| v.map(f32::to_bits)),
            )
        };
        if group
            .iter()
            .skip(1)
            .any(|&index| value(index) != value(group[0]))
        {
            seams.extend(group.into_iter().map(|index| index as u32));
        }
    }
    seams
}

pub(crate) fn enabled() -> bool {
    std::env::var_os("NEMETON_GENERATED_LOD").is_some_and(|value| value == "1")
}

pub(crate) fn source_eligible(prim: &Prim) -> Result<bool> {
    // Generation needs explicit source authority for this particular object.
    if prim
        .type_name()?
        .as_ref()
        .is_none_or(|name| name.as_str() != "Mesh")
        || prim
            .attribute("userProperties:nemeton:allowGeneratedLod")
            .get::<bool>()?
            != Some(true)
        || prim.is_instance_proxy()?
        || !prim.children()?.is_empty()
        || !prim.variant_sets().names()?.is_empty()
    {
        return Ok(false);
    }
    let mut current = Some(prim.clone());
    while let Some(prim) = current {
        if !prim.variant_sets().names()?.is_empty()
            || prim
                .type_name()?
                .is_some_and(|kind| kind.as_str() == "SkelRoot")
            || prim
                .authored_api_schemas()?
                .iter()
                .any(|schema| schema.as_str().starts_with("SkelBindingAPI"))
            || prim
                .authored_property_names()?
                .iter()
                .any(|name| name.as_str().starts_with("skel:"))
        {
            return Ok(false);
        }
        for attribute in prim.authored_attributes()? {
            if attribute.num_time_samples()? != 0 {
                return Ok(false);
            }
        }
        current = prim.parent().filter(|parent| !parent.path().is_abs_root());
    }
    Ok(true)
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct GeneratedLodCatalogue {
    schema: u16,
    recipe: String,
    generation: u64,
    pub(crate) candidates: BTreeMap<String, DerivedLodManifest>,
}

fn catalogue_path(project_root: &Path, scene: SceneId, generation: u64) -> PathBuf {
    ProjectStorageLayout::new(project_root)
        .scene_cache_dir(scene)
        .join(format!(
            "generated-lod-v{DERIVED_LOD_SCHEMA}-{generation}.json"
        ))
}

pub(crate) fn prepare_scene_lods(
    project_root: &Path,
    scene: SceneId,
    generation: u64,
    eligible: &std::collections::HashSet<String>,
    store: &impl BlobStore,
    entries: &[SceneCacheEntry],
) -> Result<()> {
    let mut candidates = BTreeMap::new();
    for entry in entries {
        let (
            SceneCacheAddress {
                occurrence: SceneCacheOccurrence::PrimPath(path),
                ..
            },
            Some(geometry),
        ) = (&entry.address, &entry.geometry)
        else {
            continue;
        };
        if entry.bim_enabled || entry.animation.is_some() || !eligible.contains(path) {
            continue;
        }
        let started = std::time::Instant::now();
        let result = (|| -> Result<DerivedLodManifest> {
            let original = store
                .get_bounded(&geometry.blob_id, MAX_CANDIDATE_BYTES)?
                .context("source render mesh is missing")?;
            generate_static_candidate(
                store,
                path,
                &geometry.blob_id,
                &original,
                LODSettings {
                    target_ratio: 0.5,
                    max_relative_error: 0.02,
                },
            )
        })();
        match result {
            Ok(manifest) => {
                log::info!(
                    "generated visual LOD for {path}: {} to {} bytes in {:?}",
                    manifest.original_bytes,
                    manifest.derived_bytes,
                    started.elapsed()
                );
                candidates.insert(path.clone(), manifest);
            }
            Err(error) => log::debug!("kept full geometry for {path}: {error:#}"),
        }
    }

    let catalogue = GeneratedLodCatalogue {
        schema: DERIVED_LOD_SCHEMA,
        recipe: DERIVED_RECIPE.to_owned(),
        generation,
        candidates,
    };
    let path = catalogue_path(project_root, scene, generation);
    let bytes = serde_json::to_vec(&catalogue)?;
    ensure!(
        bytes.len() as u64 <= MAX_CATALOGUE_BYTES,
        "generated LOD catalogue exceeded its size limit"
    );
    let folder = path
        .parent()
        .context("generated LOD catalogue requires a cache directory")?;
    std::fs::create_dir_all(folder)?;
    let mut staged = tempfile::NamedTempFile::new_in(folder)?;
    staged.write_all(&bytes)?;
    match staged.persist_noclobber(&path) {
        Ok(_) => Ok(()),
        Err(error)
            if error.error.kind() == std::io::ErrorKind::AlreadyExists
                && std::fs::read(&path).ok().as_deref() == Some(bytes.as_slice()) =>
        {
            Ok(())
        }
        Err(error) => Err(error.error.into()),
    }
}

pub(crate) fn read_scene_lods(
    project_root: &Path,
    scene: SceneId,
    generation: u64,
) -> Option<GeneratedLodCatalogue> {
    let file = std::fs::File::open(catalogue_path(project_root, scene, generation)).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_CATALOGUE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CATALOGUE_BYTES {
        return None;
    }
    let catalogue: GeneratedLodCatalogue = serde_json::from_slice(&bytes).ok()?;
    (catalogue.schema == DERIVED_LOD_SCHEMA
        && catalogue.recipe == DERIVED_RECIPE
        && catalogue.generation == generation)
        .then_some(catalogue)
}

#[cfg(test)]
#[path = "../../../tests/unit/project/cache/derived_lod_tests.rs"]
mod tests;
