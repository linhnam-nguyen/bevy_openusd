//! Render-host Project activation boundary.
//!
//! Transport code submits stable Project identity. This module is the only
//! place where that identity is resolved through the Project application
//! service and handed to the existing LiveStage stage-open lifecycle.

use std::path::PathBuf;

use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{App, Resource, Update, World};
use project_protocol::ProjectActivationReply;
use viewport_streaming::{
    ProjectActivationRequest, ProjectActivationResult as RoutedProjectActivationResult,
};

use crate::project::service::ProjectApplicationService;
use crate::viewport::api::RenderServerInterface;
use crate::viewport::session::activate_stage;

const PROJECT_REGISTRY_PATH_ENV: &str = "USDHUB_PROJECT_WORKSPACE_REGISTRY";

/// Host-owned locator for the machine-local Project registry.
#[derive(Debug, Resource)]
pub(super) struct ProjectStageActivationRuntime {
    registry_path: Option<PathBuf>,
}

impl ProjectStageActivationRuntime {
    pub(super) fn from_environment() -> Self {
        let registry_path = std::env::var_os(PROJECT_REGISTRY_PATH_ENV).map(PathBuf::from);
        if registry_path.is_none() {
            bevy::log::warn!(
                "[project-activation] {PROJECT_REGISTRY_PATH_ENV} is not configured; Project stage activation is unavailable"
            );
        }
        Self { registry_path }
    }

    fn resolve(
        &self,
        request: &ProjectActivationRequest,
    ) -> Result<Option<crate::project::service::ProjectStageActivationTarget>, String> {
        let Some(registry_path) = self.registry_path.as_ref() else {
            return Err("Project activation registry is unavailable".to_owned());
        };
        let service = ProjectApplicationService::open(registry_path.clone())
            .map_err(|error| format!("Project activation service is unavailable: {error}"))?;
        service
            .resolve_stage_activation(request.command.project_id, request.command.root.clone())
            .map_err(|error| format!("Project root activation was rejected: {error}"))
    }
}

pub(super) fn install(app: &mut App) {
    app.insert_resource(ProjectStageActivationRuntime::from_environment())
        .add_systems(
            Update,
            process_project_activations.before(crate::viewport::session::spawn_when_ready),
        );
}

/// Resolves and applies all queued Project activations on the Bevy main world.
///
/// The candidate Stage is opened before the current LiveStage is replaced, so
/// a failed activation leaves the previous renderer state untouched.
pub(super) fn process_project_activations(world: &mut World) {
    let Some(interface_resource) = world.get_resource::<RenderServerInterface>() else {
        return;
    };
    let interface = interface_resource.shared();
    while let Some(request) = interface.pop_project_activation() {
        let command = request.command.clone();
        let reply = match world
            .resource::<ProjectStageActivationRuntime>()
            .resolve(&request)
        {
            Ok(None) => ProjectActivationReply::activated(&command),
            Ok(Some(target)) => match activate_stage(world, target.path) {
                Ok(()) => ProjectActivationReply::activated(&command),
                Err(error) => ProjectActivationReply::failed(&command, error),
            },
            Err(error) => ProjectActivationReply::failed(&command, error),
        };
        let result = RoutedProjectActivationResult {
            session_id: request.session_id,
            reply,
        };
        if let Err(error) = interface.publish_project_activation_result(result) {
            bevy::log::error!(
                "[project-activation] could not publish activation result: {error:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_protocol::{ProjectActivationCommand, ProjectActivationReply};
    use usd_project::{ProjectId, ProjectRoot, SceneId};
    use viewport_protocol::SessionId;

    #[test]
    fn activation_reply_preserves_project_root_generation_and_has_no_path() {
        let project_id = ProjectId::new_v4();
        let command = ProjectActivationCommand::new(
            "activation-b",
            7,
            project_id,
            ProjectRoot::Scene(SceneId::new_v4()),
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
        let command =
            ProjectActivationCommand::new("activation-a", 1, project_id, ProjectRoot::Empty);
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
}
