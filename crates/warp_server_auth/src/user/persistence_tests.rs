use super::PersistedUser;

/// Verifies that the `WARP_USER_SECRET` JSON format as of March 6, 2026 can be deserialized.
#[test]
#[allow(deprecated)]
fn test_deserialize_2026_03_06_persisted_user() {
    const BLOB: &str = r#"{"id_token":{"id_token":"test-id-token","refresh_token":"test-refresh-token","expiration_time":"2099-01-01T00:00:00Z"},"refresh_token":"","local_id":"test-uid","email":"test@example.com","display_name":"Test User","photo_url":"https://example.com/photo.jpg","is_onboarded":true,"needs_sso_link":false,"anonymous_user_type":null,"linked_at":null,"personal_object_limits":null,"is_on_work_domain":false}"#;

    let user: PersistedUser =
        serde_json::from_str(BLOB).expect("2026-03-06 JSON should deserialize");

    assert_eq!(user.auth_tokens.id_token, "test-id-token");
    assert_eq!(user.auth_tokens.refresh_token, "test-refresh-token");
    assert_eq!(user.refresh_token, "");
    assert_eq!(user.local_id.as_str(), "test-uid");
    assert_eq!(user.metadata.email, "test@example.com");
    assert_eq!(user.metadata.display_name.as_deref(), Some("Test User"));
    assert_eq!(
        user.metadata.photo_url.as_deref(),
        Some("https://example.com/photo.jpg")
    );
    assert!(user.is_onboarded);
    assert!(!user.needs_sso_link);
    assert_eq!(user.anonymous_user_type, None);
    assert_eq!(user.linked_at, None);
    assert!(user.personal_object_limits.is_none());
    assert!(!user.is_on_work_domain);
}
