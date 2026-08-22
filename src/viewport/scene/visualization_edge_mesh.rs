use std::collections::HashSet;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};

pub(super) fn build_edge_mesh(source: &Mesh) -> Option<Mesh> {
    let VertexAttributeValues::Float32x3(positions) = source.attribute(Mesh::ATTRIBUTE_POSITION)?
    else {
        return None;
    };

    let indices: Vec<u32> = match source.indices() {
        Some(Indices::U16(indices)) => indices.iter().map(|index| u32::from(*index)).collect(),
        Some(Indices::U32(indices)) => indices.clone(),
        None => (0..positions.len() as u32).collect(),
    };

    let mut edge_indices = Vec::with_capacity(indices.len() * 2);
    let mut seen = HashSet::with_capacity(indices.len());
    match source.primitive_topology() {
        PrimitiveTopology::TriangleList => {
            for triangle in indices.chunks_exact(3) {
                add_edge(&mut seen, &mut edge_indices, triangle[0], triangle[1]);
                add_edge(&mut seen, &mut edge_indices, triangle[1], triangle[2]);
                add_edge(&mut seen, &mut edge_indices, triangle[2], triangle[0]);
            }
        }
        PrimitiveTopology::TriangleStrip => {
            for triangle in indices.windows(3) {
                add_edge(&mut seen, &mut edge_indices, triangle[0], triangle[1]);
                add_edge(&mut seen, &mut edge_indices, triangle[1], triangle[2]);
                add_edge(&mut seen, &mut edge_indices, triangle[2], triangle[0]);
            }
        }
        _ => return None,
    }

    if edge_indices.is_empty() {
        return None;
    }

    let mut edge_mesh = Mesh::new(
        PrimitiveTopology::LineList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    edge_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
    edge_mesh.insert_indices(Indices::U32(edge_indices));
    Some(edge_mesh)
}

fn add_edge(seen: &mut HashSet<(u32, u32)>, output: &mut Vec<u32>, a: u32, b: u32) {
    if a == b {
        return;
    }
    let edge = if a < b { (a, b) } else { (b, a) };
    if seen.insert(edge) {
        output.extend_from_slice(&[edge.0, edge.1]);
    }
}
