use std::collections::HashSet;

use project_protocol::{
    ProjectReadCommand, ProjectReadRequest, ProjectReadResponse, ProjectStageTarget,
};
use tempfile::tempdir;
use usd_project::ProjectContentNode;

use crate::project::service::ProjectApplicationService;

use super::*;

#[test]
fn c2_freezes_proj_t_hierarchy_and_scoped_identity_lookup() {
    let directory = tempdir().unwrap();
    let mut service =
        ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
    let fixture = fixture::create(&mut service, &directory.path().join("projects"))
        .expect("create canonical Proj_T");

    assert_eq!(fixture.project.name, "Proj_T");
    assert_eq!(fixture.scenes.len(), 7);
    fixture.verify_readable_layers();
    let duplicate = fixture.identities_named("Sc1.1");
    assert_eq!(duplicate.len(), 2);
    assert_ne!(duplicate[0].id, duplicate[1].id);
    assert_eq!(
        duplicate
            .iter()
            .map(|scene| scene.parent)
            .collect::<HashSet<_>>(),
        HashSet::from([
            Some(fixture.identity("Sc1").id),
            Some(fixture.identity("Sc2").id),
        ])
    );

    let tree = service.execute(ProjectReadCommand::new(ProjectReadRequest::GetProjectTree(
        fixture.project.id,
    )));
    let ProjectReadResponse::ProjectTree { nodes, counts, .. } = tree.result.unwrap() else {
        panic!("canonical Project tree must be readable");
    };
    assert_eq!(counts.scenes, 8);
    let scene_ids = nodes
        .iter()
        .filter_map(|node| match node {
            ProjectContentNode::Scene { scene_id, .. } => Some(*scene_id),
            _ => None,
        })
        .collect::<HashSet<_>>();
    assert_eq!(scene_ids.len(), 8);
    assert!(scene_ids.contains(&fixture.root_scene_id));
    assert!(duplicate.iter().all(|scene| scene_ids.contains(&scene.id)));

    for scene in duplicate {
        let target = service
            .resolve_stage_activation(fixture.project.id, ProjectStageTarget::Scene(scene.id))
            .expect("scoped Scene activation resolves")
            .expect("Scene has a readable stage");
        assert_eq!(target.project_id, fixture.project.id);
        assert_eq!(target.target, ProjectStageTarget::Scene(scene.id));
        assert_eq!(
            target.path,
            fixture.scene_path(scene.id).canonicalize().unwrap()
        );
    }
}
