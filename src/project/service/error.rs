use project_protocol::ProjectReadErrorCode;

use crate::project::catalog::catalogue::ProjectCatalogueUnavailableReason;

pub(super) fn project_read_error_code(
    reason: ProjectCatalogueUnavailableReason,
) -> ProjectReadErrorCode {
    match reason {
        ProjectCatalogueUnavailableReason::ManifestUnavailable => {
            ProjectReadErrorCode::ManifestUnavailable
        }
        ProjectCatalogueUnavailableReason::RepositoryMissing => {
            ProjectReadErrorCode::RepositoryMissing
        }
        ProjectCatalogueUnavailableReason::RepositoryPermissionDenied => {
            ProjectReadErrorCode::RepositoryPermissionDenied
        }
        ProjectCatalogueUnavailableReason::InvalidManifest => ProjectReadErrorCode::InvalidManifest,
        ProjectCatalogueUnavailableReason::RegistryIdentityMismatch => {
            ProjectReadErrorCode::RegistryIdentityMismatch
        }
    }
}
