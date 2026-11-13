//! Test-only helpers attached to the owning production module.
use super::*;

impl ResidencyAuthority {
    pub(crate) fn complete_cpu(
        &mut self,
        key: ScenePayloadKey,
        generation: u64,
        cpu_bytes: u64,
        gpu_bytes: u64,
    ) -> bool {
        let mask = self.in_flight_mask(&key);
        self.complete_cpu_payload(
            key,
            generation,
            cpu_bytes,
            gpu_bytes,
            if mask.is_empty() {
                PayloadLoadMask::GEOMETRY
            } else {
                mask
            },
            Some(test_mesh()),
        )
    }
}

pub(super) fn test_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
}
