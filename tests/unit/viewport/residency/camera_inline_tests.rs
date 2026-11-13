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
