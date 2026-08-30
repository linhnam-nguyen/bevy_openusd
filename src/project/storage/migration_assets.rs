use std::{collections::HashMap, fs, path::Path};

use anyhow::{Context, Result, bail, ensure};
use openusd::{sdf, sdf::Value, usd::Stage};
use usd_project::{ModelId, ProjectManifestV1, ProjectRoot, SceneId, SceneMemberTarget};

use crate::project::scene::authoring;
use crate::project::storage::ProjectStorageLayout;

const REFERENCES_FIELD: &str = "references";

pub(super) struct AssetMap {
    scene_targets: HashMap<SceneId, std::path::PathBuf>,
    model_targets: HashMap<ModelId, std::path::PathBuf>,
    rules: Vec<(std::path::PathBuf, std::path::PathBuf)>,
}

impl AssetMap {
    pub(super) fn from_manifest(project_root: &Path, manifest: &ProjectManifestV1) -> Result<Self> {
        let layout = ProjectStorageLayout::new(project_root);
        let mut map = Self {
            scene_targets: HashMap::with_capacity(manifest.scenes.len()),
            model_targets: HashMap::with_capacity(manifest.models.len()),
            rules: Vec::new(),
        };
        for scene in &manifest.scenes {
            let target = if manifest.root == ProjectRoot::Scene(scene.id) {
                layout.canonical_root_scene_path(&scene.storage_key)
            } else {
                layout.canonical_scene_path(&scene.storage_key)
            };
            map.scene_targets.insert(scene.id, target.clone());
            map.rules.push((layout.legacy_scene_path(scene.id), target));
            map.rules.push((
                layout.legacy_scene_import_dir(scene.id),
                layout.canonical_scene_import_dir(scene.id),
            ));
        }
        for model in &manifest.models {
            let target = layout.canonical_model_wrapper_path(model);
            map.model_targets.insert(model.id, target.clone());
            map.rules
                .push((layout.legacy_model_wrapper_path(model.id), target.clone()));
            map.rules.push((
                layout
                    .legacy_model_wrapper_path(model.id)
                    .parent()
                    .expect("legacy Model wrapper has a parent")
                    .to_owned(),
                target
                    .parent()
                    .expect("canonical Model wrapper has a parent")
                    .to_owned(),
            ));
            map.rules.push((
                layout.legacy_model_import_dir(model.id),
                layout.canonical_model_import_dir(model.id),
            ));
        }
        map.rules
            .sort_by_key(|(old, _)| std::cmp::Reverse(old.components().count()));
        Ok(map)
    }

    pub(super) fn add_rule(&mut self, old: std::path::PathBuf, new: std::path::PathBuf) {
        self.rules.push((old, new));
        self.rules
            .sort_by_key(|(old, _)| std::cmp::Reverse(old.components().count()));
    }

    pub(super) fn target_path(&self, target: SceneMemberTarget) -> Result<&Path> {
        match target {
            SceneMemberTarget::Scene(id) => self
                .scene_targets
                .get(&id)
                .map(std::path::PathBuf::as_path)
                .with_context(|| format!("migrated Scene target {id} is not registered")),
            SceneMemberTarget::Model(id) => self
                .model_targets
                .get(&id)
                .map(std::path::PathBuf::as_path)
                .with_context(|| format!("migrated Model target {id} is not registered")),
        }
    }

    fn map_path(&self, old_path: &Path) -> Option<std::path::PathBuf> {
        self.rules.iter().find_map(|(old, new)| {
            old_path
                .strip_prefix(old)
                .ok()
                .map(|suffix| new.join(suffix))
        })
    }
}

pub(super) fn author_scene_migration(
    old_path: &Path,
    staged_path: &Path,
    final_path: &Path,
    project_root: &Path,
    scene_id: SceneId,
    asset_map: &AssetMap,
) -> Result<()> {
    let members = authoring::read_scene_members(old_path, scene_id)
        .context("read legacy Project Scene members before migration")?;
    let old_path_string = old_path.to_string_lossy().into_owned();
    let stage = Stage::open(&old_path_string).context("open legacy Project Scene layer")?;
    authoring::prepare_scene_for_direct_members(&stage)
        .context("prepare legacy Project Scene for direct members")?;
    rewrite_source_references(&stage, old_path, final_path, project_root, asset_map)?;
    for member in &members {
        let target_path = asset_map.target_path(member.target.clone())?;
        authoring::author_scene_member_at_path(
            &stage,
            project_root,
            final_path,
            member,
            Some(target_path),
        )?;
    }
    if let Some(parent) = staged_path.parent() {
        fs::create_dir_all(parent).context("create Scene migration staging directory")?;
    }
    stage
        .root_layer()
        .export(staged_path.to_string_lossy().as_ref())
        .context("stage migrated Project Scene layer")?;
    authoring::validate_scene_file(staged_path, scene_id, &members)
        .context("validate migrated Project Scene layer")?;
    Ok(())
}

pub(super) fn author_model_migration(
    old_path: &Path,
    staged_path: &Path,
    final_path: &Path,
    project_root: &Path,
    model_id: ModelId,
    asset_map: &AssetMap,
) -> Result<()> {
    let old_path_string = old_path.to_string_lossy().into_owned();
    let stage = Stage::open(&old_path_string).context("open legacy Model wrapper")?;
    let root = stage.prim("/ModelRoot");
    let Some(Value::Dictionary(data)) = root.custom_data()? else {
        bail!("legacy Model wrapper root is missing customData");
    };
    ensure!(
        data.get("usdhub:modelId") == Some(&Value::String(model_id.to_string())),
        "legacy Model wrapper identity does not match its manifest"
    );
    rewrite_source_references(&stage, old_path, final_path, project_root, asset_map)?;
    if let Some(parent) = staged_path.parent() {
        fs::create_dir_all(parent).context("create Model migration staging directory")?;
    }
    stage
        .root_layer()
        .export(staged_path.to_string_lossy().as_ref())
        .context("stage migrated Model wrapper")?;
    Ok(())
}

fn rewrite_source_references(
    stage: &Stage,
    old_layer_path: &Path,
    final_layer_path: &Path,
    project_root: &Path,
    asset_map: &AssetMap,
) -> Result<()> {
    for prim_path in ["/SceneRoot/Source", "/ModelRoot/Source"] {
        let prim = stage.prim(prim_path);
        if !prim.is_defined()? {
            continue;
        }
        let spec_path = sdf::path(prim_path)?;
        let references = {
            let root_layer = stage.root_layer();
            let Some(spec) = root_layer.prim(&spec_path) else {
                continue;
            };
            let Some(Value::ReferenceListOp(references)) = spec.field(REFERENCES_FIELD)? else {
                continue;
            };
            references.iter().cloned().collect::<Vec<_>>()
        };
        let mut rewritten = Vec::with_capacity(references.len());
        for mut reference in references {
            let resolved = resolve_reference_path(old_layer_path, &reference.asset_path)?;
            let mapped = asset_map.map_path(&resolved).with_context(|| {
                format!(
                    "legacy Project reference does not map to a Project-owned asset: {}",
                    resolved.display()
                )
            })?;
            reference.asset_path = crate::project::storage::authored_relative_project_asset_path(
                project_root,
                final_layer_path,
                &mapped,
            )?;
            rewritten.push(reference);
        }
        prim.set_metadata(
            REFERENCES_FIELD,
            Value::ReferenceListOp(sdf::ReferenceListOp::prepended(rewritten)),
        )?;
    }
    Ok(())
}

fn resolve_reference_path(authoring_layer: &Path, asset_path: &str) -> Result<std::path::PathBuf> {
    let asset_path = Path::new(asset_path);
    if asset_path.is_absolute() {
        return Ok(asset_path.to_owned());
    }
    authoring_layer
        .parent()
        .context("legacy USD layer has no parent directory")
        .map(|parent| parent.join(asset_path))
}
