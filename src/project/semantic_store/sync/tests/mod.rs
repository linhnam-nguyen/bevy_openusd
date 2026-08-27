mod authorization;
mod close;
mod control_lane;
mod lifecycle;
mod projection;
mod provisioning;

use anyhow::{Result, bail};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use usd_model::{
    BlobId, Bounds3, CanonicalValue, EntityKey, EntitySnapshot, GeometrySignature, HashDigest,
    IdentitySource, QuantizedPoint3, SemanticInfo, SemanticProperty, SemanticSnapshot, SnapshotId,
    SnapshotSource, TransformSignature,
};
use viewport_protocol::{
    AuthorizationPolicy, DeliveryMode, HistoryPermission, ModelDownloadPermission, RuntimeProfile,
    SemanticPropertyScope, SessionId,
};

use super::client_config::TursoCloudProvisioningConfig;
use super::platform_api::{TursoCloudAdmin, TursoCloudDatabase};
use super::provisioning::{
    TursoClientSyncCredentials, TursoClientSyncProvisionRequest, TursoClientSyncProvisioner,
};

pub(super) const MESH_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub(super) fn digest(value: u8) -> HashDigest {
    HashDigest::new([value; HashDigest::BYTE_LEN])
}

pub(super) fn snapshot() -> SemanticSnapshot {
    let key = EntityKey::from("entity-1");
    SemanticSnapshot {
        snapshot_id: SnapshotId("working-7".to_owned()),
        source: SnapshotSource::Working {
            session: "editor".to_owned(),
            live_revision: 7,
        },
        config_hash: digest(1),
        entities: HashMap::from([(
            key.clone(),
            EntitySnapshot {
                key,
                prim_path: "/Root/Asset".to_owned(),
                identity_source: IdentitySource::PrimPath,
                semantic: SemanticInfo {
                    category: Some("asset".to_owned()),
                    family: None,
                    type_name: Some("Mesh".to_owned()),
                    type_id: None,
                    display_name: Some("Asset".to_owned()),
                },
                transform: TransformSignature {
                    translation_mm: [0, 0, 0],
                    rotation_quantized: [0, 0, 0, 1],
                    scale_quantized: [1, 1, 1],
                    hash: digest(2),
                },
                geometry: Some(GeometrySignature {
                    vertex_count: 3,
                    index_count: 3,
                    local_bounds: Bounds3 {
                        min: [0.0, 0.0, 0.0],
                        max: [1.0, 1.0, 1.0],
                    },
                    local_centroid: QuantizedPoint3([500, 500, 500]),
                    topology_hash: digest(3),
                    shape_hash: digest(4),
                    render_blob: Some(BlobId(MESH_ID.to_owned())),
                }),
                properties: vec![SemanticProperty {
                    name: "secret_cost".to_owned(),
                    value: CanonicalValue::Integer(42),
                    measurement: None,
                }],
                metadata_hash: digest(5),
                full_hash: digest(6),
            },
        )]),
    }
}

pub(super) fn policy(scope: SemanticPropertyScope, allow_mesh: bool) -> AuthorizationPolicy {
    AuthorizationPolicy {
        allowed_delivery_modes: vec![DeliveryMode::SelfRender],
        model_download: ModelDownloadPermission::Allowed,
        allowed_blob_ids: allow_mesh
            .then_some(MESH_ID.to_owned())
            .or_else(|| {
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned())
            })
            .into_iter()
            .collect(),
        semantic_property_scope: scope,
        history: HistoryPermission::ReadOnly,
        runtime_profile: RuntimeProfile::NativeMedium,
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingProvisioner {
    pub(super) provisioned: Arc<Mutex<Vec<SessionId>>>,
    pub(super) revoked: Arc<Mutex<Vec<SessionId>>>,
    pub(super) fail_revoke: Arc<Mutex<bool>>,
}

impl TursoClientSyncProvisioner for RecordingProvisioner {
    fn provision(
        &self,
        request: &TursoClientSyncProvisionRequest,
    ) -> Result<TursoClientSyncCredentials> {
        self.provisioned
            .lock()
            .expect("provision records should not be poisoned")
            .push(request.session_id.clone());
        TursoClientSyncCredentials::new(
            super::client_config::TursoClientSyncConfig {
                local_path: PathBuf::from(format!("client-{}.db", request.session_id.0)),
                remote_url: "libsql://client.turso.io".to_owned(),
                client_name: request.client_name.clone(),
            },
            "opaque-test-token".to_owned(),
        )
    }

    fn revoke(&self, session_id: &SessionId) -> Result<()> {
        if *self.fail_revoke.lock().expect("fail_revoke lock") {
            bail!("simulated revoke failure");
        }
        self.revoked
            .lock()
            .expect("revoke records should not be poisoned")
            .push(session_id.clone());
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingCloudAdmin {
    pub(super) created: Arc<Mutex<Vec<(String, String, String)>>>,
    pub(super) token_requests: Arc<Mutex<Vec<String>>>,
    pub(super) deleted: Arc<Mutex<Vec<String>>>,
    pub(super) fail_token: Arc<Mutex<bool>>,
}

impl TursoCloudAdmin for RecordingCloudAdmin {
    fn create_database(
        &self,
        organization_slug: &str,
        database_name: &str,
        group_name: &str,
    ) -> Result<TursoCloudDatabase> {
        self.created
            .lock()
            .expect("cloud admin records should not be poisoned")
            .push((
                organization_slug.to_owned(),
                database_name.to_owned(),
                group_name.to_owned(),
            ));
        Ok(TursoCloudDatabase {
            hostname: format!("{database_name}.turso.io"),
        })
    }

    fn create_database_token(
        &self,
        _organization_slug: &str,
        database_name: &str,
        _expiration: Option<&str>,
    ) -> Result<String> {
        self.token_requests
            .lock()
            .expect("cloud admin records should not be poisoned")
            .push(database_name.to_owned());
        if *self
            .fail_token
            .lock()
            .expect("cloud admin records should not be poisoned")
        {
            bail!("test token issuance failure");
        }
        Ok("database-scoped-token".to_owned())
    }

    fn delete_database(&self, _organization_slug: &str, database_name: &str) -> Result<()> {
        self.deleted
            .lock()
            .expect("cloud admin records should not be poisoned")
            .push(database_name.to_owned());
        Ok(())
    }
}

pub(super) fn cloud_config() -> TursoCloudProvisioningConfig {
    TursoCloudProvisioningConfig {
        organization_slug: "usdhub".to_owned(),
        group_name: "default".to_owned(),
        database_prefix: "usdhub-client".to_owned(),
        local_root: PathBuf::from("client-sync"),
        token_expiration: Some("2h".to_owned()),
    }
}

pub(super) fn cloud_request() -> TursoClientSyncProvisionRequest {
    TursoClientSyncProvisionRequest {
        session_id: SessionId::new("session-cloud"),
        client_name: "native-client".to_owned(),
        authorization: policy(SemanticPropertyScope::None, false),
    }
}
