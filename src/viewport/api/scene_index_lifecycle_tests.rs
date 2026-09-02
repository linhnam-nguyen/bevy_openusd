use std::collections::BTreeSet;

use bevy::app::App;
use project_protocol::ProjectStageTarget;
use tempfile::tempdir;
use usd_bevy::{LiveStagePlugin, PrimEntities, UsdPlugin};
use usd_project::ProjectRoot;

use super::*;
use crate::project::catalog::manifest_store::ManifestStore;
use crate::project::service::ProjectApplicationService;
use crate::viewport::session::{
    ReloadRequest, Spawned, StageInfo, StagePresentationContext,
    activate_stage_with_cache_context_for_generation, handle_usd_hot_reload, spawn_when_ready,
};

fn assert_project_names(app: &App, root_name: &str) {
    let index = app.world().resource::<SceneAnchorIndex>();
    let root = index
        .roots_read_model()
        .prims
        .into_iter()
        .find(|node| node.anchor.prim_path == "/SceneRoot")
        .expect("canonical Scene root is in the protocol read model");
    assert_eq!(root.display_name.as_deref(), Some(root_name));
    let visible_names = index
        .nodes
        .iter()
        .filter_map(|node| node.display_name.as_deref())
        .collect::<BTreeSet<_>>();
    for expected in [root_name, "Sc1", "Sc2", "Kitchen_set"] {
        assert!(
            visible_names.contains(expected),
            "semantic name {expected} is present in the protocol hierarchy"
        );
    }
    assert!(
        app.world()
            .resource::<CurrentHierarchyProjection>()
            .snapshot()
            .nodes
            .iter()
            .all(|node| !node.name.starts_with("Member_"))
    );
    assert!(
        index
            .roots_read_model()
            .prims
            .iter()
            .chain(index.nodes.iter())
            .all(|node| !node
                .display_name
                .as_deref()
                .is_some_and(|name| name.starts_with("Member_")))
    );
    let physical_member_anchors = index
        .by_anchor
        .keys()
        .filter(|anchor| anchor.prim_path.contains("/Member_"))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !physical_member_anchors.is_empty(),
        "physical Project member anchors remain indexed"
    );
    assert!(
        physical_member_anchors
            .iter()
            .all(|anchor| index.resolve(anchor).is_some()),
        "transparent semantic wrappers remain selectable by physical anchor"
    );
}

fn update_until_projected(app: &mut App) {
    for _ in 0..8 {
        app.update();
    }
}

#[test]
fn project_and_direct_scene_activation_keep_root_authority_through_refresh() {
    let directory = tempdir().expect("temporary Project directory");
    let projects = directory.path().join("projects");
    std::fs::create_dir(&projects).expect("Project parent directory");
    let mut service = ProjectApplicationService::open(directory.path().join("workspace.json"))
        .expect("Project service opens");
    let project = service
        .create_project(&projects, "Pro3")
        .expect("Project creates");
    let root_scene_id = match project.root {
        ProjectRoot::Scene(scene_id) => scene_id,
        _ => panic!("new Project has a protected Scene root"),
    };
    let sc1 = service
        .create_scene(
            project.id,
            project_protocol::ProjectWriteTarget::Scene(root_scene_id),
            "Sc1",
        )
        .expect("Sc1 creates");
    let _sc2 = service
        .create_scene(
            project.id,
            project_protocol::ProjectWriteTarget::Scene(root_scene_id),
            "Sc2",
        )
        .expect("Sc2 creates");
    service
        .create_scene(
            project.id,
            project_protocol::ProjectWriteTarget::Scene(sc1.scene_id),
            "Kitchen_set",
        )
        .expect("Kitchen_set creates");

    let project_root = projects.join("Pro3");
    let mut manifest = ManifestStore::read_validated(&project_root)
        .expect("canonical manifest reads")
        .raw()
        .clone();
    manifest
        .scenes
        .iter_mut()
        .find(|scene| scene.id == root_scene_id)
        .expect("protected root is registered")
        .display_name = "Sc3".to_owned();
    ManifestStore::write_manifest_atomic(&project_root, &manifest).expect("manifest updates");
    let root_path = crate::project::scene::authoring::scene_path(&project_root, root_scene_id);
    crate::project::scene::authoring::update_display_name_atomic(&root_path, "/SceneRoot", "Sc3")
        .expect("authored root label updates");

    let project_target = service
        .resolve_stage_activation(
            project.id,
            ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(root_scene_id)),
        )
        .expect("Project root resolves")
        .expect("Project root has a Stage");
    let direct_target = service
        .resolve_stage_activation(project.id, ProjectStageTarget::Scene(root_scene_id))
        .expect("direct Scene resolves")
        .expect("direct Scene has a Stage");
    let mut app = App::new();
    app.add_plugins(UsdPlugin)
        .add_plugins(LiveStagePlugin)
        .init_resource::<SceneAnchorIndex>()
        .init_resource::<CurrentHierarchyProjection>()
        .init_resource::<PrimEntities>()
        .init_resource::<Spawned>()
        .init_resource::<StageInfo>()
        .add_systems(
            bevy::app::Update,
            (spawn_when_ready, refresh_scene_anchor_index)
                .chain()
                .after(usd_bevy::LiveStageSet::Reconcile),
        );
    app.world_mut()
        .insert_resource(ReloadRequest { requested: false });
    activate_stage_with_cache_context_for_generation(
        app.world_mut(),
        project_target.path.clone(),
        None,
        1,
        StagePresentationContext::from_project(project_target.presentation),
    )
    .expect("Project root activation opens the canonical Stage");
    update_until_projected(&mut app);
    assert_project_names(&app, "Pro3");

    activate_stage_with_cache_context_for_generation(
        app.world_mut(),
        direct_target.path.clone(),
        None,
        2,
        StagePresentationContext::from_project(direct_target.presentation.clone()),
    )
    .expect("direct Scene activation opens the canonical Stage");
    update_until_projected(&mut app);
    assert_project_names(&app, "Sc3");

    app.world_mut().resource_mut::<ReloadRequest>().requested = true;
    handle_usd_hot_reload(app.world_mut());
    update_until_projected(&mut app);
    assert_project_names(&app, "Sc3");

    let project_context = service
        .resolve_stage_activation(
            project.id,
            ProjectStageTarget::ProjectRoot(ProjectRoot::Scene(root_scene_id)),
        )
        .expect("Project root resolves after refresh")
        .expect("Project root remains available");
    activate_stage_with_cache_context_for_generation(
        app.world_mut(),
        project_context.path,
        None,
        3,
        StagePresentationContext::from_project(project_context.presentation),
    )
    .expect("Project root activation remains available after Refresh");
    update_until_projected(&mut app);
    assert_project_names(&app, "Pro3");

    app.world_mut().resource_mut::<ReloadRequest>().requested = true;
    handle_usd_hot_reload(app.world_mut());
    update_until_projected(&mut app);
    assert_project_names(&app, "Pro3");
}
