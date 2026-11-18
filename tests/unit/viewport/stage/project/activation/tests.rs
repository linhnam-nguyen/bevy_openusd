use super::*;
use crate::viewport::api::RenderServerInterface;
use project_protocol::{ProjectActivationCommand, ProjectActivationReply, ProjectStageTarget};
use usd_project::{ProjectId, ProjectRoot, SceneId};
use viewport_protocol::SessionId;
use viewport_streaming::ProjectActivationResult as RoutedProjectActivationResult;

#[test]
fn activation_reply_preserves_project_root_generation_and_has_no_path() {
    let project_id = ProjectId::new_v4();
    let command = ProjectActivationCommand::new(
        "activation-b",
        7,
        project_id,
        ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(SceneId::new_v4())),
    );
    let reply = ProjectActivationReply::activated(&command);
    let encoded = serde_json::to_string(&reply).unwrap();

    assert!(encoded.contains("activation-b"));
    assert!(encoded.contains(&project_id.to_string()));
    assert!(!encoded.contains("/Users/"));
}

#[test]
fn activation_requests_are_routed_to_the_requesting_session() {
    let interface = RenderServerInterface::default();
    let project_id = ProjectId::new_v4();
    let command = ProjectActivationCommand::new(
        "activation-a",
        1,
        project_id,
        ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
    );
    interface
        .submit_project_activation(ProjectActivationRequest {
            session_id: SessionId::new("session-a"),
            command: command.clone(),
        })
        .unwrap();
    let request = interface.pop_project_activation().unwrap();
    interface
        .publish_project_activation_result(RoutedProjectActivationResult {
            session_id: request.session_id.clone(),
            reply: ProjectActivationReply::activated(&request.command),
        })
        .unwrap();

    let result = interface
        .take_project_activation_result(&SessionId::new("session-a"))
        .unwrap();
    assert_eq!(result.session_id, SessionId::new("session-a"));
    assert_eq!(result.reply.request_id, command.request_id);
}

#[test]
fn preparation_worker_preserves_request_identity_when_registry_is_unavailable() {
    let runtime = ProjectStageActivationRuntime::with_registry_path(None);
    let command = ProjectActivationCommand::new(
        "activation-preparation-failure",
        4,
        ProjectId::new_v4(),
        ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
    );
    let request = ProjectActivationRequest {
        session_id: SessionId::new("session-preparation"),
        command: command.clone(),
    };

    assert!(runtime.submit(request.clone()).is_none());
    let prepared = runtime
        .wait_for_prepared()
        .expect("preparation worker should return a result");
    assert_eq!(prepared.request.session_id, request.session_id);
    assert_eq!(prepared.request.command, command);
    assert_eq!(
        prepared.target,
        Err("Project activation registry is unavailable".to_owned())
    );
}
