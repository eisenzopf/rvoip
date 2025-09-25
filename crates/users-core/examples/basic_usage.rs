//! Basic usage example for users-core
//! 
//! This example demonstrates:
//! - Creating a new user
//! - Authenticating with password
//! - Refreshing tokens
//! - Changing passwords

use users_core::{init, UsersConfig, CreateUserRequest};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    // Configure users-core
    let config = UsersConfig {
        database_url: "sqlite://example_users.db?mode=rwc".to_string(),
        ..Default::default()
    };

    println!("🚀 Initializing users-core...");
    let auth_service = init(config).await?;

    // Create a new user
    println!("\n📝 Creating a new user...");
    let user = auth_service.create_user(CreateUserRequest {
        username: "alice".to_string(),
        password: "SecurePassword123!".to_string(),
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Johnson".to_string()),
        roles: vec!["user".to_string()],
    }).await?;

    println!("✅ User created successfully!");
    println!("   ID: {}", user.id);
    println!("   Username: {}", user.username);
    println!("   Roles: {:?}", user.roles);

    // Authenticate the user
    println!("\n🔐 Authenticating user...");
    let auth_result = auth_service
        .authenticate_password("alice", "SecurePassword123!")
        .await?;

    println!("✅ Authentication successful!");
    println!("   Access token (first 50 chars): {}...", &auth_result.access_token[..50]);
    println!("   Token expires in: {} seconds", auth_result.expires_in.as_secs());
    println!("   Refresh token (first 50 chars): {}...", &auth_result.refresh_token[..50]);

    // Simulate token expiration and refresh
    println!("\n🔄 Refreshing access token...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    let refreshed = auth_service
        .refresh_token(&auth_result.refresh_token)
        .await?;

    println!("✅ Token refreshed!");
    println!("   New access token (first 50 chars): {}...", &refreshed.access_token[..50]);
    println!("   Token is different: {}", refreshed.access_token != auth_result.access_token);

    // Change password
    println!("\n🔑 Changing password...");
    auth_service
        .change_password(&user.id, "SecurePassword123!", "NewSecurePassword456!")
        .await?;

    println!("✅ Password changed successfully!");

    // Try to authenticate with new password
    println!("\n🔐 Authenticating with new password...");
    let new_auth = auth_service
        .authenticate_password("alice", "NewSecurePassword456!")
        .await?;

    println!("✅ Authentication with new password successful!");

    // Old refresh token should be revoked
    println!("\n❌ Trying to use old refresh token (should fail)...");
    match auth_service.refresh_token(&auth_result.refresh_token).await {
        Ok(_) => println!("⚠️ Old token still works - unexpected!"),
        Err(e) => println!("✅ Old token rejected as expected: {}", e),
    }

    // List users
    println!("\n📋 Listing all users...");
    let all_users = auth_service.user_store()
        .list_users(Default::default())
        .await?;

    for user in all_users {
        println!("   - {} ({})", user.username, user.id);
    }

    // Clean up
    println!("\n🧹 Cleaning up test database...");
    std::fs::remove_file("example_users.db").ok();

    println!("\n✨ Example completed successfully!");
    Ok(())
}
