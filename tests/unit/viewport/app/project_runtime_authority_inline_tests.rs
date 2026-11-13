use super::*;
use std::{sync::Arc, thread, time::Duration};

use crate::project::service::ProjectRuntimeAuthority;
use project_protocol::ProjectCommitTarget;

#[test]
fn runtime_lease_rejects_a_synchronous_authoring_generation_change() {
    let stage = openusd::usd::Stage::builder()
        .in_memory("runtime-lease-generation.usda")
        .expect("in-memory stage");
    let live = usd_bevy::LiveStage::new(stage);
    let project_id = usd_project::ProjectId::new_v4();
    let session_id = live.session_id();
    let mut world = World::new();
    world.insert_resource(ProjectRuntimeAuthorityRuntime {
        queue: crate::project::service::ProjectRuntimeAuthorityQueue::default(),
        active_lease: Some(super::super::ActiveProjectRuntimeLease {
            project_id,
            lease_id: "lease".to_owned(),
            session_id,
            live_revision: 0,
            expires_at_ms: crate::project::service::unix_time_ms() + 60_000,
        }),
    });
    world.insert_non_send(live);
    let live = world
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage");
    assert!(live.try_freeze_authoring());
    usd_bevy::authoring::define_prim(&live.stage, "/ConcurrentEdit", "Xform")
        .expect("author concurrent edit");
    assert!(live.authoring_generation().0 > 0);

    assert!(matches!(
        super::super::runtime_revision_response(&world, "request", project_id, "lease", 0),
        ProjectRuntimeResponse::Failed {
            code: project_protocol::ProjectWriteErrorCode::ConcurrentChange,
            ..
        }
    ));
}

#[test]
fn no_active_stage_answers_registered_requests_as_inactive() {
    let directory = tempfile::tempdir().expect("temporary runtime root");
    let project_id = usd_project::ProjectId::new_v4();
    let registry_path = directory.path().join("workspace.json");
    let mut registry =
        crate::project::catalog::workspace_registry::WorkspaceRegistry::load(&registry_path)
            .expect("workspace registry");
    registry
        .register(project_id, directory.path(), None)
        .expect("register project root");
    let queue = Arc::new(
        crate::project::service::ProjectRuntimeAuthorityQueue::with_timeout_and_registry_path(
            Duration::from_millis(250),
            registry_path,
        ),
    );
    let request_queue = queue.clone();
    let root = directory.path().to_path_buf();
    let caller = thread::spawn(move || {
        request_queue.begin_commit(&root, project_id, &ProjectCommitTarget::Project)
    });
    let request_directory = directory
        .path()
        .join(".usdhub/cache/project-runtime-authority/requests");
    for _ in 0..100 {
        if request_directory.is_dir()
            && request_directory
                .read_dir()
                .expect("request directory")
                .filter_map(Result::ok)
                .any(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
        {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }

    let mut world = World::new();
    world.insert_resource(ProjectRuntimeAuthorityRuntime {
        queue: (*queue).clone(),
        active_lease: None,
    });
    consume_project_runtime_authority(&mut world);

    assert!(matches!(caller.join().expect("authority caller"), Ok(None)));
}

#[test]
fn expired_runtime_lease_unfreezes_live_stage() {
    let stage = openusd::usd::Stage::builder()
        .in_memory("expired-runtime-lease.usda")
        .expect("in-memory stage");
    let live = usd_bevy::LiveStage::new(stage);
    let project_id = usd_project::ProjectId::new_v4();
    let session_id = live.session_id();
    let mut world = World::new();
    world.insert_resource(ProjectRuntimeAuthorityRuntime {
        queue: crate::project::service::ProjectRuntimeAuthorityQueue::default(),
        active_lease: Some(super::super::ActiveProjectRuntimeLease {
            project_id,
            lease_id: "expired".to_owned(),
            session_id,
            live_revision: 0,
            expires_at_ms: 0,
        }),
    });
    world.insert_non_send(live);
    world
        .get_non_send::<usd_bevy::LiveStage>()
        .expect("live stage")
        .try_freeze_authoring();

    super::super::expire_runtime_lease(&mut world);

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
