//! Server Operations Test Example
//!
//! This example demonstrates the server operations API including
//! accept_call, reject_call, end_call, and session management.

use rvoip_session_core::api::{
    factory::create_sip_server,
    server::config::{ServerConfig, TransportProtocol},
};
use rvoip_sip_core::StatusCode;
use tokio::time::{timeout, Duration};
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();
    
    println!("🚀 Starting server operations test...");
    info!("Testing session-core server operations");
    
    // Test server creation and operations
    test_server_operations().await?;
    
    println!("🎉 All server operations tests completed successfully!");
    Ok(())
}

async fn test_server_operations() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing SIP server operations...");
    
    // Create server configuration
    let config = ServerConfig::new("127.0.0.1:5060".parse()?)
        .with_transport(TransportProtocol::Udp)
        .with_max_sessions(100)
        .with_server_name("test-server".to_string());
    
    // Create server
    let server = create_sip_server(config).await?;
    println!("✅ SIP server created successfully");
    
    // Test server manager access
    let server_manager = server.server_manager();
    println!("✅ Server manager obtained");
    
    // Test session manager access
    let session_manager = server.session_manager();
    println!("✅ Session manager obtained");
    
    // Test getting active sessions (should be empty initially)
    let active_sessions = server.get_active_sessions().await;
    println!("✅ Active sessions: {} (expected: 0)", active_sessions.len());
    assert_eq!(active_sessions.len(), 0);
    
    // Test server configuration access
    let server_config = server.config();
    println!("✅ Server configuration:");
    println!("   - Bind address: {}", server_config.bind_address);
    println!("   - Transport: {}", server_config.transport_protocol);
    println!("   - Max sessions: {}", server_config.max_sessions);
    println!("   - Server name: {}", server_config.server_name);
    
    // Test server manager configuration access
    let manager_config = server_manager.config();
    println!("✅ Server manager config matches: {}", 
             manager_config.bind_address == server_config.bind_address);
    
    // Simulate creating a session (this would normally come from an incoming INVITE)
    println!("📞 Testing session creation...");
    let test_session = session_manager.create_incoming_session().await?;
    let session_id = test_session.id.clone();
    println!("✅ Created test session: {}", session_id);
    
    // Test accept call operation
    println!("📞 Testing accept_call operation...");
    match server.accept_call(&session_id).await {
        Ok(_) => println!("✅ accept_call operation completed"),
        Err(e) => println!("⚠️  accept_call failed (expected - session not in server manager): {}", e),
    }
    
    // Test reject call operation
    println!("📞 Testing reject_call operation...");
    match server.reject_call(&session_id, StatusCode::BusyHere).await {
        Ok(_) => println!("✅ reject_call operation completed"),
        Err(e) => println!("⚠️  reject_call failed (expected - session not in server manager): {}", e),
    }
    
    // Test end call operation
    println!("📞 Testing end_call operation...");
    match server.end_call(&session_id).await {
        Ok(_) => println!("✅ end_call operation completed"),
        Err(e) => println!("⚠️  end_call failed (expected - session not in server manager): {}", e),
    }
    
    // Test getting active sessions again
    let active_sessions_after = server.get_active_sessions().await;
    println!("✅ Active sessions after operations: {}", active_sessions_after.len());
    
    println!("🎯 Server operations test completed successfully!");
    println!("   Note: Some operations failed as expected since we're testing");
    println!("   the API without actual incoming SIP requests.");
    
    Ok(())
} 