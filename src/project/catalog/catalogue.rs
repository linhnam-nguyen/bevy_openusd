use std::cmp::Ordering;

use usd_project::{
    ProjectCapabilities, ProjectContentCounts, ProjectId, ProjectSummary, RepositorySummary,
};

use super::{
    manifest_store::ManifestStore,
    workspace_registry::{WorkspaceProjectEntry, WorkspaceRegistry},
};

const UNAVAILABLE_MANIFEST_REASON: &str = "Project manifest unavailable";

/// One cheap catalogue result without exposing a machine-local repository path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProjectCatalogueItem {
    Available(ProjectSummary),
    Unavailable {
        project_id: ProjectId,
        reason: &'static str,
    },
}

/// List registered Projects from metadata manifests without opening USD stages.
pub(crate) fn list_projects(registry: &WorkspaceRegistry) -> Vec<ProjectCatalogueItem> {
    let mut items = Vec::with_capacity(registry.entries().len());
    for entry in registry.entries() {
        items.push(catalogue_item(entry));
    }
    items.sort_by(compare_items);
    items
}

fn catalogue_item(entry: &WorkspaceProjectEntry) -> ProjectCatalogueItem {
    match ManifestStore::read_validated(entry.repository_locator()) {
        Ok(manifest) => ProjectCatalogueItem::Available(ProjectSummary {
            id: manifest.raw().project_id,
            name: manifest.raw().name.clone(),
            root: manifest.raw().root.clone(),
            repository: RepositorySummary {
                active_branch: None,
                branches: Vec::new(),
                dirty: false,
                head: None,
            },
            counts: ProjectContentCounts {
                scenes: manifest.scenes().len() as u64,
                models: manifest.models().len() as u64,
                scene_placements: 0,
                model_placements: 0,
            },
            capabilities: ProjectCapabilities::default(),
        }),
        Err(_) => ProjectCatalogueItem::Unavailable {
            project_id: entry.project_id(),
            reason: UNAVAILABLE_MANIFEST_REASON,
        },
    }
}

fn compare_items(left: &ProjectCatalogueItem, right: &ProjectCatalogueItem) -> Ordering {
    match (left, right) {
        (ProjectCatalogueItem::Available(left), ProjectCatalogueItem::Available(right)) => left
            .name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id)),
        (
            ProjectCatalogueItem::Unavailable {
                project_id: left, ..
            },
            ProjectCatalogueItem::Unavailable {
                project_id: right, ..
            },
        ) => left.cmp(right),
        (ProjectCatalogueItem::Available(_), ProjectCatalogueItem::Unavailable { .. }) => {
            Ordering::Less
        }
        (ProjectCatalogueItem::Unavailable { .. }, ProjectCatalogueItem::Available(_)) => {
            Ordering::Greater
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use usd_project::{ProjectManifestV1, ProjectRoot};

    use super::*;
    use crate::project::catalog::{
        manifest_store::ManifestStore, workspace_registry::WorkspaceRegistry,
    };

    #[test]
    fn lists_many_manifests_without_failing_for_one_missing_repository() {
        let directory = tempdir().unwrap();
        let registry_path = directory.path().join("workspace.json");
        let mut registry = WorkspaceRegistry::load(registry_path).unwrap();

        for index in 0..100 {
            let project_id = ProjectId::new_v4();
            let project_root = directory.path().join(format!("project-{index}"));
            let manifest = ProjectManifestV1::new(
                project_id,
                format!("Project {index:03}"),
                ProjectRoot::Empty,
                Vec::new(),
                Vec::new(),
            );
            ManifestStore::write_manifest_atomic(&project_root, &manifest).unwrap();
            registry.register(project_id, project_root, None).unwrap();
        }

        let missing_project_id = ProjectId::new_v4();
        let missing_root = directory.path().join("missing-repository");
        registry
            .register(missing_project_id, &missing_root, None)
            .unwrap();

        let items = list_projects(&registry);
        assert_eq!(items.len(), 101);
        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(item, ProjectCatalogueItem::Available(_)))
                .count(),
            100
        );
        assert!(items.iter().any(|item| {
            matches!(
                item,
                ProjectCatalogueItem::Unavailable { project_id, .. }
                    if *project_id == missing_project_id
            )
        }));
        let encoded = format!("{items:?}");
        assert!(!encoded.contains(&missing_root.to_string_lossy().to_string()));
    }

    #[test]
    fn output_is_sorted_by_product_name_then_project_id() {
        let directory = tempdir().unwrap();
        let mut registry =
            WorkspaceRegistry::load(directory.path().join("workspace.json")).unwrap();
        let names = [("Zulu", "zulu"), ("Alpha", "alpha"), ("Bravo", "bravo")];

        for (name, folder) in names {
            let project_id = ProjectId::new_v4();
            let project_root = directory.path().join(folder);
            let manifest = ProjectManifestV1::new(
                project_id,
                name,
                ProjectRoot::Empty,
                Vec::new(),
                Vec::new(),
            );
            ManifestStore::write_manifest_atomic(&project_root, &manifest).unwrap();
            registry.register(project_id, project_root, None).unwrap();
        }

        let names = list_projects(&registry)
            .into_iter()
            .map(|item| match item {
                ProjectCatalogueItem::Available(summary) => summary.name,
                ProjectCatalogueItem::Unavailable { .. } => unreachable!(),
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["Alpha", "Bravo", "Zulu"]);
        assert!(fs::read_dir(directory.path()).unwrap().count() >= 4);
    }

    #[test]
    fn catalogue_source_has_no_stage_open_or_adapter_scan_path() {
        let source = include_str!("catalogue.rs");
        let forbidden = [
            ["Stage", "::", "open"].concat(),
            ["open", "usd"].concat(),
            ["g", "ix"].concat(),
            ["t", "urso"].concat(),
            ["render", "er"].concat(),
        ];

        for token in forbidden {
            assert!(
                !source.contains(&token),
                "forbidden catalogue token: {token}"
            );
        }
    }
}
