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
#[path = "../../../tests/unit/project/service/scene_adoption_errors_tests.rs"]
mod tests;
