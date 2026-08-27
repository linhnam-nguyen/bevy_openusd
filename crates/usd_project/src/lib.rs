//! Pure Project domain contracts shared by Project adapters and read models.

mod capability;
mod error;
mod id;
mod manifest;
mod read_model;
mod root;

#[cfg(test)]
mod validation;

pub use capability::ProjectCapabilities;
pub use error::ProjectDomainError;
pub use id::{ModelId, ProjectId, SceneId, SceneMemberId};
pub use manifest::{
    ModelManifestEntry, PROJECT_MANIFEST_SCHEMA_VERSION, ProjectManifestError, ProjectManifestV1,
    SceneManifestEntry, StorageKey, ValidatedProjectManifest,
};
pub use read_model::{
    BranchSummary, ModelSourceSummary, ProjectContentCounts, ProjectContentNode, ProjectSummary,
    RepositorySummary, RevisionSummary,
};
pub use root::{ModelSourceKind, ProjectRoot};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_ids_round_trip_through_serde() {
        let project_id = ProjectId::new_v4();
        let scene_id = SceneId::new_v4();
        let member_id = SceneMemberId::new_v4();
        let model_id = ModelId::new_v4();

        assert_eq!(
            project_id,
            serde_json::from_str(&serde_json::to_string(&project_id).unwrap()).unwrap()
        );
        assert_eq!(
            scene_id,
            serde_json::from_str(&serde_json::to_string(&scene_id).unwrap()).unwrap()
        );
        assert_eq!(
            member_id,
            serde_json::from_str(&serde_json::to_string(&member_id).unwrap()).unwrap()
        );
        assert_eq!(
            model_id,
            serde_json::from_str(&serde_json::to_string(&model_id).unwrap()).unwrap()
        );
    }

    #[test]
    fn malformed_and_nil_ids_are_rejected() {
        assert!(ProjectId::parse("not-a-uuid").is_err());
        assert!(SceneId::parse("00000000-0000-0000-0000-000000000000").is_err());
        assert!(serde_json::from_str::<ModelId>("\"not-a-uuid\"").is_err());
    }

    #[test]
    fn project_root_preserves_typed_identity() {
        let root = ProjectRoot::Scene(SceneId::new_v4());
        let encoded = serde_json::to_string(&root).unwrap();
        let decoded: ProjectRoot = serde_json::from_str(&encoded).unwrap();

        assert_eq!(root, decoded);
    }

    #[test]
    fn empty_external_source_kinds_are_rejected() {
        assert!(ModelSourceKind::external("").is_err());
        assert!(ModelSourceKind::external("   ").is_err());
        assert!(ModelSourceKind::External(String::new()).validate().is_err());
    }
}
