//! Application service for conservative Scene lifecycle mutations.

use project_protocol::{
    ProjectDeleteSceneRequest, ProjectSceneLifecycleResponse, ProjectWriteError,
    ProjectWriteErrorCode,
};

use super::ProjectApplicationService;

impl ProjectApplicationService {
    pub fn delete_scene(
        &mut self,
        request: ProjectDeleteSceneRequest,
    ) -> Result<ProjectSceneLifecycleResponse, ProjectWriteError> {
        super::deletion::delete_scene(self, request)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/project/service/scene_lifecycle_tests.rs"]
mod tests;
