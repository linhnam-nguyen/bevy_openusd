use std::{sync::Arc, thread, time::Duration};

use project_protocol::{ProjectCommitTarget, ProjectWriteError, ProjectWriteErrorCode};

use super::{ProjectRuntimeAuthority, ProjectRuntimeAuthorityQueue, unix_time_ms};

#[test]
fn delayed_renderer_rejects_a_consumed_expired_request() {
    let directory = tempfile::tempdir().expect("temporary runtime root");
    let project_id = usd_project::ProjectId::new_v4();
    let queue = Arc::new(ProjectRuntimeAuthorityQueue::default());
    let caller_queue = queue.clone();
    let root = directory.path().to_path_buf();
    let caller = thread::spawn(move || {
        caller_queue.begin_commit(&root, project_id, &ProjectCommitTarget::Project)
    });
    let request_directory = directory
        .path()
        .join(".usdhub/cache/project-runtime-authority/requests");
    while !request_directory.is_dir()
        || !request_directory
            .read_dir()
            .expect("request directory")
            .filter_map(Result::ok)
            .any(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
    {
        thread::yield_now();
    }
    let requests = queue
        .consume_pending(directory.path())
        .expect("consume delayed request");
    assert_eq!(requests.len(), 1);
    thread::sleep(Duration::from_millis(1_010));
    assert!(requests[0].is_expired(unix_time_ms()));
    assert!(matches!(
        caller.join().expect("authority caller"),
        Err(ProjectWriteError::Failed {
            code: ProjectWriteErrorCode::Busy
        })
    ));
}
