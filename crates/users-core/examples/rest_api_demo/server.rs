//! REST API Server for demo
//! 
//! This is a minimal server that starts on a specific port for testing

use users_core::{init, UsersConfig, api::create_router};
use std::sync::Arc;
use tracing::info;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    info!("Starting users-core REST API demo server...");

    // Use hardcoded configuration for demo simplicity
    // See users_core.toml for a complete example of configuration options
    let config = UsersConfig {
        database_url: "sqlite://examples/rest_api_demo/demo.db?mode=rwc".to_string(),
        api_bind_address: "127.0.0.1:8082".to_string(), // Different port for demo
        ..Default::default()
    };

    let auth_service = Arc::new(init(config.clone()).await?);
    info!("Users-core initialized");
    
    // Create initial admin user for demo
    info!("Creating initial admin user for demo...");
    match auth_service.create_user(users_core::CreateUserRequest {
        username: "admin".to_string(),
        password: "AdminPass123".to_string(),
        email: Some("admin@example.com".to_string()),
        display_name: Some("Demo Admin".to_string()),
        roles: vec!["admin".to_string()],
    }).await {
        Ok(user) => info!("Admin user created: {}", user.username),
        Err(users_core::Error::UserAlreadyExists(_)) => info!("Admin user already exists"),
        Err(e) => return Err(e.into()),
    }

    // Create router
    let app = create_router(auth_service);

    // Start server
    let addr = config.api_bind_address.parse::<std::net::SocketAddr>()?;
    info!("REST API listening on http://{}", addr);
    
    // Print available endpoints
    println!("\n📋 Available endpoints:");
    println!("\nAuthentication:");
    println!("  POST   /auth/login              - Login with username/password");
    println!("  POST   /auth/logout             - Logout (revoke tokens)");
    println!("  POST   /auth/refresh            - Refresh access token");
    println!("  GET    /auth/jwks.json          - Get public keys for validation");
    
    println!("\nUser Management:");
    println!("  POST   /users                   - Create new user");
    println!("  GET    /users                   - List users");
    println!("  GET    /users/:id               - Get user details");
    println!("  PUT    /users/:id               - Update user");
    println!("  DELETE /users/:id               - Delete user");
    println!("  POST   /users/:id/password      - Change password");
    println!("  POST   /users/:id/roles         - Update user roles");
    
    println!("\nAPI Keys:");
    println!("  POST   /users/:id/api-keys      - Create API key");
    println!("  GET    /users/:id/api-keys      - List API keys");
    println!("  DELETE /api-keys/:id            - Revoke API key");
    
    println!("\nHealth & Metrics:");
    println!("  GET    /health                  - Health check");
    println!("  GET    /metrics                 - Service metrics");
    
    println!("\n🔑 Demo credentials:");
    println!("  Username: admin");
    println!("  Password: AdminPass123");
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
