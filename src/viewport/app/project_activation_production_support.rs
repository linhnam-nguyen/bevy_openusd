use project_protocol::ProjectStageTarget;
use usd_bevy::LiveStage;

use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::project::service::ActiveProjectStage;
use crate::viewport::api::CurrentHierarchyProjection;
use crate::viewport::semantic::SemanticSyncState;
use crate::viewport::session::StageInfo;

use super::ProductionActivationWorld;

impl ProductionActivationWorld {
    pub(crate) fn assert_empty_activation(
        &self,
        project_id: usd_project::ProjectId,
        target: ProjectStageTarget,
    ) {
        let world = self.world();
        assert!(world.get_non_send::<LiveStage>().is_none());
        assert!(world.get_resource::<ActiveProjectCacheContext>().is_none());
        let stage_info = world.resource::<StageInfo>();
        assert_eq!(stage_info.activation_generation, 4);
        assert!(stage_info.path.is_empty());
        let semantic = world.resource::<SemanticSyncState>();
        assert!(semantic.snapshot().is_none());
        assert!(semantic.shared_bim_index().is_none());
        assert!(
            world
                .resource::<CurrentHierarchyProjection>()
                .snapshot()
                .nodes
                .is_empty()
        );
        assert_eq!(
            self.active(),
            Some(ActiveProjectStage {
                project_id,
                target,
                generation: 4,
            })
        );
    }
}
