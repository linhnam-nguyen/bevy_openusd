//! Authorization policy types kept separate from device capabilities.

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

/// A delivery path the server may authorize for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Deliver rendered pixels from the server.
    Stream,
    /// Deliver an authorized runtime manifest for local rendering.
    SelfRender,
}

/// Whether the client may receive runtime model blobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDownloadPermission {
    Denied,
    Allowed,
}

/// The semantic-property access granted to a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "paths")]
pub enum SemanticPropertyScope {
    /// Do not expose semantic property values.
    None,
    /// Expose only the exact property paths listed here.
    AllowList(Vec<String>),
    /// Expose all semantic property values permitted by the server.
    All,
}

/// Whether committed history may be queried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryPermission {
    Denied,
    ReadOnly,
}

/// An authorization-selected runtime profile, independent of hardware probe results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfile {
    /// The authoritative renderer remains on the server.
    ServerStream,
    /// Local rendering is allowed with a balanced quality/performance target.
    NativeMedium,
    /// Local rendering is allowed with a higher quality target.
    NativeHigh,
}

/// Server-owned permissions for one application session.
///
/// This policy describes what the client may receive. It does not describe
/// what the client device can decode or render; those facts remain in
/// [`crate::ClientCapabilities`] and [`crate::ServerCapabilities`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationPolicy {
    pub allowed_delivery_modes: Vec<DeliveryMode>,
    pub model_download: ModelDownloadPermission,
    pub semantic_property_scope: SemanticPropertyScope,
    pub history: HistoryPermission,
    pub runtime_profile: RuntimeProfile,
}

impl AuthorizationPolicy {
    /// The restrictive visitor policy: streamed pixels only and no history or
    /// runtime model possession.
    pub fn stream_only() -> Self {
        Self {
            allowed_delivery_modes: vec![DeliveryMode::Stream],
            model_download: ModelDownloadPermission::Denied,
            semantic_property_scope: SemanticPropertyScope::None,
            history: HistoryPermission::Denied,
            runtime_profile: RuntimeProfile::ServerStream,
        }
    }

    /// Validate cross-field authorization invariants before sending the policy.
    pub fn validate(&self) -> Result<(), AuthorizationValidationError> {
        if self.allowed_delivery_modes.is_empty() {
            return Err(AuthorizationValidationError::NoDeliveryModes);
        }

        if self.allows_delivery_mode(DeliveryMode::SelfRender)
            && self.model_download == ModelDownloadPermission::Denied
        {
            return Err(AuthorizationValidationError::SelfRenderRequiresModelDownload);
        }

        if let SemanticPropertyScope::AllowList(paths) = &self.semantic_property_scope {
            if paths.iter().any(|path| path.trim().is_empty()) {
                return Err(AuthorizationValidationError::EmptySemanticPropertyPath);
            }
        }

        match self.runtime_profile {
            RuntimeProfile::ServerStream if !self.allows_delivery_mode(DeliveryMode::Stream) => {
                Err(AuthorizationValidationError::ServerStreamRequiresStream)
            }
            RuntimeProfile::NativeMedium | RuntimeProfile::NativeHigh
                if !self.allows_delivery_mode(DeliveryMode::SelfRender) =>
            {
                Err(AuthorizationValidationError::NativeProfileRequiresSelfRender)
            }
            _ => Ok(()),
        }
    }

    /// Returns whether this policy grants the requested delivery path.
    pub fn allows_delivery_mode(&self, mode: DeliveryMode) -> bool {
        self.allowed_delivery_modes.contains(&mode)
    }

    /// Returns whether this policy grants possession of runtime model blobs.
    pub fn allows_model_download(&self) -> bool {
        self.model_download == ModelDownloadPermission::Allowed
    }

    /// Returns whether this policy grants committed-history queries.
    pub fn allows_history(&self) -> bool {
        self.history == HistoryPermission::ReadOnly
    }

    /// Returns whether the exact semantic property path may be exposed.
    pub fn allows_semantic_property(&self, path: &str) -> bool {
        match &self.semantic_property_scope {
            SemanticPropertyScope::None => false,
            SemanticPropertyScope::AllowList(paths) => paths.iter().any(|item| item == path),
            SemanticPropertyScope::All => true,
        }
    }

    /// Used by serde to omit the restrictive default from legacy handshakes.
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

impl Default for AuthorizationPolicy {
    fn default() -> Self {
        Self::stream_only()
    }
}

/// Cross-field validation failures for an authorization policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationValidationError {
    NoDeliveryModes,
    SelfRenderRequiresModelDownload,
    EmptySemanticPropertyPath,
    ServerStreamRequiresStream,
    NativeProfileRequiresSelfRender,
}

impl fmt::Display for AuthorizationValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NoDeliveryModes => "authorization policy must allow a delivery mode",
            Self::SelfRenderRequiresModelDownload => {
                "self-render delivery requires model-download permission"
            }
            Self::EmptySemanticPropertyPath => {
                "semantic-property allow-list contains an empty path"
            }
            Self::ServerStreamRequiresStream => {
                "server-stream runtime profile requires stream delivery"
            }
            Self::NativeProfileRequiresSelfRender => {
                "native runtime profile requires self-render delivery"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for AuthorizationValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_restrictive_and_valid() {
        let policy = AuthorizationPolicy::default();

        assert!(policy.validate().is_ok());
        assert!(policy.allows_delivery_mode(DeliveryMode::Stream));
        assert!(!policy.allows_delivery_mode(DeliveryMode::SelfRender));
        assert!(!policy.allows_model_download());
        assert!(!policy.allows_history());
        assert!(!policy.allows_semantic_property("xformOp:translate"));
        assert!(policy.is_default());
    }

    #[test]
    fn native_policy_requires_download_and_self_render() {
        let policy = AuthorizationPolicy {
            allowed_delivery_modes: vec![DeliveryMode::SelfRender],
            model_download: ModelDownloadPermission::Allowed,
            semantic_property_scope: SemanticPropertyScope::AllowList(vec![
                "xformOp:translate".to_owned(),
            ]),
            history: HistoryPermission::ReadOnly,
            runtime_profile: RuntimeProfile::NativeMedium,
        };

        assert!(policy.validate().is_ok());
        assert!(policy.allows_model_download());
        assert!(policy.allows_history());
        assert!(policy.allows_semantic_property("xformOp:translate"));
        assert!(!policy.allows_semantic_property("secret:cost"));
    }

    #[test]
    fn invalid_policy_rejects_self_render_without_download() {
        let policy = AuthorizationPolicy {
            allowed_delivery_modes: vec![DeliveryMode::Stream, DeliveryMode::SelfRender],
            model_download: ModelDownloadPermission::Denied,
            ..AuthorizationPolicy::default()
        };

        assert_eq!(
            policy.validate(),
            Err(AuthorizationValidationError::SelfRenderRequiresModelDownload)
        );
    }
}
