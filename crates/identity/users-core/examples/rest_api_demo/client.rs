//! REST API Client for testing all endpoints
//!
//! This client demonstrates how to interact with the users-core REST API

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

#[derive(Debug, Serialize)]
struct CreateUserRequest {
    username: String,
    password: String,
    email: Option<String>,
    display_name: Option<String>,
    roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UserResponse {
    id: String,
    username: String,
    email: Option<String>,
    display_name: Option<String>,
    roles: Vec<String>,
    active: bool,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    access_token: String,
    refresh_token: String,
    token_type: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct ApiKeyResponse {
    key: String,
    key_info: ApiKeyInfo,
}

#[derive(Debug, Deserialize)]
struct ApiKeyInfo {
    id: String,
    name: String,
    permissions: Vec<String>,
}

const BASE_URL: &str = "http://127.0.0.1:8082";

#[tokio::main]
async fn main() -> Result<()> {
    println!("🧪 Testing users-core REST API endpoints...\n");

    // Wait for server to start
    tokio::time::sleep(Duration::from_secs(2)).await;

    let client = Client::new();

    // Test 1: Health check
    println!("1️⃣ Testing health endpoint...");
    let resp = client.get(format!("{}/health", BASE_URL)).send().await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let health: serde_json::Value = resp.json().await?;
    println!("   ✅ Health check passed: {}", health["status"]);

    // Test 2: Login as admin (created by server on startup)
    println!("\n2️⃣ Testing login as admin...");
    let resp = client
        .post(format!("{}/auth/login", BASE_URL))
        .json(&json!({
            "username": "admin",
            "password": "SecurePass123"
        }))
        .send()
        .await?;

    if resp.status() != StatusCode::OK {
        let error_text = resp.text().await?;
        return Err(anyhow::anyhow!("Failed to login as admin: {}", error_text));
    }

    let login: LoginResponse = resp.json().await?;
    println!("   ✅ Admin login successful, got access token");
    let auth_header = format!("Bearer {}", login.access_token);

    // Test 3: Create regular user (requires admin auth)
    println!("\n3️⃣ Creating regular user...");
    let regular_user = CreateUserRequest {
        username: "alice".to_string(),
        password: "SecurePass456".to_string(),
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Smith".to_string()),
        roles: vec!["user".to_string()],
    };

    let resp = client
        .post(format!("{}/users", BASE_URL))
        .header("Authorization", &auth_header)
        .json(&regular_user)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let alice: UserResponse = resp.json().await?;
    println!("   ✅ User created: {} (ID: {})", alice.username, alice.id);

    // Test 4: List users
    println!("\n4️⃣ Listing users...");
    let resp = client
        .get(format!("{}/users", BASE_URL))
        .header("Authorization", &auth_header)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let users: Vec<UserResponse> = resp.json().await?;
    println!("   ✅ Found {} users:", users.len());
    for user in &users {
        println!("      - {} (roles: {:?})", user.username, user.roles);
    }

    // Test 5: Get specific user
    println!("\n5️⃣ Getting user details...");
    let resp = client
        .get(format!("{}/users/{}", BASE_URL, alice.id))
        .header("Authorization", &auth_header)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let user: UserResponse = resp.json().await?;
    println!("   ✅ Retrieved user: {}", user.username);

    // Test 6: Update user roles
    println!("\n6️⃣ Updating user roles...");
    let resp = client
        .post(format!("{}/users/{}/roles", BASE_URL, alice.id))
        .header("Authorization", &auth_header)
        .json(&json!({
            "roles": ["user", "sip", "premium"]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    println!("   ✅ Roles updated successfully");

    // Test 7: Update user details (PUT /users/:id)
    println!("\n7️⃣ Updating user details...");
    let resp = client
        .put(format!("{}/users/{}", BASE_URL, alice.id))
        .header("Authorization", &auth_header)
        .json(&json!({
            "email": "alice.smith@example.com",
            "display_name": "Alice M. Smith",
            "active": true
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let updated_user: UserResponse = resp.json().await?;
    println!("   ✅ User updated:");
    println!("      - Email: {:?}", updated_user.email);
    println!("      - Display name: {:?}", updated_user.display_name);

    // Test 8: Change password
    println!("\n8️⃣ Testing password change...");
    // First login as alice
    let resp = client
        .post(format!("{}/auth/login", BASE_URL))
        .json(&json!({
            "username": "alice",
            "password": "SecurePass456"
        }))
        .send()
        .await?;
    let alice_login: LoginResponse = resp.json().await?;

    let resp = client
        .post(format!("{}/users/{}/password", BASE_URL, alice.id))
        .header(
            "Authorization",
            format!("Bearer {}", alice_login.access_token),
        )
        .json(&json!({
            "old_password": "SecurePass456",
            "new_password": "NewSecurePass789"
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    println!("   ✅ Password changed successfully");

    // Login again with new password since tokens were revoked
    let resp = client
        .post(format!("{}/auth/login", BASE_URL))
        .json(&json!({
            "username": "alice",
            "password": "NewSecurePass789"
        }))
        .send()
        .await?;
    let alice_login = resp.json::<LoginResponse>().await?;

    // Test 9: Create API key
    println!("\n9️⃣ Creating API key...");
    let resp = client
        .post(format!("{}/users/{}/api-keys", BASE_URL, alice.id))
        .header(
            "Authorization",
            format!("Bearer {}", alice_login.access_token),
        )
        .json(&json!({
            "user_id": alice.id,
            "name": "Test API Key",
            "permissions": ["read", "write"],
            "expires_at": null
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let api_key: ApiKeyResponse = resp.json().await?;
    println!("   ✅ API key created: {}", api_key.key);

    // Test 10: Use API key authentication
    println!("\n🔟 Testing API key authentication...");
    let resp = client
        .get(format!("{}/users/{}", BASE_URL, alice.id))
        .header("X-API-Key", &api_key.key)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    println!("   ✅ API key authentication successful");

    // Test 11: List API keys
    println!("\n1️⃣1️⃣ Listing API keys...");
    let resp = client
        .get(format!("{}/users/{}/api-keys", BASE_URL, alice.id))
        .header(
            "Authorization",
            format!("Bearer {}", alice_login.access_token),
        )
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let keys: Vec<serde_json::Value> = resp.json().await?;
    println!("   ✅ Found {} API keys", keys.len());

    // Test 12: Revoke API key (DELETE /api-keys/:id)
    println!("\n1️⃣2️⃣ Revoking API key...");
    let resp = client
        .delete(format!("{}/api-keys/{}", BASE_URL, api_key.key_info.id))
        .header(
            "Authorization",
            format!("Bearer {}", alice_login.access_token),
        )
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    println!("   ✅ API key revoked successfully");

    // Test 13: Token refresh
    println!("\n1️⃣3️⃣ Testing token refresh...");
    let resp = client
        .post(format!("{}/auth/refresh", BASE_URL))
        .json(&json!({
            "refresh_token": alice_login.refresh_token
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let new_tokens: LoginResponse = resp.json().await?;
    println!("   ✅ Token refreshed successfully");

    // Test 14: Metrics endpoint
    println!("\n1️⃣4️⃣ Testing metrics endpoint...");
    let resp = client.get(format!("{}/metrics", BASE_URL)).send().await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let metrics: serde_json::Value = resp.json().await?;
    println!("   ✅ Metrics retrieved:");
    println!("      - Total users: {}", metrics["users"]["total"]);
    println!("      - API requests: {}", metrics["api_requests"]);
    println!(
        "      - Auth attempts: {}",
        metrics["authentication"]["attempts"]
    );

    // Test 15: JWKS endpoint
    println!("\n1️⃣5️⃣ Testing JWKS endpoint...");
    let resp = client
        .get(format!("{}/auth/jwks.json", BASE_URL))
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let jwks: serde_json::Value = resp.json().await?;
    println!(
        "   ✅ JWKS retrieved: {} keys",
        jwks["keys"].as_array().unwrap().len()
    );

    // Test 16: Logout
    println!("\n1️⃣6️⃣ Testing logout...");
    let resp = client
        .post(format!("{}/auth/logout", BASE_URL))
        .header(
            "Authorization",
            format!("Bearer {}", new_tokens.access_token),
        )
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    println!("   ✅ Logout successful");

    // Test 17: Password validation
    println!("\n1️⃣7️⃣ Testing password validation...");
    let weak_user = CreateUserRequest {
        username: "weakuser".to_string(),
        password: "weak".to_string(), // Too short
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    };

    let resp = client
        .post(format!("{}/users", BASE_URL))
        .header("Authorization", &auth_header)
        .json(&weak_user)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let error: serde_json::Value = resp.json().await?;
    println!(
        "   ✅ Password validation working: {}",
        error["error"]["message"]
    );

    // Test 18: Delete user
    println!("\n1️⃣8️⃣ Testing user deletion...");
    let resp = client
        .delete(format!("{}/users/{}", BASE_URL, alice.id))
        .header("Authorization", &auth_header)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    println!("   ✅ User deleted successfully");

    println!("\n✨ All 18 tests passed! The REST API is working correctly.");

    Ok(())
}
