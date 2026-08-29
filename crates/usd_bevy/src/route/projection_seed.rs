use std::collections::HashMap;

use bevy::prelude::*;

/// Renderer-owned assets prepared by an application cache before a stage is
/// projected. The renderer receives only validated Bevy handles keyed by USD
/// prim path; it has no knowledge of Project identities or cache storage.
#[derive(Resource, Default)]
pub struct ProjectionSeed {
    meshes: HashMap<String, SeededMesh>,
    materials: HashMap<String, Handle<StandardMaterial>>,
}

#[derive(Clone)]
pub(crate) struct SeededMesh {
    pub(crate) handle: Handle<Mesh>,
    pub(crate) local_extent: Option<([f32; 3], [f32; 3])>,
}

impl ProjectionSeed {
    /// Insert a validated mesh for one composed USD prim path.
    pub fn insert_mesh(
        &mut self,
        prim_path: impl Into<String>,
        handle: Handle<Mesh>,
        local_extent: Option<([f32; 3], [f32; 3])>,
    ) {
        self.meshes.insert(
            prim_path.into(),
            SeededMesh {
                handle,
                local_extent,
            },
        );
    }

    /// Insert a validated material for one composed USD prim path.
    pub fn insert_material(
        &mut self,
        prim_path: impl Into<String>,
        handle: Handle<StandardMaterial>,
    ) {
        self.materials.insert(prim_path.into(), handle);
    }

    /// Drop all unconsumed prepared assets before replacing the active stage.
    pub fn clear(&mut self) {
        self.meshes.clear();
        self.materials.clear();
    }

    /// Number of prepared mesh seeds waiting for the normal projection route.
    pub fn pending_meshes(&self) -> usize {
        self.meshes.len()
    }

    /// Number of prepared material seeds waiting for the normal projection route.
    pub fn pending_materials(&self) -> usize {
        self.materials.len()
    }

    pub(crate) fn take_mesh(&mut self, prim_path: &str) -> Option<SeededMesh> {
        self.meshes.remove(prim_path)
    }

    pub(crate) fn take_material(&mut self, prim_path: &str) -> Option<Handle<StandardMaterial>> {
        self.materials.remove(prim_path)
    }
}
