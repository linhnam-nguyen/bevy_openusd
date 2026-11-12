use usd_model::BlobId;
use viewport_protocol::{AuthorizationPolicy, SemanticPropertyScope};

use super::super::client::{
    initialize_client_projection_store, read_client_projection, replace_client_projection,
};
use super::super::projection::authorize_snapshot;
use super::{MESH_ID, digest, policy, snapshot};

#[test]
fn projection_filters_properties_and_unauthorized_mesh_ids() {
    let projected = authorize_snapshot(&snapshot(), &policy(SemanticPropertyScope::None, false))
        .expect("self-render policy should project");
    let entity = projected.entities.values().next().unwrap();

    assert!(entity.properties.is_empty());
    assert_eq!(
        entity.geometry.as_ref().unwrap().render_blob,
        None,
        "unauthorized blob IDs must not enter client metadata"
    );
    assert_eq!(entity.source_full_hash, digest(6));
    assert_ne!(projected.projection_hash, digest(1));
}

#[test]
fn projection_keeps_explicitly_allowed_values_and_blob_ids() {
    let projected = authorize_snapshot(&snapshot(), &policy(SemanticPropertyScope::All, true))
        .expect("self-render policy should project");
    let entity = projected.entities.values().next().unwrap();

    assert_eq!(entity.properties.len(), 1);
    assert_eq!(
        entity.geometry.as_ref().unwrap().render_blob,
        Some(BlobId(MESH_ID.to_owned()))
    );
}

#[test]
fn projection_requires_self_render_and_download_authorization() {
    let visitor = AuthorizationPolicy::default();
    assert!(authorize_snapshot(&snapshot(), &visitor).is_err());
}

#[test]
fn projection_hash_is_stable_for_the_same_authorized_view() {
    let policy = policy(SemanticPropertyScope::None, false);
    let first = authorize_snapshot(&snapshot(), &policy).unwrap();
    let second = authorize_snapshot(&snapshot(), &policy).unwrap();
    assert_eq!(first.projection_hash, second.projection_hash);
}

#[test]
fn client_projection_store_round_trips_one_verified_projection() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime.block_on(async {
        let database = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("local Turso database should build");
        let mut connection = database
            .connect()
            .expect("local Turso connection should open");
        initialize_client_projection_store(&connection)
            .await
            .expect("client projection schema should apply");

        let projection =
            authorize_snapshot(&snapshot(), &policy(SemanticPropertyScope::None, false))
                .expect("projection should build");
        replace_client_projection(&mut connection, &projection)
            .await
            .expect("projection should store");
        let loaded = read_client_projection(&connection)
            .await
            .expect("projection should read")
            .expect("projection should exist");
        assert_eq!(loaded, projection);
    });
}
