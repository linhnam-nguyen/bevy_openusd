use super::*;
use crate::{
    AuthorizedRuntimeManifest, RuntimeBlobReference, RuntimePayloadKind, RuntimeProfile,
    SessionEvent,
};

const HIERARCHY_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MESH_ID: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn reference(blob_id: &str, kind: RuntimePayloadKind, byte_size: u64) -> RuntimeBlobReference {
    RuntimeBlobReference {
        blob_id: blob_id.to_owned(),
        payload_kind: kind,
        payload_version: 1,
        byte_size,
    }
}

fn manifest(meshes: Vec<RuntimeBlobReference>) -> AuthorizedRuntimeManifest {
    AuthorizedRuntimeManifest {
        revision: "working-7".to_owned(),
        profile: RuntimeProfile::NativeMedium,
        hierarchy: reference(HIERARCHY_ID, RuntimePayloadKind::Hierarchy, 3),
        meshes,
        materials: Vec::new(),
        textures: Vec::new(),
        redacted_blob_count: 1,
    }
}

#[test]
fn assembles_out_of_order_manifest_and_blob_chunks() {
    let mut assembler = RuntimeDeliveryAssembler::default();
    let full = manifest(vec![
        reference(MESH_ID, RuntimePayloadKind::Mesh, 4),
        reference(
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            RuntimePayloadKind::Mesh,
            2,
        ),
    ]);
    let first = AuthorizedRuntimeManifest {
        meshes: vec![full.meshes[0].clone()],
        ..full.clone()
    };
    let second = AuthorizedRuntimeManifest {
        meshes: vec![full.meshes[1].clone()],
        ..full.clone()
    };

    assert!(
        !assembler
            .accept_manifest_chunk("manifest-7", 1, 2, second)
            .unwrap()
    );
    assert!(
        assembler
            .accept_manifest_chunk("manifest-7", 0, 2, first)
            .unwrap()
    );
    assert!(!assembler.is_ready());

    assert!(
        !assembler
            .accept_blob_chunk(MESH_ID, 1, 2, vec![3, 4])
            .unwrap()
    );
    assert!(
        assembler
            .accept_blob_chunk(MESH_ID, 0, 2, vec![1, 2])
            .unwrap()
    );
    assert!(
        assembler
            .accept_blob_chunk(HIERARCHY_ID, 0, 1, vec![9, 8, 7])
            .unwrap()
    );
    assert!(
        assembler
            .accept_blob_chunk(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                0,
                1,
                vec![5, 6],
            )
            .unwrap()
    );

    let hydrated = assembler.hydrated().unwrap();
    assert_eq!(hydrated.blob(HIERARCHY_ID), Some([9, 8, 7].as_slice()));
    assert_eq!(hydrated.blob(MESH_ID), Some([1, 2, 3, 4].as_slice()));
}

#[test]
fn rejects_unauthorized_or_wrong_sized_blob() {
    let mut assembler = RuntimeDeliveryAssembler::default();
    assembler
        .accept_manifest(manifest(Vec::new()))
        .expect("fixture manifest is valid");

    assert!(matches!(
        assembler.accept_blob_chunk(
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            0,
            1,
            vec![1],
        ),
        Err(RuntimeDeliveryClientError::BlobNotAuthorized(_))
    ));
    assert!(matches!(
        assembler.accept_blob_chunk(HIERARCHY_ID, 0, 1, vec![1, 2]),
        Err(RuntimeDeliveryClientError::BlobSizeMismatch { .. })
    ));
    assert!(!assembler.is_ready());
}

#[test]
fn applies_session_events_and_replaces_old_revision() {
    let mut assembler = RuntimeDeliveryAssembler::default();
    let event = SessionEvent::RuntimeManifest {
        manifest: manifest(Vec::new()),
    };
    assert_eq!(
        assembler.apply_session_event(&event).unwrap(),
        RuntimeDeliveryUpdate::ManifestAccepted
    );
    assert!(matches!(
        assembler
            .apply_session_event(&SessionEvent::RuntimeBlobRejected {
                reason: "denied".to_owned(),
            })
            .unwrap(),
        RuntimeDeliveryUpdate::BlobRejected { .. }
    ));

    assembler
        .accept_blob_chunk(HIERARCHY_ID, 0, 1, vec![1, 2, 3])
        .unwrap();
    assert!(assembler.is_ready());

    let mut replacement = manifest(Vec::new());
    replacement.revision = "working-8".to_owned();
    assembler.accept_manifest(replacement).unwrap();
    assert!(!assembler.is_ready());
}
