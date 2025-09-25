//! Tests for API key management
//! These tests demonstrate how to create, validate, and manage API keys

use users_core::{SqliteUserStore, UserStore, ApiKeyStore, CreateUserRequest};
use users_core::api_keys::CreateApiKeyRequest;
use chrono::{Duration, Utc};
use tempfile::TempDir;

/// Helper to create a test database with a user
async fn create_test_db_with_user() -> (SqliteUserStore, String, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let store = SqliteUserStore::new(&db_url).await
        .expect("Failed to create test database");
    
    // Create a test user
    let user = store.create_user(CreateUserRequest {
        username: "api_test_user".to_string(),
        password: "password".to_string(),
        email: Some("api@example.com".to_string()),
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    (store, user.id, temp_dir)
}

#[tokio::test]
async fn test_create_api_key() {
    let (store, user_id, _temp_dir) = create_test_db_with_user().await;
    
    let (api_key, raw_key) = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Test API Key".to_string(),
        permissions: vec!["read".to_string(), "write".to_string()],
        expires_at: None,
    }).await.unwrap();
    
    // Verify API key properties
    assert_eq!(api_key.user_id, user_id);
    assert_eq!(api_key.name, "Test API Key");
    assert_eq!(api_key.permissions, vec!["read", "write"]);
    assert!(api_key.expires_at.is_none());
    assert!(api_key.last_used.is_none());
    
    // Verify raw key format
    assert!(raw_key.starts_with("rvoip_ak_live_"));
    assert_eq!(raw_key.len(), "rvoip_ak_live_".len() + 32);
}

#[tokio::test]
async fn test_validate_api_key() {
    let (store, user_id, _temp_dir) = create_test_db_with_user().await;
    
    // Create an API key
    let (created_key, raw_key) = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Validation Test".to_string(),
        permissions: vec!["read".to_string()],
        expires_at: None,
    }).await.unwrap();
    
    // Validate with correct key
    let validated = store.validate_api_key(&raw_key).await.unwrap();
    assert!(validated.is_some());
    
    let validated = validated.unwrap();
    assert_eq!(validated.id, created_key.id);
    assert_eq!(validated.name, "Validation Test");
    assert!(validated.last_used.is_some()); // Should be updated after validation
    
    // Validate with incorrect key
    let invalid = store.validate_api_key("rvoip_ak_live_invalid1234567890").await.unwrap();
    assert!(invalid.is_none());
}

#[tokio::test]
async fn test_expired_api_key() {
    let (store, user_id, _temp_dir) = create_test_db_with_user().await;
    
    // Create an API key that will expire very soon (1 second)
    let expires_at = Utc::now() + Duration::seconds(1);
    let (_api_key, raw_key) = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Expired Key".to_string(),
        permissions: vec!["read".to_string()],
        expires_at: Some(expires_at),
    }).await.unwrap();
    
    // Wait for the key to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Validation should fail
    let result = store.validate_api_key(&raw_key).await;
    assert!(result.is_err());
    
    match result.unwrap_err() {
        users_core::Error::ApiKeyExpired => {},
        _ => panic!("Expected ApiKeyExpired error"),
    }
}

#[tokio::test]
async fn test_revoke_api_key() {
    let (store, user_id, _temp_dir) = create_test_db_with_user().await;
    
    // Create an API key
    let (api_key, raw_key) = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "To Be Revoked".to_string(),
        permissions: vec!["admin".to_string()],
        expires_at: None,
    }).await.unwrap();
    
    // Verify it works initially
    let validated = store.validate_api_key(&raw_key).await.unwrap();
    assert!(validated.is_some());
    
    // Revoke the key
    store.revoke_api_key(&api_key.id).await.unwrap();
    
    // Validation should now fail
    let validated = store.validate_api_key(&raw_key).await.unwrap();
    assert!(validated.is_none());
    
    // Revoking non-existent key should error
    let result = store.revoke_api_key("nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_list_user_api_keys() {
    let (store, user_id, _temp_dir) = create_test_db_with_user().await;
    
    // Create multiple API keys
    for i in 1..=3 {
        store.create_api_key(CreateApiKeyRequest {
            user_id: user_id.clone(),
            name: format!("Key {}", i),
            permissions: vec![if i == 1 { "read".to_string() } else if i == 2 { "write".to_string() } else { "admin".to_string() }],
            expires_at: if i == 2 { 
                Some(Utc::now() + Duration::days(30)) 
            } else { 
                None 
            },
        }).await.unwrap();
    }
    
    // List all keys for the user
    let keys = store.list_api_keys(&user_id).await.unwrap();
    assert_eq!(keys.len(), 3);
    
    // Verify they're ordered by creation date (newest first)
    assert_eq!(keys[0].name, "Key 3");
    assert_eq!(keys[1].name, "Key 2");
    assert_eq!(keys[2].name, "Key 1");
    
    // Verify expiration is set correctly
    assert!(keys[0].expires_at.is_none());
    assert!(keys[1].expires_at.is_some());
    assert!(keys[2].expires_at.is_none());
}

#[tokio::test]
async fn test_api_key_unique_names_per_user() {
    let (store, user_id, _temp_dir) = create_test_db_with_user().await;
    
    // Create first key
    store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Production Key".to_string(),
        permissions: vec!["*".to_string()],
        expires_at: None,
    }).await.unwrap();
    
    // Try to create another key with same name for same user
    let result = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Production Key".to_string(),
        permissions: vec!["read".to_string()],
        expires_at: None,
    }).await;
    
    // Should fail due to unique constraint
    assert!(result.is_err());
    
    // But different user should be able to use same name
    let user2 = store.create_user(CreateUserRequest {
        username: "another_user".to_string(),
        password: "password".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // This should succeed
    store.create_api_key(CreateApiKeyRequest {
        user_id: user2.id,
        name: "Production Key".to_string(),
        permissions: vec!["read".to_string()],
        expires_at: None,
    }).await.unwrap();
}

#[tokio::test]
async fn test_api_key_permissions() {
    let (store, user_id, _temp_dir) = create_test_db_with_user().await;
    
    // Create keys with different permission sets
    let (read_key, read_raw) = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Read Only".to_string(),
        permissions: vec!["read".to_string()],
        expires_at: None,
    }).await.unwrap();
    
    let (admin_key, admin_raw) = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Admin Key".to_string(),
        permissions: vec!["*".to_string()],  // All permissions
        expires_at: None,
    }).await.unwrap();
    
    // Validate and check permissions
    let read_validated = store.validate_api_key(&read_raw).await.unwrap().unwrap();
    assert_eq!(read_validated.permissions.len(), 1);
    assert!(read_validated.permissions.contains(&"read".to_string()));
    
    let admin_validated = store.validate_api_key(&admin_raw).await.unwrap().unwrap();
    assert_eq!(admin_validated.permissions, vec!["*"]);
}

#[tokio::test]
async fn test_api_key_cleanup_on_user_delete() {
    let (store, user_id, _temp_dir) = create_test_db_with_user().await;
    
    // Create API keys
    let (key1, _) = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Key 1".to_string(),
        permissions: vec!["read".to_string()],
        expires_at: None,
    }).await.unwrap();
    
    let (key2, _) = store.create_api_key(CreateApiKeyRequest {
        user_id: user_id.clone(),
        name: "Key 2".to_string(),
        permissions: vec!["write".to_string()],
        expires_at: None,
    }).await.unwrap();
    
    // Verify keys exist
    let keys = store.list_api_keys(&user_id).await.unwrap();
    assert_eq!(keys.len(), 2);
    
    // Delete the user
    store.delete_user(&user_id).await.unwrap();
    
    // Keys should be gone (CASCADE DELETE)
    let keys = store.list_api_keys(&user_id).await.unwrap();
    assert_eq!(keys.len(), 0);
}
