//! Direct-source and Project-adopted BIM equivalence for the canonical Revit
//! export. The comparison deliberately keys on semantic identity values, not
//! on the namespace paths introduced by Project Scene composition.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use openusd::usd::Stage;
use project_protocol::{
    PlacementSpec, ProjectActivationCommand, ProjectStageTarget, ProjectWriteTarget,
};
use tempfile::tempdir;
use usd_model::{SemanticSnapshot, SnapshotSource};
use usd_semantic::{SemanticConfig, SemanticExtractor};
use viewport_protocol::{BimFieldKey, ClassificationLevel, ClassificationRecipe, SceneAnchor};

use crate::project::scene::authoring::scene_path;
use crate::project::service::{
    ProjectApplicationService, ProjectStageActivationTarget, ProjectStagePresentationContext,
};

fn extract(path: &Path, session: &str) -> Result<SemanticSnapshot, String> {
    let stage = Stage::open(&path.to_string_lossy())
        .map_err(|error| format!("open {}: {error}", path.display()))?;
    SemanticExtractor::new(SemanticConfig::for_nvidia_revit_connector())
        .extract(
            &stage,
            SnapshotSource::Working {
                session: session.to_owned(),
                live_revision: 1,
            },
        )
        .map_err(|error| format!("extract {}: {error}", path.display()))
}

fn bim_identity_signature(
    snapshot: &SemanticSnapshot,
) -> BTreeMap<String, (String, String, String)> {
    snapshot
        .entities
        .values()
        .filter(|entity| entity.semantic.is_bim_entity())
        .filter_map(|entity| {
            let element_id = entity.semantic.bim.element_id.clone()?;
            Some((
                element_id,
                (
                    entity
                        .semantic
                        .bim_classification
                        .category
                        .clone()
                        .unwrap_or_default(),
                    entity
                        .semantic
                        .bim_classification
                        .type_name
                        .clone()
                        .unwrap_or_default(),
                    entity.semantic.bim.family_name.clone().unwrap_or_default(),
                ),
            ))
        })
        .collect()
}

fn classification_labels(snapshot: &SemanticSnapshot) -> Result<Vec<String>, String> {
    let recipe = ClassificationRecipe::new(vec![ClassificationLevel::new(
        "category",
        BimFieldKey::Category,
    )]);
    let projection = crate::viewport::bim::BimReadService::new(snapshot)
        .classification_projection(&recipe)
        .map_err(|error| error.to_string())?;
    Ok(projection
        .snapshot()
        .nodes
        .iter()
        .map(|node| node.name.clone())
        .collect())
}

fn assert_bim_views_are_equivalent(
    direct: &SemanticSnapshot,
    adopted: &SemanticSnapshot,
    direct_labels: &[String],
) {
    let adopted_labels = classification_labels(adopted).expect("adopted BIM projection");
    assert_classification_labels_are_meaningful(&adopted_labels, "adopted Scene");
    assert_eq!(
        bim_identity_signature(adopted),
        bim_identity_signature(direct),
        "Project adoption preserves canonical BIM identity/classification values"
    );
    assert_eq!(
        adopted_labels, direct_labels,
        "Project adoption preserves the real BIM service classification result"
    );
}

/// Returns a semantic path so activation is tested with a real BIM selection,
/// rather than relying on the synthetic stage root.
fn first_bim_entity_path(snapshot: &SemanticSnapshot) -> String {
    snapshot
        .entities
        .values()
        .find(|entity| entity.semantic.is_bim_entity())
        .map(|entity| entity.prim_path.clone())
        .expect("adopted Scene has a selectable BIM entity")
}

/// Rejects an empty or placeholder classification result before equivalence
/// comparison can hide a missing BIM projection.
fn assert_classification_labels_are_meaningful(labels: &[String], source: &str) {
    assert!(!labels.is_empty(), "{source} has BIM classifications");
    assert!(
        labels.iter().all(|label| !label.trim().is_empty()),
        "{source} classification labels are not blank"
    );
}

fn canonical_revit_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../Instance2/external_assets/Omniverse/V1/Projet1.usdc")
}

#[test]
fn direct_revit_and_adopted_scene_bim_are_equivalent_after_activation() {
    let source = canonical_revit_source();
    assert!(source.is_file(), "canonical Revit source exists");
    let direct = extract(&source, "or8-direct").expect("direct Revit semantic snapshot");
    assert!(
        !bim_identity_signature(&direct).is_empty(),
        "canonical Revit source contains BIM identity evidence"
    );
    let direct_labels = classification_labels(&direct).expect("direct BIM projection");
    assert_classification_labels_are_meaningful(&direct_labels, "canonical source");

    let directory = tempdir().expect("equivalence temp directory");
    let projects_root = directory.path().join("projects");
    fs::create_dir(&projects_root).expect("equivalence projects directory");
    let mut service = ProjectApplicationService::open(directory.path().join("workspace.json"))
        .expect("equivalence Project service");
    let project = service
        .create_project(&projects_root, "Projet1-equivalence")
        .expect("equivalence Project");
    let inspection = crate::project::scene::inspection::inspect_composition(&source)
        .expect("inspect canonical Revit source");
    let adopted = service
        .adopt_scene(
            project.id,
            ProjectWriteTarget::Project(project.id),
            &source,
            &inspection,
            "Projet1 imported".to_owned(),
            "or8-m2-source-equivalence".to_owned(),
            1,
            PlacementSpec::Default,
        )
        .expect("normal Project Scene adoption");
    let project_root = projects_root.join("Projet1-equivalence");
    let adopted_path = scene_path(&project_root, adopted.scene_id);
    let adopted_snapshot =
        extract(&adopted_path, "or8-adopted").expect("adopted Scene semantic snapshot");
    assert_bim_views_are_equivalent(&direct, &adopted_snapshot, &direct_labels);

    let first_element_path = first_bim_entity_path(&adopted_snapshot);
    let mut production = crate::viewport::ProductionActivationWorld::new();
    production.replace_selection(SceneAnchor::active_session(first_element_path));
    let command = ProjectActivationCommand::new(
        "or8-m2-source-equivalence-activation",
        1,
        project.id,
        ProjectStageTarget::Scene(adopted.scene_id),
    );
    assert!(production.admit("or8-equivalence-session", &command));
    let target = ProjectStageActivationTarget {
        project_id: project.id,
        target: command.target.clone(),
        project_root,
        path: adopted_path.clone(),
        archive_paths: Vec::new(),
        cache_identity: None,
        presentation: ProjectStagePresentationContext::default(),
    };
    let reply = production.apply(
        "or8-equivalence-session",
        &command,
        Ok(Some(target.clone())),
    );
    assert!(matches!(
        reply.result,
        project_protocol::ProjectActivationResult::Activated { .. }
    ));
    for _ in 0..10_000 {
        production.update();
        if production
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness()
            == usd_bevy::ProjectionReadiness::Ready
        {
            break;
        }
    }
    assert_eq!(
        production
            .world()
            .resource::<usd_bevy::ProgressiveProjectionState>()
            .readiness(),
        usd_bevy::ProjectionReadiness::Ready,
        "adopted Scene projection reaches readiness before BIM observation"
    );
    let observation = production
        .observe(&adopted_path, 1)
        .expect("activated adopted Scene BIM state");
    assert!(observation.property_rows > 0);
    assert_eq!(
        observation.hierarchy_source,
        viewport_protocol::HierarchySource::BimClassification
    );
}
