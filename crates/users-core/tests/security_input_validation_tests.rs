//! Input Validation Security Tests

use users_core::{AuthenticationService, CreateUserRequest, UpdateUserRequest, UsersConfig};
use tempfile::TempDir;

async fn setup_test_auth_service() -> (AuthenticationService, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = UsersConfig {
        database_url: db_url,
        ..Default::default()
    };
    
    let service = users_core::init(config).await.unwrap();
    (service, temp_dir)
}

#[tokio::test]
async fn test_username_validation() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Valid usernames
    let max_username = "a".repeat(32);
    let valid_usernames = vec![
        "user123",
        "john_doe",
        "alice.smith",
        "user-name",
        "abc",  // 3 chars minimum
        max_username.as_str(),  // 32 chars maximum
    ];
    
    for username in valid_usernames {
        let result = auth_service.create_user(CreateUserRequest {
            username: username.to_string(),
            password: "ValidPassword123!".to_string(),
            email: None,
            display_name: None,
            roles: vec!["user".to_string()],
        }).await;
        
        assert!(result.is_ok(), "Valid username '{}' should be accepted", username);
    }
    
    // Invalid usernames
    let too_long_username = "a".repeat(33);
    let invalid_usernames = vec![
        "ab",  // Too short
        too_long_username.as_str(),  // Too long
        "user name",  // Contains space
        "user@name",  // Contains @
        "user#name",  // Contains #
        "<script>alert('xss')</script>",  // XSS attempt
        "'; DROP TABLE users; --",  // SQL injection
        "",  // Empty
        "用户名",  // Non-ASCII
        "user\nname",  // Contains newline
        "user\0name",  // Contains null byte
    ];
    
    for username in invalid_usernames {
        let result = auth_service.create_user(CreateUserRequest {
            username: username.to_string(),
            password: "ValidPassword123!".to_string(),
            email: None,
            display_name: None,
            roles: vec!["user".to_string()],
        }).await;
        
        assert!(result.is_err(), "Invalid username '{}' should be rejected", username);
    }
}

#[tokio::test]
async fn test_password_validation() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Valid passwords (12+ chars, upper, lower, number, optional special)
    let valid_passwords = vec![
        "ValidPassword123",      // No special char (optional)
        "ValidPassword123!",     // With special char
        "MySecurePass2024",      // Another valid example
        "ThisIsALongPassphrase123",  // Passphrase style
        "Test12Pass!X",         // Exactly 12 chars, mixed case and number
    ];
    
    let mut user_count = 0;
    for password in valid_passwords {
        let result = auth_service.create_user(CreateUserRequest {
            username: format!("validuser{}", user_count),
            password: password.to_string(),
            email: None,
            display_name: None,
            roles: vec!["user".to_string()],
        }).await;
        
        assert!(result.is_ok(), "Valid password should be accepted: {}", password);
        user_count += 1;
    }
    
    // Invalid passwords
    let too_long_password = "a".repeat(129);
    let invalid_passwords = vec![
        "Short1!",               // Too short (< 12)
        "alllowercase123",       // No uppercase
        "ALLUPPERCASE123",       // No lowercase
        "NoNumbersHere!",        // No numbers
        too_long_password.as_str(),  // Too long (> 128)
        "",                      // Empty
        "password123",           // Common password (if checking)
        "TestUser0",             // Contains username (when username is TestUser0)
    ];
    
    for (i, password) in invalid_passwords.iter().enumerate() {
        let username = if *password == "TestUser0" { "TestUser0" } else { &format!("invaliduser{}", i) };
        
        let result = auth_service.create_user(CreateUserRequest {
            username: username.to_string(),
            password: password.to_string(),
            email: None,
            display_name: None,
            roles: vec!["user".to_string()],
        }).await;
        
        assert!(result.is_err(), "Invalid password should be rejected: {}", password);
    }
}

#[tokio::test]
async fn test_email_validation() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Valid emails
    let valid_emails = vec![
        Some("user@example.com"),
        Some("john.doe+tag@company.co.uk"),
        Some("alice_smith@sub.domain.org"),
        None,  // Email is optional
    ];
    
    let mut user_count = 0;
    for email in valid_emails {
        let result = auth_service.create_user(CreateUserRequest {
            username: format!("emailuser{}", user_count),
            password: "ValidPassword123!".to_string(),
            email: email.map(String::from),
            display_name: None,
            roles: vec!["user".to_string()],
        }).await;
        
        assert!(result.is_ok(), "Valid email {:?} should be accepted", email);
        user_count += 1;
    }
    
    // Invalid emails
    let invalid_emails = vec![
        "notanemail",
        "missing@domain",
        "@nodomain.com",
        "spaces in@email.com",
        "user@",
        "<script>@example.com",
    ];
    
    for (i, email) in invalid_emails.iter().enumerate() {
        let result = auth_service.create_user(CreateUserRequest {
            username: format!("invalidemail{}", i),
            password: "ValidPassword123!".to_string(),
            email: Some(email.to_string()),
            display_name: None,
            roles: vec!["user".to_string()],
        }).await;
        
        assert!(result.is_err(), "Invalid email '{}' should be rejected", email);
    }
}

#[tokio::test]
async fn test_display_name_sanitization() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Test XSS attempts in display names
    let xss_attempts = vec![
        ("<script>alert('xss')</script>", "alert('xss')"),
        ("<img src=x onerror=alert(1)>", ""),
        ("Normal Name <b>Bold</b>", "Normal Name Bold"),
        ("Name\n\nWith\nNewlines", "Name\n\nWith\nNewlines"),  // Newlines might be ok
    ];
    
    for (i, (input, expected_sanitized)) in xss_attempts.iter().enumerate() {
        let result = auth_service.create_user(CreateUserRequest {
            username: format!("xsstest{}", i),
            password: "ValidPassword123!".to_string(),
            email: None,
            display_name: Some(input.to_string()),
            roles: vec!["user".to_string()],
        }).await;
        
        if let Ok(user) = result {
            let display_name = user.display_name.unwrap_or_default();
            // Check that HTML tags are removed
            assert!(!display_name.contains('<'), "Display name should not contain '<': {}", display_name);
            assert!(!display_name.contains('>'), "Display name should not contain '>': {}", display_name);
        }
    }
}

#[tokio::test]
async fn test_role_validation() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Valid roles
    let valid_roles = vec![
        vec!["user"],
        vec!["admin"],
        vec!["user", "moderator"],
        vec![],  // Empty is valid
    ];
    
    let mut user_count = 0;
    for roles in valid_roles {
        let result = auth_service.create_user(CreateUserRequest {
            username: format!("roleuser{}", user_count),
            password: "ValidPassword123!".to_string(),
            email: None,
            display_name: None,
            roles: roles.iter().map(|&s| s.to_string()).collect(),
        }).await;
        
        assert!(result.is_ok(), "Valid roles {:?} should be accepted", roles);
        user_count += 1;
    }
    
    // Invalid roles
    let invalid_roles = vec![
        vec!["hacker"],  // Not in whitelist
        vec!["admin'; DROP TABLE users; --"],  // SQL injection
        vec!["user", "admin", "moderator", "guest", "role5", "role6", "role7", "role8", "role9", "role10", "role11"],  // Too many
    ];
    
    for (i, roles) in invalid_roles.iter().enumerate() {
        let result = auth_service.create_user(CreateUserRequest {
            username: format!("invalidrole{}", i),
            password: "ValidPassword123!".to_string(),
            email: None,
            display_name: None,
            roles: roles.iter().map(|&s| s.to_string()).collect(),
        }).await;
        
        assert!(result.is_err(), "Invalid roles {:?} should be rejected", roles);
    }
}

#[tokio::test]
async fn test_search_input_validation() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Create a test user
    auth_service.create_user(CreateUserRequest {
        username: "searchuser".to_string(),
        password: "ValidPassword123!".to_string(),
        email: Some("search@example.com".to_string()),
        display_name: Some("Search User".to_string()),
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Test dangerous search patterns that should be rejected or sanitized
    let too_long_search = "a".repeat(101);
    let dangerous_searches = vec![
        "'; DROP TABLE users; --",
        "' OR '1'='1",
        "' UNION SELECT * FROM users--",
        too_long_search.as_str(),  // Too long (> 100 chars)
    ];
    
    for search in dangerous_searches {
        let filter = users_core::UserFilter {
            search: Some(search.to_string()),
            ..Default::default()
        };
        
        // Should either reject or sanitize, but not cause SQL injection
        let result = auth_service.user_store().list_users(filter).await;
        
        match result {
            Ok(users) => {
                // If accepted, should not return all users
                assert!(users.len() <= 1, "Dangerous search '{}' should not return multiple users", search);
            }
            Err(_) => {
                // Rejection is also acceptable
            }
        }
    }
}

#[tokio::test]
async fn test_api_key_name_validation() {
    let (auth_service, _temp_dir) = setup_test_auth_service().await;
    
    // Create a user for API keys
    let user = auth_service.create_user(CreateUserRequest {
        username: "apiuser".to_string(),
        password: "ValidPassword123!".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Valid API key names
    let valid_names = vec![
        "my-api-key",
        "Production Key",
        "test_key_123",
    ];
    
    for name in valid_names {
        let result = auth_service.api_key_store().create_api_key(
            users_core::api_keys::CreateApiKeyRequest {
                user_id: user.id.clone(),
                name: name.to_string(),
                permissions: vec!["read".to_string()],
                expires_at: None,
            }
        ).await;
        
        assert!(result.is_ok(), "Valid API key name '{}' should be accepted", name);
    }
    
    // Invalid API key names
    let too_long_name = "a".repeat(101);
    let invalid_names = vec![
        "",  // Empty
        too_long_name.as_str(),  // Too long
        "<script>alert('xss')</script>",  // XSS attempt
        "key'; DROP TABLE api_keys; --",  // SQL injection
    ];
    
    for name in invalid_names {
        let result = auth_service.api_key_store().create_api_key(
            users_core::api_keys::CreateApiKeyRequest {
                user_id: user.id.clone(),
                name: name.to_string(),
                permissions: vec!["read".to_string()],
                expires_at: None,
            }
        ).await;
        
        assert!(result.is_err(), "Invalid API key name '{}' should be rejected", name);
    }
}
