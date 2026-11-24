//! NMS3: lossless renderer-only geometry streams. No semantic source changes.
use anyhow::{Result, bail, ensure};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, Mesh, MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues},
};
const MAX: usize = super::mesh_codec::MAX_RENDER_BLOB_BYTES;
const ATTRS: [(MeshVertexAttribute, usize); 8] = [
    (Mesh::ATTRIBUTE_POSITION, 12),
    (Mesh::ATTRIBUTE_NORMAL, 12),
    (Mesh::ATTRIBUTE_UV_0, 8),
    (Mesh::ATTRIBUTE_UV_1, 8),
    (Mesh::ATTRIBUTE_TANGENT, 16),
    (Mesh::ATTRIBUTE_COLOR, 16),
    (Mesh::ATTRIBUTE_JOINT_WEIGHT, 16),
    (Mesh::ATTRIBUTE_JOINT_INDEX, 8),
];
pub(super) fn requires_extended(mesh: &Mesh) -> bool {
    mesh.primitive_topology() != PrimitiveTopology::TriangleList
        || !matches!(mesh.indices(), Some(Indices::U32(_)))
        || mesh.has_morph_targets()
        || mesh.attributes().any(|(a, _)| {
            ![
                Mesh::ATTRIBUTE_POSITION.id,
                Mesh::ATTRIBUTE_NORMAL.id,
                Mesh::ATTRIBUTE_UV_0.id,
                Mesh::ATTRIBUTE_COLOR.id,
            ]
            .contains(&a.id)
        })
}
fn topology(t: PrimitiveTopology) -> u8 {
    match t {
        PrimitiveTopology::PointList => 0,
        PrimitiveTopology::LineList => 1,
        PrimitiveTopology::LineStrip => 2,
        PrimitiveTopology::TriangleList => 3,
        PrimitiveTopology::TriangleStrip => 4,
    }
}
fn count_valid(t: u8, n: usize) -> bool {
    match t {
        0 => true,
        1 => n % 2 == 0,
        2 => n == 0 || n >= 2,
        3 => n % 3 == 0,
        4 => n == 0 || n >= 3,
        _ => false,
    }
}
fn float_bytes<const N: usize>(v: &[[f32; N]], count: usize) -> Result<Vec<u8>> {
    ensure!(
        v.len() == count && v.iter().flatten().all(|x| x.is_finite()),
        "invalid mesh attribute"
    );
    ensure!(
        count.checked_mul(N * 4).is_some_and(|n| n <= MAX),
        "mesh attribute exceeds limit"
    );
    Ok(v.iter().flatten().flat_map(|x| x.to_le_bytes()).collect())
}
// New writes use lossless raw streams until the optional compressor is
// qualified against the Owner's paired latency/memory/fidelity gates. Keep
// mode 1 decoding for immutable objects produced by the previous writer.
fn stream(out: &mut Vec<u8>, raw: &[u8]) -> Result<()> {
    let length = u32::try_from(raw.len())?;
    ensure!(
        out.len()
            .checked_add(5)
            .and_then(|n| n.checked_add(raw.len()))
            .is_some_and(|n| n <= MAX),
        "mesh payload exceeds limit"
    );
    out.push(0);
    out.extend(length.to_le_bytes());
    out.extend(raw);
    Ok(())
}
pub(super) fn encode(mesh: &Mesh) -> Result<Vec<u8>> {
    ensure!(
        !mesh.has_morph_targets(),
        "morph data requires source projection"
    );
    ensure!(
        mesh.attributes()
            .all(|(a, _)| ATTRS.iter().any(|(b, _)| a.id == b.id)),
        "custom mesh attribute requires source projection"
    );
    let count = mesh.count_vertices();
    ensure!(count > 0 && count <= MAX / 12, "invalid mesh vertex count");
    let t = topology(mesh.primitive_topology());
    let index_bytes = mesh.indices().map_or(0, |indices| match indices {
        Indices::U16(values) => values.len().saturating_mul(2),
        Indices::U32(values) => values.len().saturating_mul(4),
    });
    let vertex_stride: usize = ATTRS
        .iter()
        .filter(|(attribute, _)| mesh.attribute(attribute.id).is_some())
        .map(|(_, stride)| stride)
        .sum();
    ensure!(
        count
            .checked_mul(vertex_stride)
            .and_then(|bytes| bytes.checked_add(index_bytes))
            .is_some_and(|bytes| bytes <= MAX),
        "expanded mesh exceeds limit"
    );
    let (width, indices) = match mesh.indices() {
        None => (0, Vec::new()),
        Some(Indices::U16(v)) => (2, v.iter().map(|&x| x as u32).collect()),
        Some(Indices::U32(v)) => (4, v.clone()),
    };
    ensure!(
        count_valid(t, if width == 0 { count } else { indices.len() })
            && indices.iter().all(|&x| (x as usize) < count),
        "invalid mesh indices"
    );

    let flags = ATTRS.iter().enumerate().fold(0u16, |f, (i, (a, _))| {
        f | if mesh.attribute(a.id).is_some() {
            f | (1 << i)
        } else {
            f
        }
    });
    ensure!(flags & 1 == 1, "missing position stream");
    let mut out = b"NMS3".to_vec();
    out.push(t);
    out.push(width);
    out.extend(flags.to_le_bytes());
    out.extend((count as u32).to_le_bytes());
    out.extend((indices.len() as u32).to_le_bytes());
    for (i, (a, stride)) in ATTRS.iter().enumerate() {
        let Some(v) = mesh.attribute(a.id) else {
            continue;
        };
        let raw = match (i, v) {
            (0 | 1, VertexAttributeValues::Float32x3(v)) => float_bytes(v, count)?,
            (2 | 3, VertexAttributeValues::Float32x2(v)) => float_bytes(v, count)?,
            (4 | 5 | 6, VertexAttributeValues::Float32x4(v)) => float_bytes(v, count)?,
            (7, VertexAttributeValues::Uint16x4(v)) => {
                ensure!(v.len() == count, "joint count mismatch");
                v.iter().flatten().flat_map(|x| x.to_le_bytes()).collect()
            }
            _ => bail!("unsupported attribute format"),
        };
        ensure!(raw.len() == count * stride, "attribute length mismatch");
        stream(&mut out, &raw)?;
    }
    if width != 0 {
        let raw: Vec<u8> = indices
            .iter()
            .flat_map(|x| x.to_le_bytes().into_iter().take(width as usize))
            .collect();
        stream(&mut out, &raw)?;
    }
    Ok(out)
}
fn read_stream<'a>(data: &mut &'a [u8], len: usize) -> Result<Vec<u8>> {
    ensure!(data.len() >= 5 && len <= MAX, "invalid mesh stream header");
    let mode = data[0];
    let n = u32::from_le_bytes(data[1..5].try_into().unwrap()) as usize;
    *data = &data[5..];
    ensure!(n <= data.len(), "truncated mesh stream");
    let (b, rest) = data.split_at(n);
    *data = rest;
    if mode == 0 {
        ensure!(n == len, "raw mesh stream length mismatch");
        return Ok(b.to_vec());
    }
    ensure!(mode == 1, "unknown mesh stream codec");
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let tag = b[i];
        i += 1;
        let k = (tag & 127) as usize + 1;
        ensure!(
            out.len().checked_add(k).is_some_and(|v| v <= len),
            "mesh stream expansion exceeds limit"
        );
        if tag & 128 != 0 {
            out.resize(out.len() + k, 0);
        } else {
            ensure!(i + k <= b.len(), "truncated literal run");
            out.extend(&b[i..i + k]);
            i += k;
        }
    }
    ensure!(out.len() == len, "mesh stream length mismatch");
    Ok(out)
}
fn floats<const N: usize>(b: &[u8]) -> Result<Vec<[f32; N]>> {
    let v: Vec<[f32; N]> = b
        .chunks_exact(N * 4)
        .map(|row| {
            std::array::from_fn(|i| f32::from_le_bytes(row[4 * i..4 * i + 4].try_into().unwrap()))
        })
        .collect();
    ensure!(
        v.iter().flatten().all(|x| x.is_finite()),
        "nonfinite mesh data"
    );
    Ok(v)
}
pub(super) fn decode(bytes: &[u8]) -> Result<Mesh> {
    ensure!(
        bytes.len() >= 16 && bytes.len() <= MAX && &bytes[..4] == b"NMS3",
        "invalid mesh v3 header"
    );
    let t = bytes[4];
    let width = bytes[5];
    let flags = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
    let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let n = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    ensure!(
        flags & 1 == 1
            && flags & !255 == 0
            && count > 0
            && matches!(width, 0 | 2 | 4)
            && (width != 0 || n == 0)
            && count_valid(t, if width == 0 { count } else { n }),
        "invalid mesh v3 layout"
    );
    let mut total = n
        .checked_mul(width as usize)
        .ok_or_else(|| anyhow::anyhow!("mesh size overflow"))?;
    for (i, (_, s)) in ATTRS.iter().enumerate() {
        if flags & (1 << i) != 0 {
            total = count
                .checked_mul(*s)
                .and_then(|v| v.checked_add(total))
                .ok_or_else(|| anyhow::anyhow!("mesh size overflow"))?;
        }
    }
    ensure!(total <= MAX, "expanded mesh exceeds limit");
    let mut data = &bytes[16..];
    let topology = match t {
        0 => PrimitiveTopology::PointList,
        1 => PrimitiveTopology::LineList,
        2 => PrimitiveTopology::LineStrip,
        3 => PrimitiveTopology::TriangleList,
        4 => PrimitiveTopology::TriangleStrip,
        _ => bail!("unknown topology"),
    };
    let mut mesh = Mesh::new(topology, RenderAssetUsages::default());
    for (i, (a, stride)) in ATTRS.iter().enumerate() {
        if flags & (1 << i) == 0 {
            continue;
        }
        let raw = read_stream(&mut data, count * stride)?;
        let value = match i {
            0 | 1 => VertexAttributeValues::Float32x3(floats(&raw)?),
            2 | 3 => VertexAttributeValues::Float32x2(floats(&raw)?),
            4 | 5 | 6 => VertexAttributeValues::Float32x4(floats(&raw)?),
            7 => VertexAttributeValues::Uint16x4(
                raw.chunks_exact(8)
                    .map(|r| {
                        std::array::from_fn(|j| {
                            u16::from_le_bytes(r[2 * j..2 * j + 2].try_into().unwrap())
                        })
                    })
                    .collect(),
            ),
            _ => unreachable!(),
        };
        mesh.insert_attribute(*a, value);
    }
    if width != 0 {
        let raw = read_stream(&mut data, n * width as usize)?;
        if width == 2 {
            let v: Vec<u16> = raw
                .chunks_exact(2)
                .map(|r| u16::from_le_bytes(r.try_into().unwrap()))
                .collect();
            ensure!(
                v.iter().all(|&x| (x as usize) < count),
                "index out of range"
            );
            mesh.insert_indices(Indices::U16(v));
        } else {
            let v: Vec<u32> = raw
                .chunks_exact(4)
                .map(|r| u32::from_le_bytes(r.try_into().unwrap()))
                .collect();
            ensure!(
                v.iter().all(|&x| (x as usize) < count),
                "index out of range"
            );
            mesh.insert_indices(Indices::U32(v));
        }
    }
    ensure!(data.is_empty(), "trailing mesh bytes");
    Ok(mesh)
}
