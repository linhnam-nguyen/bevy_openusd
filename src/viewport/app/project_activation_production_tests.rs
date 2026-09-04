use std::fs;

use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{App, Update};
use project_protocol::{ProjectActivationCommand, ProjectStageTarget};
use tempfile::tempdir;
use usd_bevy::{LiveStage, PendingStageChanges, PrimEntities};
use usd_semantic::SemanticConfig;
use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe, HierarchySource};

use crate::project::cache_hydration::ActiveProjectCacheContext;
use crate::project::service::{
    ActiveProjectStage, ProjectActivationAuthority, ProjectStageActivation,
    ProjectStageActivationTarget, ProjectStagePresentationContext,
};
use crate::viewport::api::{
    ActiveHierarchyProvider, BimClassificationRecipeState, CurrentHierarchyProjection,
    SceneAnchorIndex, refresh_active_hierarchy_projection,
};
use crate::viewport::bim::BimClassificationFieldCatalogueState;
use crate::viewport::scene::{SelectedPrim, SelectedTargets};
use crate::viewport::semantic::{SemanticSyncState, SemanticWorkingStore, synchronize_live_stage};
use crate::viewport::session::{
    Spawned, StageInfo, StagePresentationContext,
    activate_open_stage_with_cache_context_for_generation, clear_active_stage_for_generation,
};
use usd_project::ProjectRoot;

#[test]
fn production_activation_keeps_live_semantic_bim_and_provider_state_coherent() {
    let directory = tempdir().expect("activation fixture directory");
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/stages/custom_attrs_extensive.usda");
    let mut paths = Vec::new();
    for generation in 1..=3 {
        let path = directory.path().join(format!("stage-{generation}.usda"));
        fs::copy(&source, &path).expect("copy activation fixture");
        paths.push(path);
    }

    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category",
        BimFieldKey::Category,
    )]);
    let mut provider = ActiveHierarchyProvider::default();
    provider.set(HierarchySource::BimClassification, Some(recipe));
    let mut app = App::new();
    app.insert_resource(provider)
        .init_resource::<BimClassificationRecipeState>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<BimClassificationFieldCatalogueState>()
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
        .add_systems(
            Update,
            (synchronize_live_stage, refresh_active_hierarchy_projection).chain(),
        );

    let project_id = usd_project::ProjectId::new_v4();
    let mut authority = ProjectActivationAuthority::default();
    let mut commands = Vec::new();
    for (index, path) in paths.into_iter().enumerate() {
        let generation = index as u64 + 1;
        let scene_id = usd_project::SceneId::new_v4();
        let command = ProjectActivationCommand::new(
            format!("production-activation-{generation}"),
            generation,
            project_id,
            ProjectStageTarget::Scene(scene_id),
        );
        assert!(authority.observe_request("production-session", &command));
        let target = ProjectStageActivationTarget {
            project_id,
            target: command.target.clone(),
            project_root: directory.path().to_path_buf(),
            path,
            presentation: ProjectStagePresentationContext::default(),
        };
        let activation = ProjectStageActivation::open("production-session", &command, target)
            .expect("production candidate opens");
        assert!(!activation.snapshot().hierarchy_paths.is_empty());
        assert!(!activation.snapshot().bim_snapshot_id.is_empty());
        let stage_path = activation.snapshot().stage_path.clone();
        activate_open_stage_with_cache_context_for_generation(
            app.world_mut(),
            stage_path.clone(),
            activation.into_stage(),
            None,
            generation,
            StagePresentationContext::default(),
        )
        .expect("production lifecycle installs candidate");
        assert!(authority.commit("production-session", &command));
        app.update();

        let live = app.world().get_non_send::<LiveStage>().expect("LiveStage");
        assert_eq!(
            live.stage.root_layer().identifier(),
            fs::canonicalize(&stage_path)
                .expect("activation path canonicalizes")
                .to_string_lossy()
        );
        assert_eq!(
            app.world().resource::<StageInfo>().activation_generation,
            generation
        );
        assert_eq!(
            app.world().resource::<StageInfo>().path,
            stage_path.to_string_lossy()
        );
        let semantic = app.world().resource::<SemanticSyncState>();
        assert_eq!(semantic.activation_generation(), generation);
        assert!(semantic.snapshot().is_some());
        assert!(semantic.shared_bim_index().is_some());
        assert_eq!(
            app.world().resource::<ActiveHierarchyProvider>().source(),
            HierarchySource::BimClassification
        );
        assert_eq!(
            app.world()
                .resource::<CurrentHierarchyProjection>()
                .source(),
            HierarchySource::BimClassification
        );
        commands.push(command);
    }

    let active = authority.active().expect("active authority");
    assert_eq!(
        active,
        &ActiveProjectStage {
            project_id,
            target: commands[2].target.clone(),
            generation: 3,
        }
    );
    let empty = ProjectActivationCommand::new(
        "production-empty",
        4,
        project_id,
        ProjectStageTarget::ProjectRoot(ProjectRoot::Empty),
    );
    assert!(authority.observe_request("production-session", &empty));
    clear_active_stage_for_generation(app.world_mut(), 4);
    assert!(authority.commit("production-session", &empty));
    assert!(app.world().get_non_send::<LiveStage>().is_none());
    assert!(
        app.world()
            .get_resource::<ActiveProjectCacheContext>()
            .is_none()
    );
    assert_eq!(app.world().resource::<StageInfo>().activation_generation, 4);
    assert!(app.world().resource::<StageInfo>().path.is_empty());
    assert!(
        app.world()
            .resource::<SemanticSyncState>()
            .snapshot()
            .is_none()
    );
    assert!(
        app.world()
            .resource::<SemanticSyncState>()
            .shared_bim_index()
            .is_none()
    );
    assert!(
        app.world()
            .resource::<CurrentHierarchyProjection>()
            .snapshot()
            .nodes
            .is_empty()
    );
    assert_eq!(
        authority.active(),
        Some(&ActiveProjectStage {
            project_id,
            target: empty.target,
            generation: 4,
        })
    );
}
