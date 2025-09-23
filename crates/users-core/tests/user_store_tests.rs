//! Tests for the user store functionality
//! These tests serve as both verification and developer examples

use users_core::{SqliteUserStore, UserStore, CreateUserRequest, UpdateUserRequest, UserFilter};
use chrono::Utc;
use tempfile::TempDir;

/// Helper to create a test database
async fn create_test_db() -> (SqliteUserStore, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let store = SqliteUserStore::new(&db_url).await
        .expect("Failed to create test database");
    
    (store, temp_dir)
}

#[tokio::test]
async fn test_create_user() {
    let (store, _temp_dir) = create_test_db().await;
    
    let request = CreateUserRequest {
        username: "alice".to_string(),
        password: "hashed_password".to_string(),  // In real usage, this would be hashed by AuthenticationService
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Smith".to_string()),
        roles: vec!["user".to_string()],
    };
    
    let user = store.create_user(request).await.unwrap();
    
    assert_eq!(user.username, "alice");
    assert_eq!(user.email.as_deref(), Some("alice@example.com"));
    assert_eq!(user.display_name.as_deref(), Some("Alice Smith"));
    assert_eq!(user.roles, vec!["user"]);
    assert!(user.active);
    assert!(user.last_login.is_none());
}

#[tokio::test]
async fn test_duplicate_username_error() {
    let (store, _temp_dir) = create_test_db().await;
    
    let request = CreateUserRequest {
        username: "bob".to_string(),
        password: "password123".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    };
    
    // First creation should succeed
    store.create_user(request.clone()).await.unwrap();
    
    // Second creation with same username should fail
    let result = store.create_user(request).await;
    assert!(result.is_err());
    
    match result.unwrap_err() {
        users_core::Error::UserAlreadyExists(username) => {
            assert_eq!(username, "bob");
        }
        _ => panic!("Expected UserAlreadyExists error"),
    }
}

#[tokio::test]
async fn test_get_user_by_id() {
    let (store, _temp_dir) = create_test_db().await;
    
    let user = store.create_user(CreateUserRequest {
        username: "charlie".to_string(),
        password: "password".to_string(),
        email: Some("charlie@example.com".to_string()),
        display_name: None,
        roles: vec!["admin".to_string()],
    }).await.unwrap();
    
    // Retrieve by ID
    let retrieved = store.get_user(&user.id).await.unwrap();
    assert!(retrieved.is_some());
    
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, user.id);
    assert_eq!(retrieved.username, "charlie");
    assert_eq!(retrieved.roles, vec!["admin"]);
}

#[tokio::test]
async fn test_get_user_by_username() {
    let (store, _temp_dir) = create_test_db().await;
    
    store.create_user(CreateUserRequest {
        username: "dave".to_string(),
        password: "password".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Retrieve by username
    let user = store.get_user_by_username("dave").await.unwrap();
    assert!(user.is_some());
    assert_eq!(user.unwrap().username, "dave");
    
    // Non-existent user
    let user = store.get_user_by_username("nonexistent").await.unwrap();
    assert!(user.is_none());
}

#[tokio::test]
async fn test_update_user() {
    let (store, _temp_dir) = create_test_db().await;
    
    let user = store.create_user(CreateUserRequest {
        username: "eve".to_string(),
        password: "password".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Update the user
    let updated = store.update_user(&user.id, UpdateUserRequest {
        email: Some("eve@newdomain.com".to_string()),
        display_name: Some("Eve Johnson".to_string()),
        roles: Some(vec!["user".to_string(), "moderator".to_string()]),
        active: Some(false),
    }).await.unwrap();
    
    assert_eq!(updated.email.as_deref(), Some("eve@newdomain.com"));
    assert_eq!(updated.display_name.as_deref(), Some("Eve Johnson"));
    assert_eq!(updated.roles, vec!["user", "moderator"]);
    assert!(!updated.active);
    assert!(updated.updated_at > user.updated_at);
}

#[tokio::test]
async fn test_delete_user() {
    let (store, _temp_dir) = create_test_db().await;
    
    let user = store.create_user(CreateUserRequest {
        username: "frank".to_string(),
        password: "password".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Delete the user
    store.delete_user(&user.id).await.unwrap();
    
    // Verify user is gone
    let retrieved = store.get_user(&user.id).await.unwrap();
    assert!(retrieved.is_none());
    
    // Deleting non-existent user should error
    let result = store.delete_user("nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_list_users_with_filters() {
    let (store, _temp_dir) = create_test_db().await;
    
    // Create multiple users
    for i in 1..=5 {
        store.create_user(CreateUserRequest {
            username: format!("user{}", i),
            password: "password".to_string(),
            email: Some(format!("user{}@example.com", i)),
            display_name: Some(format!("User {}", i)),
            roles: if i % 2 == 0 { vec!["admin".to_string()] } else { vec!["user".to_string()] },
        }).await.unwrap();
    }
    
    // Deactivate one user
    let user3 = store.get_user_by_username("user3").await.unwrap().unwrap();
    store.update_user(&user3.id, UpdateUserRequest {
        email: None,
        display_name: None,
        roles: None,
        active: Some(false),
    }).await.unwrap();
    
    // Test: Get all users
    let all_users = store.list_users(UserFilter::default()).await.unwrap();
    assert_eq!(all_users.len(), 5);
    
    // Test: Filter by active status
    let active_users = store.list_users(UserFilter {
        active: Some(true),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(active_users.len(), 4);
    
    // Test: Filter by role
    let admins = store.list_users(UserFilter {
        role: Some("admin".to_string()),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(admins.len(), 2);
    
    // Test: Search by text
    let search_results = store.list_users(UserFilter {
        search: Some("user2".to_string()),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].username, "user2");
    
    // Test: Pagination
    let page1 = store.list_users(UserFilter {
        limit: Some(2),
        offset: Some(0),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(page1.len(), 2);
    
    let page2 = store.list_users(UserFilter {
        limit: Some(2),
        offset: Some(2),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(page2.len(), 2);
    
    // Verify no overlap between pages
    let page1_ids: Vec<_> = page1.iter().map(|u| &u.id).collect();
    let page2_ids: Vec<_> = page2.iter().map(|u| &u.id).collect();
    assert!(page1_ids.iter().all(|id| !page2_ids.contains(id)));
}

#[tokio::test]
async fn test_complex_user_workflow() {
    let (store, _temp_dir) = create_test_db().await;
    
    // 1. Create a user
    let user = store.create_user(CreateUserRequest {
        username: "workflow_user".to_string(),
        password: "initial_password".to_string(),
        email: Some("workflow@example.com".to_string()),
        display_name: Some("Workflow Test User".to_string()),
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // 2. Verify creation
    let retrieved = store.get_user_by_username("workflow_user").await.unwrap().unwrap();
    assert_eq!(retrieved.id, user.id);
    
    // 3. Update email and add admin role
    let updated = store.update_user(&user.id, UpdateUserRequest {
        email: Some("workflow.updated@example.com".to_string()),
        display_name: None,
        roles: Some(vec!["user".to_string(), "admin".to_string()]),
        active: None,
    }).await.unwrap();
    
    assert_eq!(updated.email.as_deref(), Some("workflow.updated@example.com"));
    assert_eq!(updated.roles.len(), 2);
    
    // 4. Deactivate the user
    store.update_user(&user.id, UpdateUserRequest {
        email: None,
        display_name: None,
        roles: None,
        active: Some(false),
    }).await.unwrap();
    
    // 5. Verify deactivation
    let deactivated = store.get_user(&user.id).await.unwrap().unwrap();
    assert!(!deactivated.active);
    
    // 6. Search should still find inactive users by default
    let search_results = store.list_users(UserFilter {
        search: Some("workflow".to_string()),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(search_results.len(), 1);
    
    // 7. But not when filtering for active only
    let active_search = store.list_users(UserFilter {
        search: Some("workflow".to_string()),
        active: Some(true),
        ..Default::default()
    }).await.unwrap();
    assert_eq!(active_search.len(), 0);
}
