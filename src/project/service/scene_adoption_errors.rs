use project_protocol::ProjectWriteErrorCode;

use crate::project::scene::adoption_support::{AdoptionPhaseError, SourceRevalidationError};

pub(super) fn source_revalidation_error_code(
    error: &SourceRevalidationError,
) -> ProjectWriteErrorCode {
    match error {
        SourceRevalidationError::Changed => ProjectWriteErrorCode::SourceChanged,
        SourceRevalidationError::CompositionValidation(_) => {
            ProjectWriteErrorCode::CompositionValidationFailed
        }
        SourceRevalidationError::ClassificationRejected(_) => {
            ProjectWriteErrorCode::SourceClassificationRejected
        }
    }
}

/// Convert internal adoption phases into stable, path-free protocol codes.
/// The inner error is retained for logs, but never crosses the host boundary.
pub(super) fn adoption_error_code(error: &anyhow::Error) -> ProjectWriteErrorCode {
    for cause in error.chain() {
        if let Some(source_error) = cause.downcast_ref::<SourceRevalidationError>() {
            return source_revalidation_error_code(source_error);
        }
        if let Some(phase_error) = cause.downcast_ref::<AdoptionPhaseError>() {
            match phase_error {
                AdoptionPhaseError::ClassificationRejected(_) => {
                    return ProjectWriteErrorCode::SourceClassificationRejected;
                }
                AdoptionPhaseError::DependencyLocalization(_) => {
                    return ProjectWriteErrorCode::DependencyLocalizationFailed;
                }
                AdoptionPhaseError::CompositionValidation(_) => {
                    return ProjectWriteErrorCode::CompositionValidationFailed;
                }
                AdoptionPhaseError::Publication(_) => {}
            }
        }
    }
    ProjectWriteErrorCode::ScenePublicationFailed
}

#[cfg(test)]
mod tests {
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
                AdoptionPhaseError::Publication(anyhow::anyhow!(
                    "rename failed at /private/project"
                )),
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
}
