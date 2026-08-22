use bevy::prelude::Resource;

/// Renderer-neutral PointInstancer selection identity.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct PointInstancerSelection {
    pub instancer_path: Option<String>,
    pub logical_id: Option<i64>,
}

impl PointInstancerSelection {
    pub fn select(&mut self, instancer_path: impl Into<String>, logical_id: i64) {
        self.instancer_path = Some(instancer_path.into());
        self.logical_id = Some(logical_id);
    }

    pub fn clear(&mut self) {
        self.instancer_path = None;
        self.logical_id = None;
    }
}

/// PointInstancer route work counters used by correctness and release gates.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PointInstancerStats {
    pub full_projects: u64,
    pub sparse_transform_patches: u64,
    pub instance_spawns: u64,
    pub instance_despawns: u64,
    pub transform_updates: u64,
}
