use std::path::PathBuf;

use super::super::client_config::TursoClientSyncConfig;
use super::super::platform_api::{
    CreateDatabaseResponse, CreateTokenResponse, TursoPlatformApiConfig,
};
use super::super::provisioning::{TursoClientSyncProvisioner, TursoCloudProvisioner};
use super::{RecordingCloudAdmin, cloud_config, cloud_request};

#[test]
fn client_sync_config_rejects_missing_transport_credentials() {
    let config = TursoClientSyncConfig {
        local_path: PathBuf::from("client.db"),
        remote_url: "libsql://client.turso.io".to_owned(),
        client_name: "usd-hub-client".to_owned(),
    };
    assert!(config.validate("").is_err());
    assert!(config.validate("token").is_ok());
}

#[test]
fn platform_api_config_keeps_secret_out_of_debug_and_normalizes_base_url() {
    let config = TursoPlatformApiConfig::new(
        "https://api.turso.tech/v1/".to_owned(),
        "usdhub".to_owned(),
        "platform-token".to_owned(),
    )
    .expect("valid Platform API configuration should be accepted");

    assert_eq!(config.api_base_url, "https://api.turso.tech/v1");
    assert_eq!(config.organization_slug, "usdhub");
    assert!(
        TursoPlatformApiConfig::new(
            "ftp://api.turso.tech/v1".to_owned(),
            "usdhub".to_owned(),
            "platform-token".to_owned(),
        )
        .is_err()
    );
}

#[test]
fn platform_api_response_models_decode_documented_shapes() {
    let database: CreateDatabaseResponse = serde_json::from_str(
        r#"{"database":{"DbId":"db-id","Hostname":"db-name.turso.io","Name":"db-name"}}"#,
    )
    .expect("documented database response should decode");
    assert_eq!(database.database.hostname, "db-name.turso.io");

    let token: CreateTokenResponse = serde_json::from_str(r#"{"jwt":"database-token"}"#)
        .expect("documented token response should decode");
    assert_eq!(token.jwt, "database-token");
}

#[test]
fn cloud_provider_provisions_scoped_database_and_revokes_once() {
    let admin = RecordingCloudAdmin::default();
    let created = admin.created.clone();
    let token_requests = admin.token_requests.clone();
    let deleted = admin.deleted.clone();
    let provider = TursoCloudProvisioner::new(admin, cloud_config())
        .expect("cloud provider configuration should validate");
    let request = cloud_request();

    let credentials = provider
        .provision(&request)
        .expect("cloud provider should provision a client database");
    assert_eq!(credentials.auth_token, "database-scoped-token");
    assert!(
        credentials
            .config
            .remote_url
            .starts_with("libsql://usdhub-client-")
    );
    assert!(credentials.config.remote_url.ends_with(".turso.io"));
    assert!(credentials.config.local_path.starts_with("client-sync"));
    assert_eq!(created.lock().unwrap().len(), 1);
    assert_eq!(token_requests.lock().unwrap().len(), 1);

    provider
        .revoke(&request.session_id)
        .expect("cloud provider should revoke the client database");
    provider
        .revoke(&request.session_id)
        .expect("revoke should be idempotent after the lease is gone");
    assert_eq!(deleted.lock().unwrap().len(), 1);
}

#[test]
fn cloud_provider_deletes_database_when_token_issuance_fails() {
    let admin = RecordingCloudAdmin::default();
    *admin.fail_token.lock().unwrap() = true;
    let deleted = admin.deleted.clone();
    let provider = TursoCloudProvisioner::new(admin, cloud_config())
        .expect("cloud provider configuration should validate");

    let error = match provider.provision(&cloud_request()) {
        Ok(_) => panic!("token failure should fail provisioning"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("test token issuance failure"));
    assert_eq!(deleted.lock().unwrap().len(), 1);
}
