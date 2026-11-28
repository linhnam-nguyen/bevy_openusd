use super::{ProjectionIdentity, ProjectionReadback};

#[test]
fn readback_must_match_activation_stage_projection_and_target() {
    let initial = ProjectionIdentity {
        activation: 4,
        stage_session: 12,
        stage_revision: 1,
        projection: 2,
        target: 6,
    };
    let mut receipt = ProjectionReadback::default();
    receipt.observe(initial);
    assert!(
        !receipt.is_complete(initial),
        "a GPU submission needs a successful copy"
    );
    for stale in [
        ProjectionIdentity {
            activation: 3,
            ..initial
        },
        ProjectionIdentity {
            stage_session: 11,
            ..initial
        },
        ProjectionIdentity {
            stage_revision: 0,
            ..initial
        },
        ProjectionIdentity {
            projection: 1,
            ..initial
        },
        ProjectionIdentity {
            target: 5,
            ..initial
        },
    ] {
        receipt.complete(stale);
        assert!(!receipt.is_complete(initial), "stale GPU receipt {stale:?}");
    }
    receipt.complete(initial);
    assert!(receipt.is_complete(initial));
    receipt.observe(ProjectionIdentity {
        projection: 3,
        ..initial
    });
    assert!(
        !receipt.is_complete(initial),
        "reprojection invalidates the prior receipt"
    );
}

#[test]
fn a_timeout_is_terminal_for_the_same_gpu_submission() {
    let identity = ProjectionIdentity {
        activation: 1,
        stage_session: 2,
        stage_revision: 3,
        projection: 4,
        target: 5,
    };
    let completion = viewport_protocol::InitialGpuProjection::Failed {
        message: "timeout".to_owned(),
    };
    let receipt = ProjectionReadback::default();
    receipt.observe(identity);
    receipt.finish(identity, completion.clone());
    receipt.complete(identity);
    assert_eq!(receipt.status(identity), completion);
}

#[test]
fn terminal_initial_projection_survives_later_projection_work_and_resize() -> anyhow::Result<()> {
    use bevy::prelude::{App, Handle, Update};
    use usd_bevy::{LiveStage, ProgressiveProjectionState};
    use viewport_protocol::InitialGpuProjection;

    use super::{
        OffscreenTarget, PreviewReadback, ProjectionCaptureRequest, StageInfo, observe_projection,
    };

    for terminal in [
        InitialGpuProjection::Complete,
        InitialGpuProjection::Failed {
            message: "terminal GPU failure".to_owned(),
        },
    ] {
        let mut app = App::new();
        app.init_resource::<ProjectionReadback>()
            .init_resource::<PreviewReadback>()
            .init_resource::<ProjectionCaptureRequest>()
            .init_resource::<ProgressiveProjectionState>()
            .insert_resource(StageInfo {
                activation_generation: 4,
                initial_gpu_projection: Some(terminal.clone()),
                ..Default::default()
            })
            .insert_resource(OffscreenTarget {
                image_handle: Handle::default(),
                width: 16,
                height: 16,
                generation: 1,
            })
            .add_systems(Update, observe_projection);
        app.world_mut().insert_non_send(LiveStage::new(
            openusd::usd::Stage::builder().in_memory("initial-projection.usda")?,
        ));

        // The default projection is Idle; a later projection may be non-ready.
        app.update();
        assert_eq!(
            app.world().resource::<StageInfo>().initial_gpu_projection,
            Some(terminal.clone())
        );
        assert!(
            app.world()
                .resource::<ProjectionCaptureRequest>()
                .identity
                .is_none()
        );
        app.world_mut().resource_mut::<OffscreenTarget>().generation = 2;
        app.update();
        assert_eq!(
            app.world().resource::<StageInfo>().initial_gpu_projection,
            Some(terminal)
        );
        assert!(
            app.world()
                .resource::<ProjectionCaptureRequest>()
                .identity
                .is_none()
        );

        // A newly installed activation is explicitly initialized by its owner.
        app.world_mut().insert_resource(StageInfo {
            activation_generation: 5,
            initial_gpu_projection: Some(InitialGpuProjection::Pending),
            ..Default::default()
        });
        app.world_mut().insert_non_send(LiveStage::new(
            openusd::usd::Stage::builder().in_memory("new-projection.usda")?,
        ));
        app.update();
        assert_eq!(
            app.world().resource::<StageInfo>().initial_gpu_projection,
            Some(InitialGpuProjection::Pending)
        );
    }
    Ok(())
}
