use std::collections::HashSet;

use bevy::prelude::*;

pub(in crate::viewport) fn collect_mesh_descendants(
    root: Entity,
    meshes: &Query<(Option<&Mesh3d>, Option<&Children>)>,
    output: &mut HashSet<Entity>,
) {
    let Ok((mesh, children)) = meshes.get(root) else {
        return;
    };
    if mesh.is_some() {
        output.insert(root);
    }
    if let Some(children) = children {
        for child in children.iter() {
            collect_mesh_descendants(child, meshes, output);
        }
    }
}
