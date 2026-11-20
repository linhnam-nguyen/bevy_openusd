//! Durable renderer mesh framing. Bevy's serialized assets remain ephemeral.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use super::blobs::MeshBlob;

pub(super) const LEGACY_JSON_VERSION: u16 = 1;
pub(super) const BINARY_VERSION: u16 = 2;
const MAGIC: &[u8; 4] = b"NMSH";
const HEADER_BYTES: usize = 16;
pub(super) const MAX_RENDER_BLOB_BYTES: usize = 512 * 1024 * 1024;
const NORMALS: u16 = 1;
const UVS: u16 = 1 << 1;
const COLORS: u16 = 1 << 2;
const ATTRIBUTE_FLAGS: u16 = NORMALS | UVS | COLORS;

/// Binary v2 is little-endian. The header is magic, version u16, attribute
/// flags u16, vertex count u32 and index count u32. The payload contains
/// tightly packed position, normal, UV, color and index streams in that order.
struct Header {
    flags: u16,
    vertices: usize,
    indices: usize,
}

impl Header {
    fn read(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() >= HEADER_BYTES, "truncated mesh binary header");
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        ensure!(
            version == BINARY_VERSION,
            "unsupported binary mesh version {version}"
        );
        let header = Self {
            flags: u16::from_le_bytes([bytes[6], bytes[7]]),
            vertices: u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            indices: u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize,
        };
        ensure!(
            header.flags & !ATTRIBUTE_FLAGS == 0,
            "unknown mesh attribute flags"
        );
        ensure!(
            bytes.len() == header.encoded_len()?,
            "mesh binary size does not match its header"
        );
        Ok(header)
    }

    fn encoded_len(&self) -> Result<usize> {
        let mut words_per_vertex = 3;
        if self.flags & NORMALS != 0 {
            words_per_vertex += 3;
        }
        if self.flags & UVS != 0 {
            words_per_vertex += 2;
        }
        if self.flags & COLORS != 0 {
            words_per_vertex += 4;
        }
        let bytes = self
            .vertices
            .checked_mul(words_per_vertex)
            .and_then(|words| words.checked_add(self.indices))
            .and_then(|words| words.checked_mul(4))
            .and_then(|bytes| bytes.checked_add(HEADER_BYTES))
            .context("mesh binary size overflow")?;
        ensure!(
            bytes <= MAX_RENDER_BLOB_BYTES,
            "mesh payload exceeds decoder byte limit"
        );
        Ok(bytes)
    }
}

#[derive(Deserialize)]
struct LegacyJsonHeader {
    version: u16,
    #[serde(rename = "positions")]
    _positions: serde::de::IgnoredAny,
    #[serde(rename = "indices")]
    _indices: serde::de::IgnoredAny,
}

pub(crate) fn payload_version(bytes: &[u8]) -> Result<u16> {
    ensure!(
        bytes.len() <= MAX_RENDER_BLOB_BYTES,
        "mesh payload exceeds decoder byte limit"
    );
    if bytes.starts_with(MAGIC) {
        Header::read(bytes)?;
        return Ok(BINARY_VERSION);
    }
    let legacy: LegacyJsonHeader =
        serde_json::from_slice(bytes).context("read legacy mesh blob header")?;
    ensure!(
        legacy.version == LEGACY_JSON_VERSION,
        "unsupported JSON mesh version {}",
        legacy.version
    );
    Ok(legacy.version)
}

pub(super) fn encode(mesh: &MeshBlob) -> Result<Vec<u8>> {
    mesh.validate()?;
    let header = Header {
        flags: u16::from(mesh.normals.is_some()) * NORMALS
            | u16::from(mesh.uvs.is_some()) * UVS
            | u16::from(mesh.colors.is_some()) * COLORS,
        vertices: mesh.positions.len(),
        indices: mesh.indices.len(),
    };
    let mut bytes = Vec::with_capacity(header.encoded_len()?);
    bytes.extend(MAGIC);
    bytes.extend(BINARY_VERSION.to_le_bytes());
    bytes.extend(header.flags.to_le_bytes());
    bytes.extend(u32::try_from(header.vertices)?.to_le_bytes());
    bytes.extend(u32::try_from(header.indices)?.to_le_bytes());
    write_vectors(&mut bytes, &mesh.positions);
    if let Some(normals) = &mesh.normals {
        write_vectors(&mut bytes, normals);
    }
    if let Some(uvs) = &mesh.uvs {
        write_vectors(&mut bytes, uvs);
    }
    if let Some(colors) = &mesh.colors {
        write_vectors(&mut bytes, colors);
    }
    for index in &mesh.indices {
        bytes.extend(index.to_le_bytes());
    }
    Ok(bytes)
}

fn write_vectors<const N: usize>(bytes: &mut Vec<u8>, vectors: &[[f32; N]]) {
    for component in vectors.iter().flatten() {
        bytes.extend(component.to_le_bytes());
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<MeshBlob> {
    ensure!(
        bytes.len() <= MAX_RENDER_BLOB_BYTES,
        "mesh payload exceeds decoder byte limit"
    );
    let mesh = if bytes.starts_with(MAGIC) {
        let header = Header::read(bytes)?;
        let mut words = bytes[HEADER_BYTES..]
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes(word.try_into().unwrap()));
        MeshBlob {
            version: BINARY_VERSION,
            positions: read_vectors(&mut words, header.vertices),
            normals: (header.flags & NORMALS != 0)
                .then(|| read_vectors(&mut words, header.vertices)),
            uvs: (header.flags & UVS != 0).then(|| read_vectors(&mut words, header.vertices)),
            colors: (header.flags & COLORS != 0).then(|| read_vectors(&mut words, header.vertices)),
            indices: words.collect(),
        }
    } else {
        let legacy: MeshBlob =
            serde_json::from_slice(bytes).context("decode legacy JSON mesh blob")?;
        ensure!(
            legacy.version == LEGACY_JSON_VERSION,
            "unsupported JSON mesh version {}",
            legacy.version
        );
        legacy
    };
    mesh.validate()?;
    Ok(mesh)
}

fn read_vectors<const N: usize>(
    words: &mut impl Iterator<Item = u32>,
    vertices: usize,
) -> Vec<[f32; N]> {
    (0..vertices)
        .map(|_| {
            std::array::from_fn(|_| f32::from_bits(words.next().expect("validated mesh size")))
        })
        .collect()
}
