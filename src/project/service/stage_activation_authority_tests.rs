use super::*;
use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use usd_project::{ProjectId, SceneId};

fn command(generation: u64) -> ProjectActivationCommand {
    ProjectActivationCommand::new(
        format!("authority-{generation}"),
        generation,
        ProjectId::new_v4(),
        ProjectStageTarget::Scene(SceneId::new_v4()),
    )
}

#[test]
fn stale_completion_cannot_replace_latest_active_identity() {
    let mut authority = ProjectActivationAuthority::default();
    let first = command(1);
    let second = command(2);

    assert!(authority.observe_request("session", &first));
    assert!(authority.commit("session", &first));
    assert!(authority.observe_request("session", &second));
    assert!(!authority.commit("session", &first));
    assert!(authority.commit("session", &second));
    assert_eq!(
        authority.active(),
        Some(&ActiveProjectStage {
            project_id: second.project_id,
            target: second.target,
            generation: 2,
        })
    );
}

#[test]
fn stage_send_probe_keeps_activation_on_safe_two_phase_boundary() {
    assert_eq!(
        OPENUSD_STAGE_SEND_PROBE,
        StageSendProbeOutcome::NotSend,
        "OpenUSD Stage must not cross the worker-thread boundary"
    );
}
