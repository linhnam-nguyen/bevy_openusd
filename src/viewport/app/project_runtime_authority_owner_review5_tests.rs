use std::{sync::Arc, thread, time::Duration};

use bevy::prelude::World;
use project_protocol::ProjectCommitTarget;

use super::super::{
    ActiveProjectRuntimeLease, ProjectRuntimeAuthorityRuntime, runtime_renew_response,
    runtime_snapshot_response,
};
use crate::project::service::ProjectRuntimeAuthority;

#[test]
fn active_runtime_stage_returns_ready_and_freezes_authoring() {
    let directory = tempfile::tempdir().expect("temporary runtime root");
    let stage = openusd::usd::Stage::builder()
        .in_memory("active-runtime-request.usda")
        .expect("in-memory stage");
    let live = usd_bevy::LiveStage::new(stage);
    let project_id = usd_project::ProjectId::new_v4();
    let scene_id = usd_project::SceneId::new_v4();
    let mut world = World::new();
    let queue = crate::project::service::ProjectRuntimeAuthorityQueue::default();
    world.insert_resource(ProjectRuntimeAuthorityRuntime {
        queue: queue.clone(),
        active_lease: None,
    });
    world.insert_non_send(live);

    assert!(matches!(
        runtime_snapshot_response(
            &mut world,
            "active-request",
            project_id,
            scene_id,
            &queue,
            directory.path(),
        ),
        crate::project::service::ProjectRuntimeResponse::Ready { .. }
    ));
    assert!(
        world
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage")
            .is_authoring_frozen()
    );
    assert!(
        world
            .resource::<ProjectRuntimeAuthorityRuntime>()
            .active_lease
            .is_some()
    );
}

#[test]
fn timed_out_consumed_request_never_freezes_or_creates_a_lease() {
    let directory = tempfile::tempdir().expect("temporary runtime root");
    let project_id = usd_project::ProjectId::new_v4();
    let queue = Arc::new(crate::project::service::ProjectRuntimeAuthorityQueue::default());
    let caller_queue = queue.clone();
    let root = directory.path().to_path_buf();
    let caller = thread::spawn(move || {
        caller_queue.begin_commit(&root, project_id, &ProjectCommitTarget::Project)
    });
    let request_directory = directory
        .path()
        .join(".usdhub/cache/project-runtime-authority/requests");
    let mut saw_request = false;
    for _ in 0..1_000 {
        if request_directory.is_dir()
            && request_directory
                .read_dir()
                .expect("request directory")
                .filter_map(Result::ok)
                .any(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
        {
            saw_request = true;
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(saw_request, "runtime request should be published");
    let requests = queue
        .consume_pending(directory.path())
        .expect("consume request before timeout");
    assert_eq!(requests.len(), 1);
    let request_id = requests[0].clone().into_request().request_id().to_owned();
    assert!(matches!(
        caller.join().expect("authority caller"),
        Err(project_protocol::ProjectWriteError::Failed {
            code: project_protocol::ProjectWriteErrorCode::Busy
        })
    ));

    let stage = openusd::usd::Stage::builder()
        .in_memory("cancelled-runtime-request.usda")
        .expect("in-memory stage");
    let live = usd_bevy::LiveStage::new(stage);
    let mut world = World::new();
    world.insert_resource(ProjectRuntimeAuthorityRuntime {
        queue: (*queue).clone(),
        active_lease: None,
    });
    world.insert_non_send(live);
    let response = runtime_snapshot_response(
        &mut world,
        &request_id,
        project_id,
        usd_project::SceneId::new_v4(),
        queue.as_ref(),
        directory.path(),
    );

    assert!(matches!(
        response,
        crate::project::service::ProjectRuntimeResponse::Failed {
            code: project_protocol::ProjectWriteErrorCode::Busy,
            ..
        }
    ));
    assert!(
        !world
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage")
            .is_authoring_frozen()
    );
    assert!(
        world
            .resource::<ProjectRuntimeAuthorityRuntime>()
            .active_lease
            .is_none()
    );
}

#[test]
fn healthy_runtime_lease_renewal_extends_deadline_and_stays_frozen() {
    let stage = openusd::usd::Stage::builder()
        .in_memory("renewed-runtime-lease.usda")
        .expect("in-memory stage");
    let live = usd_bevy::LiveStage::new(stage);
    let project_id = usd_project::ProjectId::new_v4();
    let session_id = live.session_id();
    let mut world = World::new();
    world.insert_resource(ProjectRuntimeAuthorityRuntime {
        queue: crate::project::service::ProjectRuntimeAuthorityQueue::default(),
        active_lease: Some(ActiveProjectRuntimeLease {
            project_id,
            lease_id: "renewed".to_owned(),
            session_id,
            live_revision: 0,
            expires_at_ms: crate::project::service::unix_time_ms() + 1,
        }),
    });
    world.insert_non_send(live);
    world
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .try_freeze_authoring();
    let before = world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .active_lease
        .as_ref()
        .expect("active lease")
        .expires_at_ms;

    assert!(matches!(
        runtime_renew_response(&mut world, "renew-request", project_id, "renewed", 0),
        crate::project::service::ProjectRuntimeResponse::Renewed { .. }
    ));
    let after = world
        .resource::<ProjectRuntimeAuthorityRuntime>()
        .active_lease
        .as_ref()
        .expect("active lease")
        .expires_at_ms;
    assert!(after > before);
    assert!(
        world
            .get_non_send::<usd_bevy::LiveStage>()
            .expect("live stage")
            .is_authoring_frozen()
    );
}
