use std::path::Path;

use project_protocol::{ProjectRenameResponse, ProjectWriteError, ProjectWriteTarget};

use super::{PlacementWriter, ProjectApplicationService, rename_with_writer};

struct FailingAfterPlacementWriter {
    fail_after: usize,
    rewritten: usize,
}

impl PlacementWriter for FailingAfterPlacementWriter {
    fn update_member_display_name_atomic(
        &mut self,
        scene_path: &Path,
        scene_id: usd_project::SceneId,
        member_id: usd_project::SceneMemberId,
        display_name: &str,
    ) -> anyhow::Result<()> {
        if self.rewritten == self.fail_after {
            anyhow::bail!(
                "injected placement rewrite failure after {}",
                self.rewritten
            );
        }
        crate::project::scene::authoring::update_member_display_name_atomic(
            scene_path,
            scene_id,
            member_id,
            display_name,
        )?;
        self.rewritten += 1;
        Ok(())
    }
}

pub(super) fn rename_failing_after_placement(
    service: &mut ProjectApplicationService,
    project_id: usd_project::ProjectId,
    target: ProjectWriteTarget,
    requested_name: &str,
    fail_after: usize,
) -> Result<ProjectRenameResponse, ProjectWriteError> {
    let mut writer = FailingAfterPlacementWriter {
        fail_after,
        rewritten: 0,
    };
    rename_with_writer(service, project_id, target, requested_name, &mut writer)
}
