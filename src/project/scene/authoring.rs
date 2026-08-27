use std::{
    collections::HashMap,
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use openusd::{sdf::Value, usd::Stage};
use usd_project::SceneId;
use uuid::Uuid;

const PROJECT_METADATA_DIRECTORY: &str = ".usdhub";
const SCENES_DIRECTORY: &str = "scenes";
const SCENE_ROOT_PRIM: &str = "SceneRoot";
const SCENE_ID_METADATA: &str = "usdhub:sceneId";
const SCHEMA_VERSION_METADATA: &str = "usdhub:schemaVersion";
const SCENE_SCHEMA_VERSION: i32 = 1;

pub(crate) fn author_scene_atomic(project_root: &Path, scene_id: SceneId) -> Result<PathBuf> {
    let scene_directory = project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(SCENES_DIRECTORY);
    fs::create_dir_all(&scene_directory).context("create Project Scene directory")?;

    let final_path = scene_path(project_root, scene_id);
    let temporary_path = scene_directory.join(format!(".{scene_id}.{}.tmp.usda", Uuid::new_v4()));
    let mut temporary_created = false;

    let result = (|| {
        let stage = new_scene_stage(scene_id)?;
        let temporary_path_string = temporary_path.to_string_lossy().into_owned();
        temporary_created = true;
        stage
            .root_layer()
            .export(&temporary_path_string)
            .context("export temporary Project Scene layer")?;
        validate_scene_file(&temporary_path, scene_id)?;
        fs::rename(&temporary_path, &final_path).with_context(|| {
            format!(
                "publish temporary Project Scene {} as {}",
                temporary_path.display(),
                final_path.display()
            )
        })?;
        sync_parent_best_effort(final_path.parent());
        Ok(final_path.clone())
    })();

    if result.is_err() && temporary_created {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

pub(crate) fn scene_path(project_root: &Path, scene_id: SceneId) -> PathBuf {
    project_root
        .join(PROJECT_METADATA_DIRECTORY)
        .join(SCENES_DIRECTORY)
        .join(format!("{scene_id}.usda"))
}

fn new_scene_stage(scene_id: SceneId) -> Result<Stage> {
    let stage = Stage::builder().in_memory(format!("scene-{scene_id}.usda"))?;
    let mut custom_data = HashMap::new();
    custom_data.insert(
        SCENE_ID_METADATA.to_owned(),
        Value::String(scene_id.to_string()),
    );
    custom_data.insert(
        SCHEMA_VERSION_METADATA.to_owned(),
        Value::Int(SCENE_SCHEMA_VERSION),
    );

    stage
        .define_prim(format!("/{SCENE_ROOT_PRIM}").as_str())?
        .set_type_name("Xform")?
        .set_metadata("customData", Value::Dictionary(custom_data))?;
    stage.set_default_prim(SCENE_ROOT_PRIM)?;
    Ok(stage)
}

fn validate_scene_file(path: &Path, expected_scene_id: SceneId) -> Result<()> {
    let path_string = path.to_string_lossy().into_owned();
    let stage = Stage::open(&path_string).context("reopen exported Project Scene layer")?;
    let default_prim = stage.default_prim();
    ensure!(
        default_prim
            .as_ref()
            .is_some_and(|token| token.as_str() == SCENE_ROOT_PRIM),
        "Project Scene defaultPrim must be /{SCENE_ROOT_PRIM}"
    );

    let root = stage.prim(format!("/{SCENE_ROOT_PRIM}").as_str());
    ensure!(
        root.is_defined()?,
        "Project Scene root prim must be defined"
    );
    let Some(Value::Dictionary(custom_data)) = root.custom_data()? else {
        bail!("Project Scene root prim is missing customData");
    };

    let Some(scene_id_value) = custom_data.get(SCENE_ID_METADATA) else {
        bail!("Project Scene root prim is missing {SCENE_ID_METADATA}");
    };
    let Some(scene_id_text) = scene_id_value.as_str() else {
        bail!("Project Scene {SCENE_ID_METADATA} must be a string");
    };
    ensure!(
        SceneId::parse(scene_id_text)? == expected_scene_id,
        "Project Scene metadata identity does not match the registry identity"
    );

    ensure!(
        custom_data.get(SCHEMA_VERSION_METADATA) == Some(&Value::Int(SCENE_SCHEMA_VERSION)),
        "Project Scene schema version is unsupported or missing"
    );
    Ok(())
}

fn sync_parent_best_effort(parent: Option<&Path>) {
    let Some(parent) = parent else {
        return;
    };
    let Ok(directory) = File::open(parent) else {
        return;
    };
    let _ = directory.sync_all();
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use openusd::{sdf::Value, usd::Stage};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn authored_scene_reopens_with_default_prim_and_registry_identity() -> Result<()> {
        let project_directory = tempdir()?;
        let scene_id = SceneId::new_v4();

        let path = author_scene_atomic(project_directory.path(), scene_id)?;

        assert_eq!(path, scene_path(project_directory.path(), scene_id));
        let path_string = path.to_string_lossy().into_owned();
        let stage = Stage::open(&path_string)?;
        assert_eq!(
            stage.default_prim().as_ref().map(|token| token.as_str()),
            Some(SCENE_ROOT_PRIM)
        );

        let root = stage.prim(format!("/{SCENE_ROOT_PRIM}").as_str());
        assert!(root.is_defined()?);
        let Some(Value::Dictionary(custom_data)) = root.custom_data()? else {
            panic!("SceneRoot customData should be authored");
        };
        assert_eq!(
            custom_data.get(SCENE_ID_METADATA),
            Some(&Value::String(scene_id.to_string()))
        );
        assert_eq!(
            custom_data.get(SCHEMA_VERSION_METADATA),
            Some(&Value::Int(SCENE_SCHEMA_VERSION))
        );
        validate_scene_file(&path, scene_id)?;

        let scene_directory = path.parent().expect("scene directory");
        assert!(fs::read_dir(scene_directory)?.all(|entry| {
            entry
                .expect("read Project Scene directory entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".usda")
        }));
        Ok(())
    }
}
