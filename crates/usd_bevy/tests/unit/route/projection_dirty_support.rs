use super::*;

impl RenderProjectionDirtySet {
    pub(crate) fn len(&self) -> usize {
        self.entities.len()
    }
}

impl MeshProjectionConsumers {
    pub(crate) fn consumer_count(&self, mesh: AssetId<Mesh>) -> usize {
        self.by_mesh.get(&mesh).map_or(0, HashSet::len)
    }
}
