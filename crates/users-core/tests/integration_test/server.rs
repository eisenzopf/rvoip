//! Integration test server for users-core
//! 
//! This module provides a test server with aggressive rate limiting
//! configurations for integration testing.

use users_core::{
    init, UsersConfig, CreateUserRequest,
    api::{ApiState, Metrics},
    api::rate_limit::{EnhancedRateLimiter, RateLimitConfig},
    AuthenticationService,
    config::{PasswordConfig, TlsSettings},
    jwt::JwtConfig,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tracing::info;

/// Test server configuration and handle
pub struct TestServer {
    pub url: String,
    pub auth_service: Arc<AuthenticationService>,
    _temp_dir: TempDir,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl TestServer {
    /// Shutdown the test server
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        // Small delay to ensure server stops
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Start a test server with aggressive rate limiting for integration tests
pub async fn start_test_server() -> anyhow::Result<TestServer> {
    // Initialize logging
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("users_core=debug,tower_http=debug")
        .try_init();

    // Create temporary directory for database
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test_users.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    // Configure with aggressive rate limits for testing
    let config = UsersConfig {
        database_url: db_url,
        jwt: JwtConfig {
            issuer: "https://users.rvoip.local".to_string(),
            audience: vec!["rvoip-api".to_string()],
            access_ttl_seconds: 300,  // 5 minutes for testing
            refresh_ttl_seconds: 600, // 10 minutes for testing
            algorithm: "RS256".to_string(),
            signing_key: None, // Will generate a key
        },
        password: PasswordConfig {
            min_length: 12,  // Updated to match validation.rs
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,
            argon2_memory_cost: 4096,  // Lower for faster tests
            argon2_time_cost: 1,       // Lower for faster tests
            argon2_parallelism: 1,     // Lower for faster tests
        },
        api_bind_address: "127.0.0.1:0".to_string(), // Let OS assign port
        tls: TlsSettings::default(), // Default TLS settings (disabled)
    };

    // Initialize authentication service
    let auth_service = Arc::new(init(config.clone()).await?);
    info!("Test auth service initialized");
    
    // Create test admin user
    match auth_service.create_user(CreateUserRequest {
        username: "admin".to_string(),
        password: "SecurePass123".to_string(),
        email: Some("admin@test.local".to_string()),
        display_name: Some("Test Admin".to_string()),
        roles: vec!["admin".to_string()],
    }).await {
        Ok(user) => info!("Test admin user created: {}", user.username),
        Err(users_core::Error::UserAlreadyExists(_)) => info!("Admin user already exists"),
        Err(e) => return Err(e.into()),
    }

    // Create test regular user
    match auth_service.create_user(CreateUserRequest {
        username: "testuser".to_string(),
        password: "SecurePass123".to_string(),
        email: Some("test@test.local".to_string()),
        display_name: Some("Test User".to_string()),
        roles: vec!["user".to_string()],
    }).await {
        Ok(user) => info!("Test regular user created: {}", user.username),
        Err(users_core::Error::UserAlreadyExists(_)) => info!("Test user already exists"),
        Err(e) => return Err(e.into()),
    }

    // Configure aggressive rate limiting for testing
    let rate_limit_config = RateLimitConfig {
        requests_per_minute: 10,           // Very low for testing
        requests_per_hour: 100,            // Low for testing
        login_attempts_per_hour: 3,        // Only 3 attempts before lockout
        lockout_duration: Duration::from_secs(2), // 2 seconds for quick testing
        cleanup_interval: Duration::from_secs(60),
    };
    
    // Create API state with custom rate limiter
    let rate_limiter = EnhancedRateLimiter::new(rate_limit_config);
    let api_state = ApiState {
        auth_service: auth_service.clone(),
        rate_limiter,
        metrics: Arc::new(Mutex::new(Metrics {
            start_time: Instant::now(),
            ..Default::default()
        })),
    };
    
    // Create router with our custom state that has aggressive rate limiting
    let app = users_core::api::create_router_with_state(api_state);
    
    // Bind to any available port
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let url = format!("http://{}", addr);
    
    info!("Test server starting on {}", url);
    
    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    
    // Spawn server task
    tokio::spawn(async move {
        let server = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
        
        if let Err(e) = server.await {
            tracing::error!("Server error: {}", e);
        }
    });
    
    // Wait a bit for server to start
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    Ok(TestServer {
        url,
        auth_service,
        _temp_dir: temp_dir,
        shutdown_tx,
    })
}

/// Create a test user for testing
pub async fn create_test_user(
    auth_service: &AuthenticationService,
    username: &str,
    password: &str,
) -> anyhow::Result<users_core::User> {
    Ok(auth_service.create_user(CreateUserRequest {
        username: username.to_string(),
        password: password.to_string(),
        email: Some(format!("{}@test.local", username)),
        display_name: Some(format!("Test User {}", username)),
        roles: vec!["user".to_string()],
    }).await?)
}
