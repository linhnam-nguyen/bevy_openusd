use viewport_protocol::*;

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
        allowed_blob_ids: vec![
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        ],
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
        allowed_blob_ids: vec![],
        ..AuthorizationPolicy::default()
    };

    assert_eq!(
        policy.validate(),
        Err(AuthorizationValidationError::SelfRenderRequiresModelDownload)
    );
}
