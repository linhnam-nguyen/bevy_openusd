use std::{fs, sync::Arc, thread, time::Duration};

use project_protocol::{ProjectCommitTarget, ProjectWriteError, ProjectWriteErrorCode};

use super::{ProjectRuntimeAuthority, ProjectRuntimeAuthorityQueue, unix_time_ms};

#[test]
fn delayed_renderer_rejects_a_consumed_request_after_host_timeout() {
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
        ProjectRuntimeAuthorityQueue::with_timeout_and_registry_path(
            Duration::from_secs(5),
            &registry_path,
        ),
    );
    let caller_queue = queue.clone();
    let root = directory.path().to_path_buf();
    let caller = thread::spawn(move || {
        caller_queue.begin_commit(&root, project_id, &ProjectCommitTarget::Project)
    });
    let request_directory = directory
        .path()
        .join(".usdhub/cache/project-runtime-authority/requests");
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
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        request_directory.is_dir()
            && request_directory
                .read_dir()
                .expect("request directory")
                .filter_map(Result::ok)
                .any(
                    |entry| entry.path().extension().and_then(|value| value.to_str())
                        == Some("json")
                ),
        "runtime request should be published"
    );
    let requests = queue.consume_pending().expect("consume delayed request");
    assert_eq!(requests.len(), 1);
    let request_id = requests[0].clone().into_request().request_id().to_owned();
    assert!(matches!(
        caller.join().expect("authority caller"),
        Err(ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::Busy
        })
    ));
    assert!(queue.is_cancelled(directory.path(), &request_id));
    assert!(!requests[0].is_expired(unix_time_ms()));
}

#[test]
fn waiting_claim_and_cancellation_have_one_atomic_winner() {
    let directory = tempfile::tempdir().expect("temporary runtime root");
    let project_id = usd_project::ProjectId::new_v4();
    let registry_path = directory.path().join("workspace.json");
    let mut registry =
        crate::project::catalog::workspace_registry::WorkspaceRegistry::load(&registry_path)
            .expect("workspace registry");
    registry
        .register(project_id, directory.path(), None)
        .expect("register project root");
    let queue = ProjectRuntimeAuthorityQueue::with_workspace_registry_path(&registry_path);

    queue
        .prepare_request_claim(directory.path(), "transition")
        .expect("prepare waiting claim");
    assert!(
        queue
            .cancel_request_for_test(directory.path(), "transition")
            .expect("cancel waiting claim")
    );
    assert!(
        !queue
            .claim_request(directory.path(), "transition")
            .expect("claim cancelled request")
    );

    queue
        .prepare_request_claim(directory.path(), "transition-again")
        .expect("prepare second waiting claim");
    assert!(
        queue
            .claim_request(directory.path(), "transition-again")
            .expect("claim waiting request")
    );
    assert!(
        !queue
            .cancel_request_for_test(directory.path(), "transition-again")
            .expect("cancel active request")
    );
}

#[test]
fn idle_consumer_uses_one_shared_inbox_for_all_registered_projects() {
    let directory = tempfile::tempdir().expect("temporary runtime root");
    let registry_path = directory.path().join("workspace.json");
    let mut registry =
        crate::project::catalog::workspace_registry::WorkspaceRegistry::load(&registry_path)
            .expect("workspace registry");
    for index in 0..128 {
        let project_id = usd_project::ProjectId::new_v4();
        let project_root = directory.path().join(format!("project-{index}"));
        registry
            .register(project_id, &project_root, None)
            .expect("register project root");
        let old_queue =
            project_root.join(".usdhub/cache/project-runtime-authority/requests/old-request.json");
        fs::create_dir_all(old_queue.parent().expect("old request parent"))
            .expect("create old request directory");
        fs::write(old_queue, b"not a shared request").expect("write old request");
    }
    let queue = ProjectRuntimeAuthorityQueue::with_workspace_registry_path(&registry_path);

    assert!(
        queue
            .consume_pending()
            .expect("consume shared inbox")
            .is_empty()
    );
    assert_eq!(queue.registered_project_roots().len(), 128);
}

#[test]
fn registry_snapshot_refreshes_only_after_registry_metadata_changes() {
    let directory = tempfile::tempdir().expect("temporary runtime root");
    let registry_path = directory.path().join("workspace.json");
    let mut registry =
        crate::project::catalog::workspace_registry::WorkspaceRegistry::load(&registry_path)
            .expect("workspace registry");
    let first_id = usd_project::ProjectId::new_v4();
    registry
        .register(first_id, &directory.path().join("first"), None)
        .expect("register first project root");
    let queue = ProjectRuntimeAuthorityQueue::with_workspace_registry_path(&registry_path);
    assert_eq!(queue.registered_project_roots().len(), 1);

    let second_id = usd_project::ProjectId::new_v4();
    registry
        .register(second_id, &directory.path().join("second"), None)
        .expect("register second project root");
    assert_eq!(queue.registered_project_roots().len(), 2);
}

#[test]
fn stale_runtime_artifacts_are_removed_from_all_queue_directories() {
    let directory = tempfile::tempdir().expect("temporary runtime root");
    let project_id = usd_project::ProjectId::new_v4();
    let registry_path = directory.path().join("workspace.json");
    let mut registry =
        crate::project::catalog::workspace_registry::WorkspaceRegistry::load(&registry_path)
            .expect("workspace registry");
    registry
        .register(project_id, directory.path(), None)
        .expect("register project root");
    let queue = ProjectRuntimeAuthorityQueue::with_workspace_registry_path(&registry_path);
    let runtime_root = directory
        .path()
        .join(".usdhub/cache/project-runtime-authority");
    for subdirectory in ["requests", "responses", "cancellations", "claims"] {
        let path = runtime_root.join(subdirectory).join("stale.json");
        fs::create_dir_all(path.parent().expect("artifact parent")).expect("create artifact dir");
        fs::write(path, b"stale").expect("write stale artifact");
    }
    thread::sleep(Duration::from_millis(2));

    queue.cleanup_for_test(directory.path());

    for subdirectory in ["requests", "responses", "cancellations", "claims"] {
        assert!(!runtime_root.join(subdirectory).join("stale.json").exists());
    }
}
