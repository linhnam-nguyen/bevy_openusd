use serde::{Deserialize, Serialize};
use usd_project::{ProjectContentNode, ProjectId, ProjectSummary, RepositorySummary};

use crate::ProjectReadErrorCode;

/// Read-only Project queries supported by protocol V1.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectReadRequest {
    ListProjects,
    GetProjectTree(ProjectId),
    GetProjectRepositorySummary(ProjectId),
}

/// One item in the Project catalogue. Unavailable entries retain the
/// registry-owned identity while exposing only a typed diagnostic code.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectListItem {
    Available(ProjectSummary),
    Unavailable {
        project_id: ProjectId,
        code: ProjectReadErrorCode,
    },
}

/// Read models returned by the Project application boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProjectReadResponse {
    Projects(Vec<ProjectListItem>),
    ProjectTree {
        project_id: ProjectId,
        nodes: Vec<ProjectContentNode>,
    },
    RepositorySummary {
        project_id: ProjectId,
        repository: RepositorySummary,
    },
}
