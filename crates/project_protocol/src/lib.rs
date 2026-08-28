//! Shared, adapter-neutral Project read protocol.
//!
//! This crate is intentionally limited to serde data transfer objects. It
//! does not know about Tauri, Git implementations, filesystems, OpenUSD, or
//! renderer state.

mod command;
mod error;
mod location;
mod read;
mod write;

pub use command::{PROJECT_READ_PROTOCOL_VERSION, ProjectReadCommand, ProjectReadReply};
pub use error::{ProjectReadError, ProjectReadErrorCode};
pub use location::{
    LocalSelectionToken, LocalSelectionView, ProjectLocationKind, ProjectLocationResult,
};
pub use read::{ProjectListItem, ProjectReadRequest, ProjectReadResponse};
pub use write::{
    PROJECT_WRITE_PROTOCOL_VERSION, ProjectCreateRequest, ProjectCreateSceneRequest,
    ProjectImportRequest, ProjectInspection, ProjectInspectionClassification,
    ProjectInspectionWarning, ProjectSceneWriteResponse, ProjectWriteCommand, ProjectWriteError,
    ProjectWriteErrorCode, ProjectWriteReply, ProjectWriteRequest, ProjectWriteResponse,
    ProjectWriteTarget,
};

#[cfg(test)]
mod tests {
    use usd_project::{ProjectId, ProjectSummary};

    use super::*;

    #[test]
    fn command_and_reply_round_trip_as_shared_json() {
        let project_id = ProjectId::new_v4();
        let command = ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(project_id));
        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: ProjectReadCommand = serde_json::from_str(&encoded).unwrap();
        assert_eq!(command, decoded);

        let reply = ProjectReadReply::success(ProjectReadResponse::Projects(vec![
            ProjectListItem::Available(ProjectSummary {
                id: project_id,
                name: "Project".to_owned(),
                root: usd_project::ProjectRoot::Empty,
                repository: usd_project::RepositorySummary {
                    active_branch: None,
                    branches: Vec::new(),
                    dirty: false,
                    head: None,
                },
                counts: usd_project::ProjectContentCounts::default(),
                capabilities: usd_project::ProjectCapabilities::default(),
            }),
        ]));
        let encoded = serde_json::to_string(&reply).unwrap();
        let decoded: ProjectReadReply = serde_json::from_str(&encoded).unwrap();
        assert_eq!(reply, decoded);
    }

    #[test]
    fn invalid_protocol_version_is_typed_and_does_not_need_a_path() {
        let command = ProjectReadCommand {
            protocol_version: PROJECT_READ_PROTOCOL_VERSION + 1,
            request: ProjectReadRequest::ListProjects,
        };

        assert_eq!(
            command.validate().unwrap_err(),
            ProjectReadError::UnsupportedProtocolVersion {
                expected: PROJECT_READ_PROTOCOL_VERSION,
                actual: PROJECT_READ_PROTOCOL_VERSION + 1,
            }
        );
        let encoded = serde_json::to_string(&command.validate().unwrap_err()).unwrap();
        assert!(!encoded.contains("/"));
    }

    #[test]
    fn local_selection_wire_contract_does_not_contain_a_filesystem_path() {
        let result = ProjectLocationResult::Selected(LocalSelectionView {
            token: LocalSelectionToken::new("session-token"),
            display_name: "Selected Project Folder".to_owned(),
        });
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(encoded.contains("session-token"));
        assert!(!encoded.contains("PathBuf"));
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn write_command_round_trip_keeps_selection_and_fingerprint_opaque() {
        let command = ProjectWriteCommand::new(ProjectWriteRequest::Import(ProjectImportRequest {
            selection: LocalSelectionToken::new("selection-token"),
            inspection: ProjectInspection {
                classification: ProjectInspectionClassification::AdoptableGit,
                display_name: "Project".to_owned(),
                warnings: vec![ProjectInspectionWarning::MissingLocalCacheRoots],
                fingerprint: "opaque-fingerprint".to_owned(),
            },
        }));
        let encoded = serde_json::to_string(&command).unwrap();
        let decoded: ProjectWriteCommand = serde_json::from_str(&encoded).unwrap();

        assert_eq!(command, decoded);
        assert!(!encoded.contains("PathBuf"));
        assert!(!encoded.contains("/Users/"));
    }

    #[test]
    fn create_scene_command_round_trips_typed_target_and_result() {
        let project_id = ProjectId::new_v4();
        let command = ProjectWriteCommand::new(ProjectWriteRequest::CreateScene(
            ProjectCreateSceneRequest {
                project_id,
                target: ProjectWriteTarget::Project(project_id),
                name: "Main Scene".to_owned(),
            },
        ));
        let encoded = serde_json::to_string(&command).unwrap();
        assert!(!encoded.contains("PathBuf"));
        assert_eq!(
            command,
            serde_json::from_str::<ProjectWriteCommand>(&encoded).unwrap()
        );
    }
}
