use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use openusd::usd::Stage;
use tempfile::tempdir;
use usd_project::{
    ProjectId, ProjectManifestV1, ProjectRoot, SceneMember, SceneMemberId, SceneMemberTarget,
};

use super::*;
use crate::project::{
    catalog::manifest_store::ManifestStore,
    model_import::{ModelImportRequest, ModelImporter, UsdModelImporter},
};

fn source(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(
        &path,
        r#"#usda 1.0
(
    defaultPrim = "Asset"
)
def Xform "Asset" (
    kind = "component"
) {
    def Xform "OpaqueChild" {}
}
"#,
    )
    .unwrap();
    path
}

#[test]
fn stable_wrapper_survives_original_source_rename() -> Result<()> {
    let project = tempdir()?;
    let original = source(project.path(), "original.usda");
    let base = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Model Project",
        ProjectRoot::Empty,
        vec![],
        vec![],
    );
    ManifestStore::write_manifest_atomic(project.path(), &base)?;
    let importer = UsdModelImporter;
    let inspection = importer.inspect(&original)?;
    let prepared = importer.prepare(ModelImportRequest {
        source: original.clone(),
        inspection,
    })?;
    let model_id = prepared.id;
    let published = publish_model_wrapper_atomic(ModelWrapperRequest {
        project_root: project.path(),
        base_manifest: &base,
        prepared: &prepared,
        set_as_root: true,
        placement: None,
    })?;

    fs::rename(&original, project.path().join("renamed-original.usda"))?;
    assert_eq!(published.id, model_id);
    assert_eq!(published.manifest.models[0].id, model_id);
    assert_eq!(
        published.manifest.models[0].storage_key.as_str(),
        "original"
    );
    assert!(published.wrapper_path.exists());
    let wrapper = Stage::open(&published.wrapper_path.to_string_lossy())?;
    assert!(wrapper.prim("/ModelRoot/Source").is_defined()?);
    assert!(
        project
            .path()
            .join(format!(".usdhub/models/{model_id}/source/model.usda"))
            .exists()
    );
    Ok(())
}

#[test]
fn repeated_scene_placements_share_model_id_but_not_member_id() -> Result<()> {
    let model_id = usd_project::ModelId::new_v4();
    let first_model = SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Model(model_id),
        name: None,
        transform: Default::default(),
    };
    let second_model = SceneMember {
        id: SceneMemberId::new_v4(),
        target: SceneMemberTarget::Model(model_id),
        name: None,
        transform: Default::default(),
    };
    assert_ne!(first_model.id, second_model.id);
    assert_eq!(first_model.target, second_model.target);
    Ok(())
}

#[test]
fn composed_model_source_remains_one_opaque_product_model() -> Result<()> {
    let project = tempdir()?;
    let original = source(project.path(), "opaque.usda");
    let base = ProjectManifestV1::new(
        ProjectId::new_v4(),
        "Opaque Model Project",
        ProjectRoot::Empty,
        vec![],
        vec![],
    );
    ManifestStore::write_manifest_atomic(project.path(), &base)?;
    let importer = UsdModelImporter;
    let inspection = importer.inspect(&original)?;
    let prepared = importer.prepare(ModelImportRequest {
        source: original,
        inspection,
    })?;
    let published = publish_model_wrapper_atomic(ModelWrapperRequest {
        project_root: project.path(),
        base_manifest: &base,
        prepared: &prepared,
        set_as_root: true,
        placement: None,
    })?;

    assert_eq!(published.manifest.models.len(), 1);
    assert_eq!(published.manifest.root, ProjectRoot::Model(prepared.id));
    assert_eq!(published.manifest.models[0].id, prepared.id);
    assert_eq!(published.manifest.models[0].storage_key.as_str(), "opaque");
    Ok(())
}
