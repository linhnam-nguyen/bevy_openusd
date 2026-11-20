//! Derived assembly manifests and content-addressed partition payloads.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use openusd::usd::{PrimPredicate, Stage};
use openusd_schemas::geom::{BBoxCache, Purpose};
use serde::{Deserialize, Serialize};
use usd_model::{BlobId, Bounds3, HashDigest};

use super::blobs::{BlobStore, FilesystemBlobStore};
use crate::project::assets::closure::{source_closure_files, source_closure_fingerprint};

/// Manifest wire schema. The recipe revision tracks derivation policy separately.
const MANIFEST_SCHEMA: u16 = 1;
const PARTITION_SCHEMA: u16 = 1;
const RECIPE_REVISION: u16 = 1;
const ASSEMBLIES_DIRECTORY: &str = "assemblies";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AssemblyManifest {
    schema: u16,
    recipe_revision: u16,
    source_digest: HashDigest,
    root_asset: String,
    sources: BTreeMap<String, BlobId>,
    partitions: BTreeMap<String, BlobId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PartitionPayload {
    schema: u16,
    root: String,
    bounds: Option<Bounds3>,
    occurrences: Vec<Occurrence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Occurrence {
    path: String,
    native_prototype_source: Option<String>,
    native_prototype_member: Option<String>,
    authored_lod: BTreeMap<String, String>,
    point_instances: Vec<PointInstance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PointInstance {
    authored_id: Option<i64>,
    source_row: usize,
    prototype_path: String,
}

pub(super) fn prepare_and_publish(source: &Path, cache_root: &Path) -> Result<HashDigest> {
    let assembly_root = cache_root.join(ASSEMBLIES_DIRECTORY);
    let blobs = FilesystemBlobStore::new(assembly_root.join("objects"))?;
    let manifest = prepare(source, &blobs)?;
    publish(&assembly_root, &blobs, &manifest)
}

fn prepare(source: &Path, blobs: &impl BlobStore) -> Result<AssemblyManifest> {
    let (root, files) = source_closure_files(source)?;
    let mut sources = BTreeMap::new();
    let mut retained = Vec::new();
    let mut root_asset = None;
    let mut digest = blake3::Hasher::new();
    digest.update(b"USDHub source closure fingerprint v2\0");
    for (path, logical) in files {
        let bytes = fs::read(&path)
            .with_context(|| format!("retain USD source asset {}", path.display()))?;
        digest.update(&(logical.len() as u64).to_le_bytes());
        digest.update(logical.as_bytes());
        digest.update(&(bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
        if path == root {
            root_asset = Some(logical.clone());
        }
        ensure!(
            sources.insert(logical, blobs.put(&bytes)?).is_none(),
            "duplicate assembly source name"
        );
        retained.push((path, bytes));
    }
    let source_digest = HashDigest::new(*digest.finalize().as_bytes());
    let stage = usd_bevy::LiveStage::open_authoritative_from_source_closure(&root, retained)?;
    let partitions = build_partitions(&stage)?
        .into_iter()
        .map(|payload| {
            Ok((
                payload.root.clone(),
                blobs.put(&serde_json::to_vec(&payload)?)?,
            ))
        })
        .collect::<Result<_>>()?;
    ensure!(
        source_closure_fingerprint(&root)? == source_digest.to_hex(),
        "USD source closure changed during assembly preparation"
    );
    Ok(AssemblyManifest {
        schema: MANIFEST_SCHEMA,
        recipe_revision: RECIPE_REVISION,
        source_digest,
        root_asset: root_asset.context("assembly source closure has no root asset")?,
        sources,
        partitions,
    })
}

fn build_partitions(stage: &Stage) -> Result<Vec<PartitionPayload>> {
    let mut paths = BTreeSet::new();
    stage.traverse(PrimPredicate::DEFAULT.with_instance_proxies(true), |path| {
        if !path.is_abs_root() {
            paths.insert(path.clone());
        }
    })?;
    let mut prototype_sources = BTreeMap::new();
    for path in &paths {
        if let Some(prototype) = stage.prim(path.clone())?.prototype()? {
            prototype_sources.entry(prototype).or_insert(path.clone());
        }
    }
    let mut partitions = BTreeMap::<String, Vec<Occurrence>>::new();
    for path in paths {
        let prim = stage.prim(path.clone())?;
        let prototype = prim.prototype()?;
        let proxy = prim.prim_in_prototype()?;
        let (native_prototype_source, native_prototype_member) = if let Some(prototype) = prototype
        {
            (
                prototype_sources.get(&prototype).map(ToString::to_string),
                None,
            )
        } else if let Some(proxy) = proxy {
            let (root, member) = proxy
                .path()
                .as_str()
                .trim_start_matches('/')
                .split_once('/')
                .context("native prototype proxy has no member path")?;
            (
                prototype_sources
                    .get(&openusd::sdf::path(&format!("/{root}"))?)
                    .map(ToString::to_string),
                Some(member.to_owned()),
            )
        } else {
            (None, None)
        };
        ensure!(
            native_prototype_member.is_none() || native_prototype_source.is_some(),
            "native proxy {path} has no canonical prototype representative"
        );
        let mut authored_lod = BTreeMap::new();
        for name in prim.variant_sets().names()? {
            if matches!(name.to_ascii_lowercase().as_str(), "lod" | "levelofdetail")
                && let Some(selected) = prim.variant_set(&name).selection()?
            {
                authored_lod.insert(name, selected);
            }
        }
        let point_instances = if prim.type_name()?.as_deref() == Some("PointInstancer") {
            point_instances(stage, &path)?
        } else {
            Vec::new()
        };
        let scope = path
            .as_str()
            .split('/')
            .take(3)
            .collect::<Vec<_>>()
            .join("/");
        partitions.entry(scope).or_default().push(Occurrence {
            path: path.to_string(),
            native_prototype_source,
            native_prototype_member,
            authored_lod,
            point_instances,
        });
    }
    let mut bbox = BBoxCache::new(None, Purpose::ORDERED);
    partitions
        .into_iter()
        .map(|(root, occurrences)| {
            let range = bbox
                .world_bound(&stage.prim(root.clone())?)?
                .aligned_range();
            let bounds = (!range.is_empty()).then(|| Bounds3 {
                min: range.min().into(),
                max: range.max().into(),
            });
            ensure!(
                bounds.as_ref().is_none_or(|bounds| bounds
                    .min
                    .into_iter()
                    .chain(bounds.max)
                    .all(f64::is_finite)),
                "invalid spatial bounds for {root}"
            );
            Ok(PartitionPayload {
                schema: PARTITION_SCHEMA,
                root,
                bounds,
                occurrences,
            })
        })
        .collect()
}

fn point_instances(stage: &Stage, path: &openusd::sdf::Path) -> Result<Vec<PointInstance>> {
    let Some(instances) = usd_bevy::read::geom::read_point_instancer(stage, path)? else {
        return Ok(Vec::new());
    };
    ensure!(
        instances.positions.len() == instances.proto_indices.len(),
        "point instancer {path} has mismatched positions and prototype indices"
    );
    if let Some(ids) = instances.ids.as_ref() {
        ensure!(
            ids.len() == instances.positions.len()
                && ids.iter().collect::<BTreeSet<_>>().len() == ids.len(),
            "point instancer {path} has duplicate or incomplete authored IDs"
        );
    }
    instances
        .proto_indices
        .iter()
        .enumerate()
        .map(|(source_row, &index)| {
            let index = usize::try_from(index)?;
            let prototype = instances.prototypes.get(index).with_context(|| {
                format!("point instancer {path} has invalid prototype index {index}")
            })?;
            Ok(PointInstance {
                authored_id: instances.ids.as_ref().map(|ids| ids[source_row]),
                source_row,
                prototype_path: prototype.to_string(),
            })
        })
        .collect()
}

fn assembly_id(manifest: &AssemblyManifest) -> HashDigest {
    let mut digest = blake3::Hasher::new();
    digest.update(b"Nemeton derived assembly\0");
    digest.update(&manifest.schema.to_le_bytes());
    digest.update(&manifest.recipe_revision.to_le_bytes());
    digest.update(manifest.source_digest.as_bytes());
    HashDigest::new(*digest.finalize().as_bytes())
}

fn validate(blobs: &impl BlobStore, manifest: &AssemblyManifest) -> Result<()> {
    ensure!(
        manifest.schema == MANIFEST_SCHEMA && manifest.recipe_revision == RECIPE_REVISION,
        "unsupported derived assembly schema or recipe"
    );
    ensure!(
        manifest.sources.contains_key(&manifest.root_asset),
        "missing assembly root"
    );
    let mut digest = blake3::Hasher::new();
    digest.update(b"USDHub source closure fingerprint v2\0");
    for (logical, source) in &manifest.sources {
        let bytes = blobs.get(source)?.context("missing retained source blob")?;
        digest.update(&(logical.len() as u64).to_le_bytes());
        digest.update(logical.as_bytes());
        digest.update(&(bytes.len() as u64).to_le_bytes());
        digest.update(&bytes);
    }
    ensure!(
        digest.finalize().as_bytes() == manifest.source_digest.as_bytes(),
        "retained assembly source checksum mismatch"
    );
    let mut paths = BTreeSet::new();
    let mut references = BTreeSet::new();
    for (root, blob) in &manifest.partitions {
        let bytes = blobs.get(blob)?.context("missing derived partition blob")?;
        let partition: PartitionPayload = serde_json::from_slice(&bytes)?;
        ensure!(
            partition.schema == PARTITION_SCHEMA && partition.root == *root,
            "invalid derived partition identity"
        );
        let root = openusd::sdf::path(root)?;
        for occurrence in partition.occurrences {
            let path = openusd::sdf::path(&occurrence.path)?;
            ensure!(
                path.has_prefix(&root) && paths.insert(path),
                "invalid or duplicate partition occurrence"
            );
            references.extend(occurrence.native_prototype_source);
            references.extend(
                occurrence
                    .point_instances
                    .into_iter()
                    .map(|instance| instance.prototype_path),
            );
        }
    }
    for path in references {
        ensure!(
            paths.contains(&openusd::sdf::path(&path)?),
            "unmapped prototype occurrence {path}"
        );
    }
    Ok(())
}

fn publish(root: &Path, blobs: &impl BlobStore, manifest: &AssemblyManifest) -> Result<HashDigest> {
    validate(blobs, manifest)?;
    let id = assembly_id(manifest);
    fs::create_dir_all(root)?;
    let path = root.join(format!("{id}.json"));
    let bytes = serde_json::to_vec(manifest)?;
    if fs::read(&path).is_ok_and(|existing| existing == bytes) {
        return Ok(id);
    }
    ensure!(
        load(root, blobs, id).ok().flatten().is_none(),
        "derived assembly recipe produced differing manifests for {id}"
    );
    let mut staged = tempfile::NamedTempFile::new_in(root)?;
    staged.write_all(&bytes)?;
    staged.as_file().sync_all()?;
    staged.persist(&path)?;
    Ok(id)
}

fn load(root: &Path, blobs: &impl BlobStore, id: HashDigest) -> Result<Option<AssemblyManifest>> {
    let bytes = match fs::read(root.join(format!("{id}.json"))) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let manifest: AssemblyManifest = serde_json::from_slice(&bytes)?;
    ensure!(
        assembly_id(&manifest) == id,
        "derived assembly identity mismatch"
    );
    validate(blobs, &manifest)?;
    Ok(Some(manifest))
}

#[cfg(test)]
#[path = "../../../tests/unit/project/cache/assembly_tests.rs"]
mod tests;
