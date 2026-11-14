use viewport_protocol::*;
use viewport_protocol::{
    DeliveryMode, HistoryPermission, ModelDownloadPermission, SemanticPropertyScope,
};

const HIERARCHY_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MESH_ID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TEXTURE_ID: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn reference(blob_id: &str, payload_kind: RuntimePayloadKind) -> RuntimeBlobReference {
    RuntimeBlobReference {
        blob_id: blob_id.to_owned(),
        payload_kind,
        payload_version: 1,
        byte_size: 42,
    }
}

fn manifest() -> RuntimeManifest {
    RuntimeManifest {
        revision: "working-7".to_owned(),
        profile: RuntimeProfile::NativeMedium,
        hierarchy: reference(HIERARCHY_ID, RuntimePayloadKind::Hierarchy),
        meshes: vec![reference(MESH_ID, RuntimePayloadKind::Mesh)],
        materials: Vec::new(),
        textures: vec![reference(TEXTURE_ID, RuntimePayloadKind::Texture)],
    }
}

fn policy(blob_ids: &[&str]) -> AuthorizationPolicy {
    AuthorizationPolicy {
        allowed_delivery_modes: vec![DeliveryMode::SelfRender],
        model_download: ModelDownloadPermission::Allowed,
        allowed_blob_ids: blob_ids.iter().map(|id| (*id).to_owned()).collect(),
        semantic_property_scope: SemanticPropertyScope::None,
        history: HistoryPermission::Denied,
        runtime_profile: RuntimeProfile::NativeMedium,
    }
}

#[test]
fn authorization_omits_disallowed_optional_payloads() {
    let authorized = manifest()
        .authorize(&policy(&[HIERARCHY_ID, MESH_ID]))
        .unwrap();

    assert!(authorized.allows_blob(HIERARCHY_ID));
    assert!(authorized.allows_blob(MESH_ID));
    assert!(!authorized.allows_blob(TEXTURE_ID));
    assert_eq!(authorized.redacted_blob_count, 1);
}

#[test]
fn authorization_rejects_a_missing_required_hierarchy_blob() {
    assert_eq!(
        manifest().authorize(&policy(&[MESH_ID])),
        Err(RuntimeManifestAuthorizationError::RequiredBlobNotAllowed(
            HIERARCHY_ID.to_owned()
        ))
    );
}

#[test]
fn manifest_rejects_duplicate_blob_references() {
    let mut manifest = manifest();
    manifest.textures[0].blob_id = MESH_ID.to_owned();
    assert!(matches!(
        manifest.validate(),
        Err(RuntimeManifestValidationError::DuplicateBlobId(id)) if id == MESH_ID
    ));
}
