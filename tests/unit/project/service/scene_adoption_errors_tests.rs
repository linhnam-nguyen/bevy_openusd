use super::*;

#[test]
fn adoption_phases_map_to_path_free_protocol_codes() {
    let cases = [
        (
            AdoptionPhaseError::ClassificationRejected(anyhow::anyhow!(
                "dependency at /private/source.usda"
            )),
            ProjectWriteErrorCode::SourceClassificationRejected,
        ),
        (
            AdoptionPhaseError::DependencyLocalization(anyhow::anyhow!(
                "filesystem failure at /private/source.usda"
            )),
            ProjectWriteErrorCode::DependencyLocalizationFailed,
        ),
        (
            AdoptionPhaseError::CompositionValidation(anyhow::anyhow!(
                "invalid wrapper at /private/scene.usda"
            )),
            ProjectWriteErrorCode::CompositionValidationFailed,
        ),
        (
            AdoptionPhaseError::Publication(anyhow::anyhow!("rename failed at /private/project")),
            ProjectWriteErrorCode::ScenePublicationFailed,
        ),
    ];

    for (phase, expected) in cases {
        let code = adoption_error_code(&anyhow::Error::new(phase));
        assert_eq!(code, expected);
        let wire = serde_json::to_string(&project_protocol::ProjectWriteError::Failed { code })
            .expect("protocol error serializes");
        assert!(!wire.contains("/private/"));
    }
}
