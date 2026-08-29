//! Content-addressed storage for derived runtime payloads.
//!
//! Large render payloads do not belong in ordinary project metadata rows. The
//! first backend is deliberately local and boring: objects are keyed by their
//! BLAKE3 digest and written atomically under an `objects/<shard>/` directory.
//! A future remote backend can implement [`BlobStore`] without changing the
//! payload references stored in the project model.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use serde::{Deserialize, Serialize};
use usd_model::BlobId;

use super::storage::CACHE_OBJECTS_RELATIVE_PATH;

pub(crate) const OBJECTS_DIRECTORY: &str = CACHE_OBJECTS_RELATIVE_PATH;
/// Storage interface for content-addressed derived payloads.
pub(crate) trait BlobStore {
    fn put(&self, bytes: &[u8]) -> Result<BlobId>;
    fn get(&self, id: &BlobId) -> Result<Option<Vec<u8>>>;
    fn contains(&self, id: &BlobId) -> Result<bool>;
}

/// Immutable mesh payload prepared on the Bevy owner thread and persisted by
/// the runtime-delivery worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedMeshBlob {
    pub(crate) blob_id: BlobId,
    pub(crate) bytes: Vec<u8>,
}

/// Local filesystem implementation of [`BlobStore`].
///
/// The supplied path is the `objects` directory itself. An object with digest
/// `abcdef...` is stored as `<root>/ab/abcdef....blob`.
#[derive(Clone, Debug)]
pub(crate) struct FilesystemBlobStore {
    root: PathBuf,
}

impl FilesystemBlobStore {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if root.as_os_str().is_empty() {
            bail!("blob store root must not be empty");
        }
        Ok(Self { root })
    }

    fn object_path(&self, id: &BlobId) -> Result<PathBuf> {
        validate_blob_id(id)?;
        Ok(self.root.join(&id.0[..2]).join(format!("{}.blob", id.0)))
    }

    fn temporary_path(&self, id: &BlobId) -> Result<PathBuf> {
        validate_blob_id(id)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Ok(self.root.join(format!(
            ".{}.{}.{}.tmp.blob",
            id.0,
            std::process::id(),
            nonce
        )))
    }
}

impl BlobStore for FilesystemBlobStore {
    fn put(&self, bytes: &[u8]) -> Result<BlobId> {
        let id = BlobId(blake3::hash(bytes).to_hex().to_string());
        let destination = self.object_path(&id)?;
        if destination.is_file() {
            // A repeated put is the normal fast path. If an object was
            // externally corrupted, continue below and repair it atomically.
            if let Ok(existing) = fs::read(&destination)
                && existing == bytes
            {
                return Ok(id);
            }
        }

        let parent = destination
            .parent()
            .ok_or_else(|| anyhow!("blob object has no parent directory"))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("create blob directory {}", parent.display()))?;

        let temporary = self.temporary_path(&id)?;
        let write_result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .with_context(|| format!("create temporary blob {}", temporary.display()))?;
            file.write_all(bytes)
                .with_context(|| format!("write temporary blob {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("sync temporary blob {}", temporary.display()))?;
            fs::rename(&temporary, &destination).with_context(|| {
                format!(
                    "publish blob {} as {}",
                    temporary.display(),
                    destination.display()
                )
            })?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result?;

        Ok(id)
    }

    fn get(&self, id: &BlobId) -> Result<Option<Vec<u8>>> {
        let path = self.object_path(id)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("read blob {}", path.display()));
            }
        };
        let actual = blake3::hash(&bytes).to_hex().to_string();
        if actual != id.0 {
            bail!(
                "blob digest mismatch for {}: stored bytes hash to {}",
                id.0,
                actual
            );
        }
        Ok(Some(bytes))
    }

    fn contains(&self, id: &BlobId) -> Result<bool> {
        // Validate the digest as well as existence. This makes corruption
        // observable to callers instead of reporting a bad object as usable.
        Ok(self.get(id)?.is_some())
    }
}

fn validate_blob_id(id: &BlobId) -> Result<()> {
    if id.0.len() != 64
        || !id
            .0
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    {
        bail!(
            "invalid blob id {:?}: expected 64 lowercase hexadecimal characters",
            id.0
        );
    }
    Ok(())
}

/// Versioned first serializer for projected triangle-list Bevy meshes.
///
/// JSON is intentionally a simple milestone-17 format. The blob boundary is
/// stable, so it can later be replaced by a compact binary codec without
/// changing object addressing or the project metadata contract.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct MeshBlob {
    pub(crate) version: u16,
    pub(crate) positions: Vec<[f32; 3]>,
    pub(crate) normals: Option<Vec<[f32; 3]>>,
    pub(crate) uvs: Option<Vec<[f32; 2]>>,
    pub(crate) colors: Option<Vec<[f32; 4]>>,
    pub(crate) indices: Vec<u32>,
}

impl MeshBlob {
    pub(crate) fn from_bevy_mesh(mesh: &Mesh) -> Result<Self> {
        if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
            bail!(
                "mesh blob serializer supports triangle lists, got {:?}",
                mesh.primitive_topology()
            );
        }

        let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
            Some(other) => bail!("mesh positions use unsupported attribute {:?}", other),
            None => bail!("mesh blob requires position data"),
        };
        let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(values)) => Some(values.clone()),
            Some(other) => bail!("mesh normals use unsupported attribute {:?}", other),
            None => None,
        };
        let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
            Some(VertexAttributeValues::Float32x2(values)) => Some(values.clone()),
            Some(other) => bail!("mesh UVs use unsupported attribute {:?}", other),
            None => None,
        };
        let colors = match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(values)) => Some(values.clone()),
            Some(other) => bail!("mesh colors use unsupported attribute {:?}", other),
            None => None,
        };
        let indices = match mesh.indices() {
            Some(Indices::U16(values)) => values.iter().map(|&value| value as u32).collect(),
            Some(Indices::U32(values)) => values.clone(),
            None => bail!("mesh blob requires indexed geometry"),
        };
        if indices.len() % 3 != 0 {
            bail!("triangle-list mesh index count must be divisible by three");
        }

        validate_attribute_len("normals", normals.as_deref(), positions.len())?;
        validate_attribute_len("UVs", uvs.as_deref(), positions.len())?;
        validate_attribute_len("colors", colors.as_deref(), positions.len())?;

        Ok(Self {
            version: 1,
            positions,
            normals,
            uvs,
            colors,
            indices,
        })
    }

    pub(crate) fn to_bevy_mesh(&self) -> Result<Mesh> {
        if self.version != 1 {
            bail!("unsupported mesh blob version {}", self.version);
        }
        if !self.indices.len().is_multiple_of(3) {
            bail!("triangle-list mesh index count must be divisible by three");
        }
        validate_attribute_len("normals", self.normals.as_deref(), self.positions.len())?;
        validate_attribute_len("UVs", self.uvs.as_deref(), self.positions.len())?;
        validate_attribute_len("colors", self.colors.as_deref(), self.positions.len())?;

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions.clone());
        if let Some(normals) = &self.normals {
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.clone());
        }
        if let Some(uvs) = &self.uvs {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.clone());
        }
        if let Some(colors) = &self.colors {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.clone());
        }
        mesh.insert_indices(Indices::U32(self.indices.clone()));
        Ok(mesh)
    }

    fn encode(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).context("encode mesh blob")
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).context("decode mesh blob")
    }
}

fn validate_attribute_len<T>(name: &str, values: Option<&[T]>, expected: usize) -> Result<()> {
    if let Some(values) = values
        && values.len() != expected
    {
        bail!(
            "mesh {name} attribute has {} values, expected {expected}",
            values.len()
        );
    }
    Ok(())
}

pub(crate) fn put_mesh(store: &impl BlobStore, mesh: &Mesh) -> Result<BlobId> {
    let prepared = prepare_mesh_payload(mesh)?;
    let stored = store.put(&prepared.bytes)?;
    if stored != prepared.blob_id {
        bail!(
            "BlobStore returned digest {} for prepared mesh {}, refusing mismatched payload",
            stored.0,
            prepared.blob_id.0
        );
    }
    Ok(stored)
}

/// Encode a projected mesh without touching the filesystem or another
/// blocking data-plane resource.
pub(crate) fn prepare_mesh_payload(mesh: &Mesh) -> Result<PreparedMeshBlob> {
    let blob = MeshBlob::from_bevy_mesh(mesh)?;
    let bytes = blob.encode()?;
    let blob_id = BlobId(blake3::hash(&bytes).to_hex().to_string());
    Ok(PreparedMeshBlob { blob_id, bytes })
}

pub(crate) fn get_mesh(store: &impl BlobStore, id: &BlobId) -> Result<Option<Mesh>> {
    let Some(bytes) = store.get(id)? else {
        return Ok(None);
    };
    Ok(Some(MeshBlob::decode(&bytes)?.to_bevy_mesh()?))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    #[test]
    fn filesystem_store_is_content_addressed_and_idempotent() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = FilesystemBlobStore::new(directory.path().join(".usdhub/cache/objects"))?;

        let first = store.put(b"mesh-payload")?;
        let second = store.put(b"mesh-payload")?;
        let other = store.put(b"different-payload")?;

        assert_eq!(first, second);
        assert_ne!(first, other);
        assert_eq!(
            store.get(&first)?.as_deref(),
            Some(b"mesh-payload".as_slice())
        );
        assert!(store.contains(&first)?);

        let object = store.object_path(&first)?;
        assert!(object.is_file());
        assert_eq!(
            object.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new(&first.0[..2]))
        );
        Ok(())
    }

    #[test]
    fn corrupt_blob_is_detected() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
        let id = store.put(b"valid-payload")?;
        fs::write(store.object_path(&id)?, b"corrupt")?;

        let error = store
            .get(&id)
            .expect_err("corrupt object should be rejected");
        assert!(error.to_string().contains("digest mismatch"));
        Ok(())
    }

    #[test]
    fn mesh_blob_round_trips() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 3]);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 0.0, 0.0, 1.0]; 3]);
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));

        let expected = MeshBlob::from_bevy_mesh(&mesh)?;
        let id = put_mesh(&store, &mesh)?;
        let restored = get_mesh(&store, &id)?.expect("mesh blob should exist");
        assert_eq!(expected, MeshBlob::from_bevy_mesh(&restored)?);
        Ok(())
    }

    #[test]
    fn invalid_blob_id_cannot_escape_store_root() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = FilesystemBlobStore::new(directory.path().join("objects"))?;
        let error = store
            .get(&BlobId("../outside".to_owned()))
            .expect_err("invalid id");
        assert!(error.to_string().contains("invalid blob id"));
        Ok(())
    }
}
#[cfg(test)]
#[path = "blob_store_c8_tests.rs"]
mod c8_tests;
