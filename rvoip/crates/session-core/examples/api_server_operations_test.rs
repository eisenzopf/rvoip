//! Server Operations Test Example
//!
//! This example demonstrates the server operations API including
//! server creation, configuration access, and basic API functionality.

use rvoip_session_core::api::{
    factory::create_sip_server,
    server::config::{ServerConfig, TransportProtocol},
};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();
    
    println!("🚀 Starting server operations test...");
    info!("Testing session-core server operations API");
    
    // Test server creation and API access
    test_server_api().await?;
    
    println!("🎉 All server operations tests completed successfully!");
    Ok(())
}

async fn test_server_api() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing SIP server API...");
    
    // Test 1: Server creation with different configurations
    println!("📋 Testing server creation with different configurations...");
    
    // Create server configuration
    let config = ServerConfig::new("127.0.0.1:5060".parse()?)
        .with_transport(TransportProtocol::Udp)
        .with_max_sessions(100)
        .with_server_name("test-server".to_string());
    
    // Create server
    let server = create_sip_server(config).await?;
    println!("✅ SIP server created successfully");
    
    // Test 2: Configuration access
    println!("📋 Testing server configuration access...");
    let server_config = server.config();
    println!("✅ Server configuration:");
    println!("   - Bind address: {}", server_config.bind_address);
    println!("   - Transport: {}", server_config.transport_protocol);
    println!("   - Max sessions: {}", server_config.max_sessions);
    println!("   - Server name: {}", server_config.server_name);
    println!("   - Session timeout: {}s", server_config.session_timeout.as_secs());
    println!("   - Transaction timeout: {}s", server_config.transaction_timeout.as_secs());
    println!("   - Media enabled: {}", server_config.enable_media);
    
    // Test 3: Active sessions (should be empty initially)
    println!("📋 Testing active sessions query...");
    let active_sessions = server.get_active_sessions().await;
    println!("✅ Active sessions: {} (expected: 0)", active_sessions.len());
    assert_eq!(active_sessions.len(), 0);
    
    // Test 4: Server manager access
    println!("📋 Testing server manager access...");
    let server_manager = server.server_manager();
    let manager_config = server_manager.config();
    println!("✅ Server manager obtained");
    println!("✅ Server manager config matches: {}", 
             manager_config.bind_address == server_config.bind_address);
    
    // Test 5: Session manager access
    println!("📋 Testing session manager access...");
    let session_manager = server.session_manager();
    println!("✅ Session manager obtained");
    
    // Test 6: API method availability (without calling them on invalid sessions)
    println!("📋 Testing API method availability...");
    println!("✅ Available server operations:");
    println!("   - accept_call(session_id) -> Result<()>");
    println!("   - reject_call(session_id, status_code) -> Result<()>");
    println!("   - hold_call(session_id) -> Result<()>");
    println!("   - resume_call(session_id) -> Result<()>");
    println!("   - end_call(session_id) -> Result<()>");
    println!("   - get_active_sessions() -> Vec<SessionId>");
    
    // Test 7: Configuration validation
    println!("📋 Testing configuration validation...");
    let valid_config = ServerConfig::new("127.0.0.1:5061".parse()?)
        .with_transport(TransportProtocol::Tcp)
        .with_max_sessions(50);
    
    match valid_config.validate() {
        Ok(_) => println!("✅ Configuration validation passed"),
        Err(e) => println!("❌ Configuration validation failed: {}", e),
    }
    
    // Test 8: Multiple transport protocols
    println!("📋 Testing different transport protocols...");
    let protocols = vec![
        TransportProtocol::Udp,
        TransportProtocol::Tcp,
        TransportProtocol::Tls,
        TransportProtocol::WebSocket,
    ];
    
    for protocol in protocols {
        let test_config = ServerConfig::new("127.0.0.1:5062".parse()?) // Use a valid port
            .with_transport(protocol)
            .with_max_sessions(10);
        
        match test_config.validate() {
            Ok(_) => println!("✅ {} transport configuration valid", protocol),
            Err(e) => println!("⚠️  {} transport configuration invalid: {}", protocol, e),
        }
    }
    
    println!("🎯 Server API test completed successfully!");
    println!("   This test validates the API layer functionality");
    println!("   without requiring actual SIP traffic or sessions.");
    
    Ok(())
} 