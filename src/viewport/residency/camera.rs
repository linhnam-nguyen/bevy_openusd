//! Camera movement thresholds used by residency re-evaluation.

use bevy::camera::primitives::Frustum;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CameraSample {
    pub(crate) position: [f64; 3],
    pub(crate) forward: [f64; 3],
    pub(crate) search_radius: f64,
    pub(crate) preload_margin: f64,
    pub(crate) projection: [f32; 16],
    pub(crate) viewport_size: [u32; 2],
    pub(crate) section_box_revision: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CameraAdmission {
    pub(crate) sample: CameraSample,
    pub(crate) frustum: Frustum,
    pub(crate) section_box: Option<SectionBoxClipPlanes>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SectionBoxClipPlanes {
    pub(crate) planes: [[f64; 4]; 6],
}

impl Default for CameraSample {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            search_radius: 64.0,
            preload_margin: 16.0,
            projection: [0.0; 16],
            viewport_size: [0; 2],
            section_box_revision: 0,
        }
    }
}

impl CameraSample {
    pub(crate) fn changed_meaningfully(self, next: Self) -> bool {
        let moved = squared_distance(self.position, next.position) > 0.25;
        let rotated = 1.0 - normalized_dot(self.forward, next.forward) > 0.001;
        moved
            || rotated
            || (self.search_radius - next.search_radius).abs() > 0.5
            || (self.preload_margin - next.preload_margin).abs() > 0.5
            || self.projection != next.projection
            || self.viewport_size != next.viewport_size
            || self.section_box_revision != next.section_box_revision
    }
}

fn squared_distance(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum()
}

fn normalized_dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    let left_len = squared_length(left).sqrt();
    let right_len = squared_length(right).sqrt();
    if left_len <= f64::EPSILON || right_len <= f64::EPSILON {
        return 1.0;
    }
    let dot = left
        .into_iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f64>()
        / (left_len * right_len);
    dot.clamp(-1.0, 1.0)
}

fn squared_length(value: [f64; 3]) -> f64 {
    value
        .into_iter()
        .map(|component| component * component)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_camera_jitter_does_not_rebuild_residency() {
        let current = CameraSample::default();
        let next = CameraSample {
            position: [0.1, 0.1, 0.1],
            ..current
        };
        assert!(!current.changed_meaningfully(next));
    }

    #[test]
    fn translation_and_rotation_thresholds_trigger_requery() {
        let current = CameraSample::default();
        assert!(current.changed_meaningfully(CameraSample {
            position: [1.0, 0.0, 0.0],
            ..current
        }));
        assert!(current.changed_meaningfully(CameraSample {
            forward: [1.0, 0.0, 0.0],
            ..current
        }));
    }

    #[test]
    fn projection_viewport_and_section_box_changes_trigger_requery() {
        let current = CameraSample::default();
        assert!(current.changed_meaningfully(CameraSample {
            projection: [1.0; 16],
            ..current
        }));
        assert!(current.changed_meaningfully(CameraSample {
            viewport_size: [1920, 1080],
            ..current
        }));
        assert!(current.changed_meaningfully(CameraSample {
            section_box_revision: 1,
            ..current
        }));
    }
}
