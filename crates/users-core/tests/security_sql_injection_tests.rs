//! SQL Injection Security Tests

use users_core::{SqliteUserStore, UserStore, UserFilter};
use tempfile::TempDir;

async fn setup_test_store() -> (SqliteUserStore, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let store = SqliteUserStore::new(&db_url).await.unwrap();
    (store, temp_dir)
}

#[tokio::test]
async fn test_sql_injection_in_search() {
    let (store, _temp_dir) = setup_test_store().await;
    
    // Create a legitimate user first
    let user = store.create_user(users_core::CreateUserRequest {
        username: "testuser".to_string(),
        password: "hashed_password".to_string(),
        email: Some("test@example.com".to_string()),
        display_name: Some("Test User".to_string()),
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // SQL injection attempts that should NOT return all users or cause errors
    let malicious_inputs = vec![
        // Classic SQL injection
        "'; DROP TABLE users; --",
        "' OR '1'='1",
        "' OR 1=1 --",
        "admin'--",
        "admin' OR '1'='1",
        
        // UNION-based attacks
        "' UNION SELECT * FROM users--",
        "' UNION SELECT id, username, password_hash FROM users--",
        
        // Blind SQL injection
        "' AND 1=1--",
        "' AND 1=2--",
        
        // Time-based blind SQL injection
        "' OR SLEEP(5)--",
        "'; WAITFOR DELAY '0:0:5'--",
        
        // Stacked queries
        "'; INSERT INTO users (username, password_hash) VALUES ('hacker', 'pwd'); --",
        
        // Comment variations
        "admin'/*",
        "admin'#",
        
        // Encoding attempts
        "%27%20OR%20%271%27%3D%271",
        
        // Special characters
        "'; SELECT * FROM users WHERE '1' LIKE '%",
        "_%' OR '_%",
    ];
    
    for malicious_input in malicious_inputs {
        println!("Testing SQL injection: {}", malicious_input);
        
        let filter = UserFilter {
            search: Some(malicious_input.to_string()),
            ..Default::default()
        };
        
        // Should not panic or error
        let result = store.list_users(filter).await;
        assert!(result.is_ok(), "Query failed for input: {}", malicious_input);
        
        let users = result.unwrap();
        
        // Should not return users unless they legitimately match the search
        if !users.is_empty() {
            // Verify that returned users actually contain the search term
            for user in &users {
                let contains_search = 
                    user.username.contains(malicious_input) ||
                    user.email.as_ref().map(|e| e.contains(malicious_input)).unwrap_or(false) ||
                    user.display_name.as_ref().map(|d| d.contains(malicious_input)).unwrap_or(false);
                    
                assert!(contains_search, 
                    "User {} returned without matching search term '{}'", 
                    user.username, malicious_input
                );
            }
        }
        
        // Verify the database is still intact
        let all_users = store.list_users(UserFilter::default()).await.unwrap();
        assert_eq!(all_users.len(), 1, "Database corrupted by: {}", malicious_input);
        assert_eq!(all_users[0].id, user.id, "User data corrupted by: {}", malicious_input);
    }
}

#[tokio::test]
async fn test_sql_injection_in_role_filter() {
    let (store, _temp_dir) = setup_test_store().await;
    
    // Create test user
    store.create_user(users_core::CreateUserRequest {
        username: "testuser".to_string(),
        password: "hashed_password".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    let malicious_roles = vec![
        "admin' OR '1'='1",
        "'; DROP TABLE users; --",
        "user%' OR roles LIKE '%",
    ];
    
    for malicious_role in malicious_roles {
        let filter = UserFilter {
            role: Some(malicious_role.to_string()),
            ..Default::default()
        };
        
        let result = store.list_users(filter).await;
        assert!(result.is_ok());
        
        // Should not return users with different roles
        let users = result.unwrap();
        for user in users {
            assert!(
                user.roles.iter().any(|r| r.contains(malicious_role)),
                "Returned user without matching role"
            );
        }
    }
}

#[tokio::test]
async fn test_special_characters_in_search() {
    let (store, _temp_dir) = setup_test_store().await;
    
    // Create users with special characters
    let special_usernames = vec![
        "user_with_percent%",
        "user_with_underscore_",
        "user'with'quotes",
        "user\"with\"doublequotes",
        "user\\with\\backslash",
    ];
    
    for (i, username) in special_usernames.iter().enumerate() {
        // Skip if username would violate constraints
        match store.create_user(users_core::CreateUserRequest {
            username: username.to_string(),
            password: "hashed_password".to_string(),
            email: Some(format!("user{}@example.com", i)),
            display_name: None,
            roles: vec!["user".to_string()],
        }).await {
            Ok(_) => {},
            Err(_) => continue, // Some special chars might be invalid usernames
        }
    }
    
    // Search for special characters - should be escaped properly
    for search_term in vec!["%", "_", "'", "\"", "\\"] {
        let filter = UserFilter {
            search: Some(search_term.to_string()),
            ..Default::default()
        };
        
        let result = store.list_users(filter).await;
        assert!(result.is_ok());
        
        // Verify only users with that character are returned
        let users = result.unwrap();
        for user in users {
            assert!(
                user.username.contains(search_term) ||
                user.email.as_ref().map(|e| e.contains(search_term)).unwrap_or(false),
                "User {} returned without containing '{}'",
                user.username,
                search_term
            );
        }
    }
}

#[tokio::test]
async fn test_limit_offset_injection() {
    let (store, _temp_dir) = setup_test_store().await;
    
    // Create multiple users
    for i in 0..5 {
        store.create_user(users_core::CreateUserRequest {
            username: format!("user{}", i),
            password: "hashed_password".to_string(),
            email: None,
            display_name: None,
            roles: vec!["user".to_string()],
        }).await.unwrap();
    }
    
    // Test that very large values are rejected
    let filter = UserFilter {
        limit: Some(u32::MAX), // Should be rejected
        offset: Some(u32::MAX), // Should be rejected
        ..Default::default()
    };
    
    let result = store.list_users(filter).await;
    assert!(result.is_err(), "Should reject very large limit/offset values");
    
    // Test with reasonable but high values
    let filter = UserFilter {
        limit: Some(10),
        offset: Some(100), // Higher than number of users
        ..Default::default()
    };
    
    let result = store.list_users(filter).await;
    assert!(result.is_ok());
    
    // With offset beyond user count, should return empty
    assert_eq!(result.unwrap().len(), 0);
}
