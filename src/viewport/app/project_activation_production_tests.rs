use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use bevy::asset::AssetApp;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::image::Image;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::{App, Update, World};
use project_protocol::{ProjectActivationCommand, ProjectActivationReply, ProjectStageTarget};
use tempfile::tempdir;
use usd_bevy::{LiveStage, PendingStageChanges, PrimEntities};
use usd_semantic::SemanticConfig;
use viewport_protocol::{
    BimFieldKey, ClassificationLevel, ClassificationRecipe, HierarchySource, SceneAnchor,
    SelectionReadModel,
};
use viewport_streaming::ProjectActivationRequest;

use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::project::service::{
    ActiveProjectStage, ProjectStageActivationTarget, ProjectStagePresentationContext,
};
use crate::viewport::api::{
    ActiveHierarchyProvider, BimClassificationRecipeState, CurrentHierarchyProjection,
    SceneAnchorIndex, refresh_active_hierarchy_projection, refresh_scene_anchor_index,
};
use crate::viewport::bim::BimClassificationFieldCatalogueState;
use crate::viewport::scene::{SelectedPrim, SelectedTargets};
use crate::viewport::semantic::{SemanticSyncState, SemanticWorkingStore, synchronize_live_stage};
use crate::viewport::session::rehydrate_activation_presentation;
use crate::viewport::session::{Spawned, StageInfo, StagePresentationContext};
use usd_project::ProjectRoot;

pub(crate) struct ProductionActivationWorld {
    app: App,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActivationObservation {
    pub(crate) stage_path: PathBuf,
    pub(crate) generation: u64,
    pub(crate) semantic_snapshot_id: String,
    pub(crate) bim_snapshot_id: String,
    pub(crate) hierarchy_source: HierarchySource,
    pub(crate) hierarchy_nodes: usize,
    pub(crate) property_rows: usize,
}

impl ProductionActivationWorld {
    pub(crate) fn new() -> Self {
        let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
            "category",
            BimFieldKey::Category,
        )]);
        let mut provider = ActiveHierarchyProvider::default();
        provider.set(HierarchySource::BimClassification, Some(recipe));
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins)
            .add_plugins(bevy::asset::AssetPlugin::default())
            .add_plugins(usd_bevy::UsdPlugin)
            .add_plugins(usd_bevy::LiveStagePlugin)
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
            .insert_resource(usd_bevy::ProjectionBudget::bounded(
                32,
                Duration::from_millis(8),
            ))
            .insert_resource(provider)
            .init_resource::<BimClassificationRecipeState>()
            .init_resource::<CurrentHierarchyProjection>()
            .init_resource::<SceneAnchorIndex>()
            .init_resource::<BimClassificationFieldCatalogueState>()
            .init_resource::<crate::viewport::animation::UsdStageTime>()
            .insert_resource(SemanticSyncState::with_config(
                SemanticConfig::for_nvidia_revit_connector(),
            ))
            .insert_resource(SemanticWorkingStore::default())
            .insert_resource(PendingStageChanges::default())
            .insert_resource(PrimEntities::default())
            .insert_resource(SelectedTargets::default())
            .insert_resource(SelectedPrim::default())
            .insert_resource(Spawned::default())
            .insert_resource(StageInfo::default())
            .insert_resource(StagePresentationContext::default())
            .insert_resource(super::ProjectActivationAuthorityRuntime::default())
            .add_systems(
                Update,
                (
                    refresh_scene_anchor_index,
                    synchronize_live_stage,
                    refresh_active_hierarchy_projection,
                    rehydrate_activation_presentation,
                )
                    .chain()
                    .after(usd_bevy::LiveStageSet::Presentation),
            );
        app.add_systems(
            Update,
            crate::viewport::animation::tick_stage_time.after(usd_bevy::LiveStageSet::Presentation),
        );
        Self { app }
    }

    pub(crate) fn admit(&mut self, session_id: &str, command: &ProjectActivationCommand) -> bool {
        crate::viewport::observe_project_activation_for_test(
            self.app.world_mut(),
            session_id,
            command,
        )
    }

    pub(crate) fn apply(
        &mut self,
        session_id: &str,
        command: &ProjectActivationCommand,
        target: Result<Option<ProjectStageActivationTarget>, String>,
    ) -> ProjectActivationReply {
        let request = ProjectActivationRequest {
            session_id: viewport_protocol::SessionId::new(session_id),
            command: command.clone(),
        };
        crate::viewport::apply_prepared_activation_for_test(self.app.world_mut(), &request, target)
    }

    pub(crate) fn update(&mut self) {
        self.app.update();
    }

    pub(crate) fn replace_selection(&mut self, target: SceneAnchor) {
        self.app
            .world_mut()
            .resource_mut::<SelectedTargets>()
            .replace(SelectionReadModel {
                targets: vec![target.clone()],
                primary: Some(target),
            })
            .expect("production test selection is valid");
    }

    pub(crate) fn world(&self) -> &World {
        self.app.world()
    }

    pub(crate) fn active(&self) -> Option<ActiveProjectStage> {
        self.world()
            .resource::<super::ProjectActivationAuthorityRuntime>()
            .0
            .active()
            .cloned()
    }

    fn assert_empty_activation(
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

    pub(crate) fn observe(
        &self,
        path: &Path,
        generation: u64,
    ) -> Result<ActivationObservation, String> {
        let live = self
            .world()
            .get_non_send::<LiveStage>()
            .ok_or_else(|| "production activation did not install LiveStage".to_owned())?;
        let expected_path = fs::canonicalize(path).map_err(|error| error.to_string())?;
        if live.stage.root_layer().identifier() != expected_path.to_string_lossy() {
            return Err("LiveStage path does not match the prepared target".to_owned());
        }
        let info = self.world().resource::<StageInfo>();
        if info.activation_generation != generation || info.path != path.to_string_lossy() {
            return Err("StageInfo does not match the active generation".to_owned());
        }
        let semantic = self.world().resource::<SemanticSyncState>();
        let semantic_snapshot_id = semantic
            .snapshot()
            .map(|snapshot| snapshot.snapshot_id.0.clone())
            .ok_or_else(|| "semantic snapshot is missing after activation".to_owned())?;
        let bim_snapshot_id = semantic
            .shared_bim_index()
            .map(|index| index.snapshot_id.0.clone())
            .ok_or_else(|| "BIM index is missing after activation".to_owned())?;
        let projection = self.world().resource::<CurrentHierarchyProjection>();
        if self.world().resource::<ActiveHierarchyProvider>().source()
            != HierarchySource::BimClassification
            || projection.source() != HierarchySource::BimClassification
        {
            return Err("hierarchy provider did not publish the BIM projection".to_owned());
        }
        let hierarchy_nodes = projection.snapshot().nodes.len();
        if hierarchy_nodes == 0 {
            return Err("hierarchy projection is empty after activation".to_owned());
        }
        let selection = self.world().resource::<SelectedTargets>();
        if selection.0.targets.is_empty() {
            return Err("retained selection was not rehydrated after activation".to_owned());
        }
        let snapshot = semantic
            .snapshot()
            .expect("semantic snapshot checked above");
        let bim_index = semantic
            .shared_bim_index()
            .expect("BIM index checked above");
        let properties = crate::viewport::bim::BimReadService::with_index(snapshot, bim_index)
            .read_properties(
                &selection.0,
                selection.revision(),
                crate::viewport::bim::BimReadPolicy {
                    allow_value_edit: true,
                },
            )
            .map_err(|error| {
                format!("BIM property service could not read retained selection: {error}")
            })?;
        let property_rows = properties
            .groups
            .iter()
            .map(|group| group.properties.len())
            .sum();
        if property_rows == 0 {
            return Err("retained BIM selection has no readable properties".to_owned());
        }
        Ok(ActivationObservation {
            stage_path: path.to_path_buf(),
            generation,
            semantic_snapshot_id,
            bim_snapshot_id,
            hierarchy_source: projection.source(),
            hierarchy_nodes,
            property_rows,
        })
    }
}

#[test]
fn production_activation_keeps_live_semantic_bim_and_provider_state_coherent() {
    let directory = tempdir().expect("activation fixture directory");
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages/production_activation_bim.usda");
    let mut paths = Vec::new();
    for generation in 1..=3 {
        let path = directory.path().join(format!("stage-{generation}.usda"));
        fs::copy(&source, &path).expect("copy activation fixture");
        paths.push(path);
    }

    let mut production = ProductionActivationWorld::new();
    production.replace_selection(SceneAnchor::active_session("/World/Wall"));

    let project_id = usd_project::ProjectId::new_v4();
    let mut commands = Vec::new();
    let mut stale_completion = None;
    let mut latest_target = None;
    for (index, path) in paths.into_iter().enumerate() {
        let generation = index as u64 + 1;
        let scene_id = usd_project::SceneId::new_v4();
        let command = ProjectActivationCommand::new(
            format!("production-activation-{generation}"),
            generation,
            project_id,
            ProjectStageTarget::Scene(scene_id),
        );
        assert!(production.admit("production-session", &command));
        let target = ProjectStageActivationTarget {
            project_id,
            target: command.target.clone(),
            project_root: directory.path().to_path_buf(),
            path,
            archive_paths: Vec::new(),
            cache_identity: None,
            presentation: ProjectStagePresentationContext::default(),
        };
        if index == 1 {
            stale_completion = Some((command.clone(), target.clone()));
        }
        latest_target = Some(target.clone());
        let reply = production.apply("production-session", &command, Ok(Some(target.clone())));
        assert!(matches!(
            reply.result,
            project_protocol::ProjectActivationResult::Activated { .. }
        ));
        production.update();
        let observation = production
            .observe(&target.path, generation)
            .expect("production resources remain coherent");
        assert_eq!(observation.generation, generation);
        assert_eq!(
            observation.hierarchy_source,
            HierarchySource::BimClassification
        );
        commands.push(command);
    }

    let active = production.active().expect("active authority");
    assert_eq!(
        active,
        ActiveProjectStage {
            project_id,
            target: commands[2].target.clone(),
            generation: 3,
        }
    );
    let (stale_command, stale_target) = stale_completion.expect("stale completion");
    let latest_target = latest_target.expect("latest target");
    let before_stale = production
        .observe(&latest_target.path, 3)
        .expect("generation 3 resources before stale completion");
    let reply = production.apply("production-session", &stale_command, Ok(Some(stale_target)));
    assert!(matches!(
        reply.result,
        project_protocol::ProjectActivationResult::Failed { .. }
    ));
    let after_stale = production
        .observe(&latest_target.path, 3)
        .expect("generation 3 resources after stale completion");
    assert_eq!(before_stale, after_stale);

    let empty = ProjectActivationCommand::new(
        "production-empty",
        4,
        project_id,
        ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
    );
    assert!(production.admit("production-session", &empty));
    let reply = production.apply("production-session", &empty, Ok(None));
    assert!(matches!(
        reply.result,
        project_protocol::ProjectActivationResult::Activated { .. }
    ));
    production.assert_empty_activation(project_id, empty.target);
}
