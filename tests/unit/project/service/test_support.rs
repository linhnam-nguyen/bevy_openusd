//! Local unit test support for ProjectApplicationService.

use std::fs;
use std::path::Path;

use usd_project::ProjectSummary;

use super::ProjectApplicationService;

/// Create a temporary workspace and project for service-level testing.
pub(crate) fn service_with_project(
    directory: &Path,
) -> (ProjectApplicationService, ProjectSummary) {
    let parent = directory.join("projects");
    fs::create_dir(&parent).unwrap();
    let mut service = ProjectApplicationService::open(directory.join("workspace.json")).unwrap();
    let project = service.create_project(&parent, "Project").unwrap();
    (service, project)
}
