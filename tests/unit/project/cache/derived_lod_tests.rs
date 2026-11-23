use super::*;
use crate::project::cache::blobs::FilesystemBlobStore;

#[test]
fn generated_planar_mesh_is_deterministic_and_preserves_the_boundary() -> Result<()> {
    const SIDE: u32 = 12;
    let positions: Vec<_> = (0..SIDE)
        .flat_map(|y| (0..SIDE).map(move |x| [x as f32, y as f32, 0.0]))
        .collect();
    let mut indices = Vec::new();
    for y in 0..SIDE - 1 {
        for x in 0..SIDE - 1 {
            let i = y * SIDE + x;
            indices.extend_from_slice(&[i, i + 1, i + SIDE, i + 1, i + SIDE + 1, i + SIDE]);
        }
    }
    let mesh = MeshBlob {
        version: mesh_codec::BINARY_VERSION,
        uvs: Some(
            positions
                .iter()
                .map(|p| [p[0] / SIDE as f32, p[1] / SIDE as f32])
                .collect(),
        ),
        positions,
        normals: Some(vec![[0.0, 0.0, 1.0]; (SIDE * SIDE) as usize]),
        colors: None,
        indices,
    };
    let directory = tempfile::tempdir()?;
    let store = FilesystemBlobStore::new(directory.path())?;
    let original = mesh_codec::encode(&mesh)?;
    let full = store.put(&original)?;
    let settings = LODSettings {
        target_ratio: 0.5,
        max_relative_error: 0.02,
    };
    let first = generate_static_candidate(&store, "/SceneRoot/Grid", &full, &original, settings)?;
    let second = generate_static_candidate(&store, "/SceneRoot/Grid", &full, &original, settings)?;
    assert_eq!(first, second);
    assert_eq!(store.get(&full)?.as_deref(), Some(original.as_slice()));
    assert!(first.derived_triangles < first.original_triangles);
    assert!(first.derived_bytes < first.original_bytes);
    assert!(first.measured_error <= settings.max_relative_error);
    assert_eq!(first.original_bounds, first.derived_bounds);
    let simplified = mesh_codec::decode(
        &store
            .get(&first.visual_blob)?
            .context("missing generated blob")?,
    )?;
    for (slot, source) in first.source_vertex_rows.iter().copied().enumerate() {
        assert_eq!(simplified.positions[slot], mesh.positions[source as usize]);
        assert_eq!(
            simplified.uvs.as_ref().unwrap()[slot],
            mesh.uvs.as_ref().unwrap()[source as usize]
        );
    }
    let mut expanded = simplified;
    for index in &mut expanded.indices {
        *index = first.source_vertex_rows[*index as usize];
    }
    assert_eq!(
        verified_boundary_edges(&expanded)?,
        verified_boundary_edges(&mesh)?
    );
    Ok(())
}
