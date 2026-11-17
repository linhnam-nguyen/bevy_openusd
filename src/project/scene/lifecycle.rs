//! Application service for conservative Scene lifecycle mutations.

use project_protocol::{
    ProjectDeleteSceneRequest, ProjectSceneLifecycleResponse, ProjectWriteError,
    ProjectWriteErrorCode,
};

use crate::project::service::ProjectApplicationService;

impl ProjectApplicationService {
    pub fn delete_scene(
        &mut self,
        request: ProjectDeleteSceneRequest,
    ) -> Result<ProjectSceneLifecycleResponse, ProjectWriteError> {
        crate::project::lifecycle::deletion::delete_scene(self, request)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/project/scene/lifecycle_tests.rs"]
mod tests;
