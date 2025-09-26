//! Example: Complete SIP Registrar Server with Authentication
//! 
//! This example shows how to build a real SIP server that:
//! - Accepts user registrations
//! - Validates JWT authentication 
//! - Manages multiple user sessions
//! - Routes calls between registered users

use rvoip_session_core_v2::{UnifiedCoordinator, SessionBuilder, EventType};
use rvoip_dialog_core::DialogClient;
use rvoip_registrar_core::{RegistrarService, api::ServiceMode};
use rvoip_users_core::{init as init_users, UsersConfig};
use rvoip_sip_transport::{UdpTransport, TransportLayer};
use std::sync::Arc;
use tracing::{info, error};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug")
        .init();

    info!("Starting SIP Registrar Server...");

    // ==========================================
    // 1. Initialize Authentication Service
    // ==========================================
    info!("Initializing users-core authentication service...");
    
    let users_config = UsersConfig {
        database_url: "sqlite://users.db".to_string(),
        api_bind_address: "127.0.0.1:8081".parse()?,
        jwt: rvoip_users_core::jwt::JwtConfig {
            issuer: "https://sip.example.com".to_string(),
            audience: vec!["rvoip-sip".to_string()],
            access_ttl_seconds: 3600,  // 1 hour tokens
            ..Default::default()
        },
        ..Default::default()
    };
    
    let auth_service = init_users(users_config).await?;
    
    // Create a demo user for testing
    auth_service.create_user(rvoip_users_core::CreateUserRequest {
        username: "alice".to_string(),
        password: "SecurePass123!".to_string(),
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Smith".to_string()),
        roles: vec!["user".to_string()],
    }).await?;
    
    info!("Created demo user: alice (password: SecurePass123!)");
    
    // Start REST API for user authentication
    let auth_service_clone = auth_service.clone();
    tokio::spawn(async move {
        let app = rvoip_users_core::api::create_router(auth_service_clone);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:8081")
            .await
            .expect("Failed to bind auth API");
        info!("Authentication API listening on http://127.0.0.1:8081");
        axum::serve(listener, app).await.expect("Auth API failed");
    });

    // ==========================================
    // 2. Initialize Registration Service
    // ==========================================
    info!("Initializing registrar-core...");
    
    let registrar = RegistrarService::new_b2bua().await?;
    info!("Registrar service initialized in B2BUA mode");

    // ==========================================
    // 3. Initialize SIP Transport & Dialog
    // ==========================================
    info!("Initializing SIP transport on :5060...");
    
    let transport = Arc::new(
        UdpTransport::bind("0.0.0.0:5060")
            .await
            .map_err(|e| format!("Failed to bind UDP transport: {}", e))?
    );
    
    let dialog_client = DialogClient::builder()
        .with_transport(transport)
        .with_local_uri("sip:server@example.com")
        .build()
        .await?;
    
    info!("Dialog client initialized");

    // ==========================================
    // 4. Build Session Coordinator
    // ==========================================
    info!("Building session coordinator...");
    
    let coordinator = SessionBuilder::new()
        .with_dialog_client(dialog_client)
        .with_registrar_service(registrar)
        .with_users_core_url("http://127.0.0.1:8081")
        .with_state_table_path("state_tables/sip_server.yaml")
        .build()
        .await?;
    
    info!("Session coordinator ready");

    // ==========================================
    // 5. Server is Ready!
    // ==========================================
    println!("\n🚀 SIP Registrar Server is running!");
    println!("├─ SIP Port: 0.0.0.0:5060 (UDP)");
    println!("├─ Auth API: http://127.0.0.1:8081");
    println!("└─ Demo User: alice / SecurePass123!");
    
    println!("\n📝 Quick Test Guide:");
    println!("1. Get auth token:");
    println!("   curl -X POST http://localhost:8081/auth/login \\");
    println!("        -H 'Content-Type: application/json' \\");
    println!("        -d '{\"username\":\"alice\",\"password\":\"SecurePass123!\"}'");
    println!("\n2. Use token in your SIP client's Authorization header");
    println!("   Authorization: Bearer <your-token-here>");
    
    // ==========================================
    // 6. Monitor Active Sessions
    // ==========================================
    let coordinator_clone = coordinator.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            
            // Show active sessions
            match coordinator_clone.list_sessions().await {
                Ok(sessions) => {
                    info!("Active sessions: {}", sessions.len());
                    for session in sessions {
                        if let Ok(info) = coordinator_clone.get_session_info(&session.session_id).await {
                            info!(
                                "  {} - Registered: {}, State: {:?}", 
                                session.from_uri,
                                info.is_registered,
                                info.state
                            );
                        }
                    }
                }
                Err(e) => error!("Failed to list sessions: {}", e),
            }
        }
    });

    // ==========================================
    // 7. Handle Shutdown
    // ==========================================
    tokio::signal::ctrl_c().await?;
    info!("Shutting down server...");
    
    coordinator.shutdown().await?;
    info!("Server stopped");
    
    Ok(())
}

// ==========================================
// Helper Functions for Testing
// ==========================================

#[allow(dead_code)]
async fn simulate_registration(coordinator: &UnifiedCoordinator) -> Result<(), Box<dyn std::error::Error>> {
    // Simulate a REGISTER message
    coordinator.process_event(
        &"test-session".into(),
        EventType::DialogREGISTER {
            from: "sip:alice@example.com".to_string(),
            contact: "sip:alice@192.168.1.100:5060".to_string(),
            expires: 3600,
            auth_header: Some("Bearer eyJhbGciOiJS...".to_string()),
        }
    ).await?;
    
    Ok(())
}

/* 
Testing with a Real SIP Client:

1. Install a SIP client (e.g., Linphone, Zoiper, or use pjsua)

2. Get authentication token:
   ```bash
   TOKEN=$(curl -s -X POST http://localhost:8081/auth/login \
     -H 'Content-Type: application/json' \
     -d '{"username":"alice","password":"SecurePass123!"}' \
     | jq -r '.access_token')
   ```

3. Configure your SIP client:
   - Server: localhost:5060
   - Username: alice
   - Domain: example.com
   - Authorization: Bearer $TOKEN
   
   Note: Most SIP clients don't support Bearer tokens directly.
   You may need to use a custom client or pjsua with custom headers.

4. Using pjsua (command line):
   ```bash
   pjsua --config-file alice.cfg --registrar sip:localhost:5060 \
         --realm example.com --username alice \
         --custom-header "Authorization: Bearer $TOKEN"
   ```
*/
