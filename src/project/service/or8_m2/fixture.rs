//! Canonical OR8 M2 Project and scoped Scene identity fixture.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use project_protocol::{ProjectWriteError, ProjectWriteTarget};
use usd_project::{ProjectRoot, ProjectSummary, SceneId};

use crate::project::{
    catalog::manifest_store::ManifestStore,
    scene::authoring::{read_scene_members, scene_path},
    service::ProjectApplicationService,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SceneIdentity {
    pub label: String,
    pub id: SceneId,
    pub parent: Option<SceneId>,
}

#[derive(Clone, Debug)]
pub(super) struct CanonicalProject {
    pub project: ProjectSummary,
    pub root: PathBuf,
    pub root_scene_id: SceneId,
    pub scenes: Vec<SceneIdentity>,
}

impl CanonicalProject {
    pub(super) fn identities_named(&self, label: &str) -> Vec<&SceneIdentity> {
        self.scenes
            .iter()
            .filter(|scene| scene.label == label)
            .collect()
    }

    pub(super) fn identity(&self, label: &str) -> &SceneIdentity {
        self.scenes
            .iter()
            .find(|scene| scene.label == label)
            .expect("canonical Scene label exists")
    }

    pub(super) fn scene_path(&self, scene_id: SceneId) -> PathBuf {
        scene_path(&self.root, scene_id)
    }

    pub(super) fn verify_readable_layers(&self) {
        for scene in &self.scenes {
            assert!(
                self.scene_path(scene.id).is_file(),
                "Scene layer is readable"
            );
            read_scene_members(&self.scene_path(scene.id), scene.id)
                .expect("Scene members are readable");
        }
        assert!(self.scene_path(self.root_scene_id).is_file());
    }
}

pub(super) fn create(
    service: &mut ProjectApplicationService,
    projects_root: &Path,
) -> Result<CanonicalProject, ProjectWriteError> {
    fs::create_dir_all(projects_root).map_err(|_| project_protocol::ProjectWriteError::Failed {
        code: project_protocol::ProjectWriteErrorCode::FilesystemFailure,
    })?;
    let project = service.create_project(projects_root, "Proj_T")?;
    let root = projects_root.join("Proj_T");
    let root_scene_id = match project.root {
        ProjectRoot::Scene(scene_id) => scene_id,
        ProjectRoot::Empty | ProjectRoot::Model(_) => {
            return Err(project_protocol::ProjectWriteError::Failed {
                code: project_protocol::ProjectWriteErrorCode::InvalidRootForComposition,
            });
        }
    };

    let sc1 = service.create_scene(project.id, ProjectWriteTarget::Project(project.id), "Sc1")?;
    let sc2 = service.create_scene(project.id, ProjectWriteTarget::Project(project.id), "Sc2")?;
    let sc1_1_left = service.create_scene(
        project.id,
        ProjectWriteTarget::Scene(sc1.scene_id),
        "Sc1_1_left",
    )?;
    let sc1_2 =
        service.create_scene(project.id, ProjectWriteTarget::Scene(sc1.scene_id), "Sc1.2")?;
    let sc1_2_3 = service.create_scene(
        project.id,
        ProjectWriteTarget::Scene(sc1_2.scene_id),
        "Sc1.2.3",
    )?;
    let sc2_1 =
        service.create_scene(project.id, ProjectWriteTarget::Scene(sc2.scene_id), "Sc2.1")?;
    let sc1_1_right = service.create_scene(
        project.id,
        ProjectWriteTarget::Scene(sc2.scene_id),
        "Sc1_1_right",
    )?;

    for scene_id in [sc1_1_left.scene_id, sc1_1_right.scene_id] {
        service.rename(project.id, ProjectWriteTarget::Scene(scene_id), "Sc1.1")?;
    }

    let scenes = vec![
        SceneIdentity {
            label: "Sc1".to_owned(),
            id: sc1.scene_id,
            parent: Some(root_scene_id),
        },
        SceneIdentity {
            label: "Sc1.1".to_owned(),
            id: sc1_1_left.scene_id,
            parent: Some(sc1.scene_id),
        },
        SceneIdentity {
            label: "Sc1.2".to_owned(),
            id: sc1_2.scene_id,
            parent: Some(sc1.scene_id),
        },
        SceneIdentity {
            label: "Sc1.2.3".to_owned(),
            id: sc1_2_3.scene_id,
            parent: Some(sc1_2.scene_id),
        },
        SceneIdentity {
            label: "Sc2".to_owned(),
            id: sc2.scene_id,
            parent: Some(root_scene_id),
        },
        SceneIdentity {
            label: "Sc2.1".to_owned(),
            id: sc2_1.scene_id,
            parent: Some(sc2.scene_id),
        },
        SceneIdentity {
            label: "Sc1.1".to_owned(),
            id: sc1_1_right.scene_id,
            parent: Some(sc2.scene_id),
        },
    ];
    let fixture = CanonicalProject {
        project,
        root,
        root_scene_id,
        scenes,
    };
    verify_manifest_names(&fixture)?;
    fixture.verify_readable_layers();
    Ok(fixture)
}

fn verify_manifest_names(fixture: &CanonicalProject) -> Result<(), ProjectWriteError> {
    let manifest = ManifestStore::read_validated(&fixture.root).map_err(|_| {
        project_protocol::ProjectWriteError::Failed {
            code: project_protocol::ProjectWriteErrorCode::ManifestUnavailable,
        }
    })?;
    let expected: HashMap<SceneId, &str> = fixture
        .scenes
        .iter()
        .map(|scene| (scene.id, scene.label.as_str()))
        .collect();
    for scene in manifest.scenes() {
        if scene.id == fixture.root_scene_id {
            assert_eq!(scene.display_name, "Proj_T");
        } else if let Some(expected_name) = expected.get(&scene.id) {
            assert_eq!(&scene.display_name, expected_name);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn canonical_fixture_has_two_distinct_scoped_sc1_1_identities() {
        let directory = tempdir().unwrap();
        let mut service =
            ProjectApplicationService::open(directory.path().join("workspace.json")).unwrap();
        let fixture = create(&mut service, &directory.path().join("projects")).unwrap();
        let duplicate = fixture.identities_named("Sc1.1");
        assert_eq!(duplicate.len(), 2);
        assert_ne!(duplicate[0].id, duplicate[1].id);
        assert_ne!(duplicate[0].parent, duplicate[1].parent);
    }
}
