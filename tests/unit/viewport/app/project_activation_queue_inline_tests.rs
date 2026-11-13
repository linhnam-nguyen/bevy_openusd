use std::sync::{Arc, Barrier};

use super::*;
use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use usd_project::{ProjectId, ProjectRoot};
use viewport_protocol::SessionId;

fn request(request_id: &str) -> ProjectActivationRequest {
    ProjectActivationRequest {
        session_id: SessionId::new("session-a"),
        command: ProjectActivationCommand::new(
            request_id,
            1,
            ProjectId::new_v4(),
            ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
        ),
    }
}

#[test]
fn latest_request_replaces_blocked_preparation() {
    let queue = LatestActivationQueue::new();
    assert!(queue.replace(request("A")).is_none());
    assert_eq!(
        queue
            .replace(request("B"))
            .expect("A should be superseded")
            .command
            .request_id,
        "A"
    );
    assert_eq!(
        queue
            .take()
            .expect("B should remain queued")
            .request
            .command
            .request_id,
        "B"
    );
}

#[test]
fn superseded_request_is_not_current() {
    let cancellation = ActivationCancellation::default();
    let first = request("A");
    let second = request("B");
    cancellation.supersede(&first);
    cancellation.supersede(&second);

    assert!(!cancellation.is_current(&first));
    assert!(cancellation.is_current(&second));
}

#[test]
fn blocked_preparation_accepts_newest_request_without_waiting_for_old_one() {
    let queue = Arc::new(LatestActivationQueue::new());
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker_queue = Arc::clone(&queue);
    let worker_entered = Arc::clone(&entered);
    let worker_release = Arc::clone(&release);
    let worker = std::thread::spawn(move || {
        let first = worker_queue.take().expect("A should be admitted");
        worker_entered.wait();
        worker_release.wait();
        let second = worker_queue.take().expect("B should remain available");
        (
            first.request.command.request_id,
            second.request.command.request_id,
        )
    });

    assert!(queue.replace(request("A")).is_none());
    entered.wait();
    assert!(
        queue.replace(request("B")).is_none(),
        "B admission must not wait for in-flight A"
    );
    release.wait();

    assert_eq!(
        worker.join().expect("blocked worker should finish"),
        ("A".to_owned(), "B".to_owned())
    );
}
