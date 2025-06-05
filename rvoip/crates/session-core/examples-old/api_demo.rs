//! API Demo for RVOIP Session Core
//!
//! This example demonstrates the new organized API structure with separate
//! client and server modules under the api/ directory.

use std::sync::Arc;
use std::time::Duration;
use std::str::FromStr;
use tokio::time::sleep;
use tracing::{info, error};

use rvoip_session_core::api::{
    client::{ClientConfig, create_full_client_manager},
    server::{ServerConfig, create_full_server_manager, UserRegistration},
    get_api_capabilities, is_feature_supported,
};
use rvoip_session_core::prelude::*;

type DemoResult = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> DemoResult {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    info!("🚀 RVOIP Session Core API Demo");
    info!("================================");

    // Display API capabilities
    demo_api_capabilities().await?;
    
    // Demo API structure (doesn't require TransactionManager)
    demo_api_structure().await?;
    
    info!("✅ API Demo completed successfully!");
    Ok(())
}

async fn demo_api_capabilities() -> DemoResult {
    info!("\n📋 API Capabilities:");
    info!("-------------------");
    
    let capabilities = get_api_capabilities();
    info!("  📞 Call Transfer: {}", capabilities.call_transfer);
    info!("  🎵 Media Coordination: {}", capabilities.media_coordination);
    info!("  ⏸️  Call Hold: {}", capabilities.call_hold);
    info!("  🛣️  Call Routing: {}", capabilities.call_routing);
    info!("  👤 User Registration: {}", capabilities.user_registration);
    info!("  🎪 Conference Calls: {}", capabilities.conference_calls);
    info!("  📊 Max Sessions: {}", capabilities.max_sessions);
    
    // Check specific features
    let features = ["call_transfer", "media_coordination", "call_hold", "conference_calls"];
    for feature in features {
        let supported = is_feature_supported(feature);
        info!("  ✓ {}: {}", feature, if supported { "Supported" } else { "Not Supported" });
    }
    
    Ok(())
}

async fn demo_client_api_config() -> DemoResult {
    info!("\n📱 Client API Configuration Demo:");
    info!("----------------------------------");
    
    // Configure client
    let client_config = ClientConfig {
        display_name: "Alice's Phone".to_string(),
        uri: "sip:alice@example.com".to_string(),
        contact: "sip:alice@192.168.1.100:5060".to_string(),
        auth_user: Some("alice".to_string()),
        auth_password: Some("secret123".to_string()),
        user_agent: "RVOIP-Demo-Client/1.0".to_string(),
        max_concurrent_calls: 5,
        auto_answer: false,
        ..Default::default()
    };
    
    info!("  ✅ Client Config:");
    info!("    📞 Display Name: {}", client_config.display_name);
    info!("    📧 URI: {}", client_config.uri);
    info!("    📞 Max Calls: {}", client_config.max_concurrent_calls);
    info!("    🤖 User Agent: {}", client_config.user_agent);
    
    Ok(())
}

async fn demo_server_api_config() -> DemoResult {
    info!("\n🖥️  Server API Configuration Demo:");
    info!("-----------------------------------");
    
    // Configure server
    let server_config = ServerConfig {
        server_name: "RVOIP Demo PBX".to_string(),
        domain: "example.com".to_string(),
        max_sessions: 1000,
        session_timeout: 3600,
        max_calls_per_user: 3,
        enable_routing: true,
        enable_transfer: true,
        enable_conference: false,
        user_agent: "RVOIP-Demo-Server/1.0".to_string(),
        ..Default::default()
    };
    
    info!("  ✅ Server Config:");
    info!("    🖥️  Server Name: {}", server_config.server_name);
    info!("    🌐 Domain: {}", server_config.domain);
    info!("    📊 Max Sessions: {}", server_config.max_sessions);
    info!("    ⏱️  Session Timeout: {}s", server_config.session_timeout);
    info!("    📞 Max Calls/User: {}", server_config.max_calls_per_user);
    info!("    🛣️  Routing: {}", server_config.enable_routing);
    info!("    🔄 Transfer: {}", server_config.enable_transfer);
    
    // Demo supporting types
    info!("\n  📋 Supporting Types:");
    
    // UserRegistration demo
    let user_registration = UserRegistration {
        user_uri: Uri::sip("bob@example.com"),
        contact_uri: Uri::sip("bob@192.168.1.101:5060"),
        expires: std::time::SystemTime::now() + Duration::from_secs(3600),
        user_agent: Some("RVOIP-Demo-Client/1.0".to_string()),
    };
    info!("    👤 User Registration: {}", user_registration.user_uri);
    
    // RouteInfo demo
    let route = rvoip_session_core::api::server::RouteInfo {
        target_uri: Uri::sip("gateway@192.168.1.1"),
        priority: 1,
        weight: 100,
        description: "Primary Gateway".to_string(),
    };
    info!("    🛣️  Route: {} (priority: {}, weight: {})", route.target_uri, route.priority, route.weight);
    
    Ok(())
}

// Alternative demo that doesn't require TransactionManager
async fn demo_api_structure() -> DemoResult {
    info!("\n🏗️  API Structure Demo:");
    info!("----------------------");
    
    info!("  📁 API Organization:");
    info!("    📂 rvoip_session_core::api");
    info!("      📂 client/");
    info!("        📄 ClientConfig");
    info!("        📄 ClientSessionManager");
    info!("        📄 create_client_session_manager()");
    info!("        📄 create_full_client_manager()");
    info!("      📂 server/");
    info!("        📄 ServerConfig");
    info!("        📄 ServerSessionManager");
    info!("        📄 RouteInfo, UserRegistration");
    info!("        📄 create_server_session_manager()");
    info!("        📄 create_full_server_manager()");
    info!("      📄 ApiCapabilities, ApiConfig");
    info!("      📄 get_api_capabilities(), is_feature_supported()");
    
    info!("  ✅ Clean separation of client and server concerns");
    info!("  ✅ Comprehensive configuration options");
    info!("  ✅ Factory functions for easy setup");
    info!("  ✅ Feature detection and capabilities");
    
    // Demo configurations
    demo_client_api_config().await?;
    demo_server_api_config().await?;
    
    Ok(())
} 