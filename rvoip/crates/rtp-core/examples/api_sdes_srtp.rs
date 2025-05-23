//! SDES-SRTP API Example
//!
//! This example demonstrates Phase 2: SDES (Security DEScriptions) integration
//! for SIP-style SDP-based key exchange. It shows:
//! - SDES server generating crypto offers
//! - SDES client processing offers and generating answers
//! - End-to-end SDP-based key negotiation
//! - SRTP context establishment

use rvoip_rtp_core::{
    api::{
        common::{
            config::{SecurityConfig, SecurityProfile, SrtpProfile, KeyExchangeMethod},
            unified_security::{SecurityContextFactory, SecurityState},
            security_manager::{SecurityContextManager, NegotiationStrategy},
        },
        client::security::srtp::{SdesClient, SdesClientConfig},
        server::security::srtp::{SdesServer, SdesServerConfig, SdesServerSession},
    },
};

use std::time::Duration;
use tokio::time;
use tracing::{info, debug, warn, error};
use std::fmt;

// Set example timeout
const MAX_RUNTIME_SECONDS: u64 = 10;

// Simple custom error type for the example
#[derive(Debug)]
struct ExampleError(String);

impl fmt::Display for ExampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ExampleError {}

impl From<Box<dyn std::error::Error + Send + Sync>> for ExampleError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        ExampleError(err.to_string())
    }
}

#[tokio::main]
async fn main() -> Result<(), ExampleError> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    // Set a timeout to ensure the example terminates
    let _timeout_handle = tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(MAX_RUNTIME_SECONDS)).await;
        warn!("Example timeout reached - this is normal for a demo");
        std::process::exit(0);
    });
    
    info!("🔐 SDES-SRTP API Example");
    info!("========================");
    info!("Demonstrating Phase 2: SDP-based Key Exchange for SIP Systems");
    info!("");
    
    // Demo 1: Basic SDES Server/Client Exchange
    demo_basic_sdes_exchange().await?;
    
    // Demo 2: Multiple Client Sessions
    demo_multi_client_sessions().await?;
    
    // Demo 3: Unified Security Context with SDES
    demo_unified_sdes_context().await?;
    
    // Demo 4: SDP Integration Examples
    demo_sdp_integration().await?;
    
    info!("✅ All SDES demos completed successfully!");
    info!("🎯 Phase 2 SDES integration is ready for production SIP systems");
    
    Ok(())
}

/// Demonstrate basic SDES offer/answer exchange
async fn demo_basic_sdes_exchange() -> Result<(), ExampleError> {
    info!("📡 Demo 1: Basic SDES Server/Client Exchange");
    info!("--------------------------------------------");
    
    // Create SDES server with enterprise-grade configuration
    let server_config = SdesServerConfig {
        supported_profiles: vec![
            SrtpProfile::AesCm128HmacSha1_80,
            SrtpProfile::AesCm128HmacSha1_32,
        ],
        offer_count: 2,
        require_strong_crypto: true,
        max_concurrent_exchanges: 100,
    };
    
    let sdes_server = SdesServer::new(server_config);
    info!("Created SDES server with enterprise configuration");
    
    // Create a session for a SIP call
    let session_id = "call-12345-media-session".to_string();
    let server_session = sdes_server.create_session(session_id.clone()).await
        .map_err(|e| ExampleError(format!("Failed to create server session: {}", e)))?;
    
    info!("Created server session: {}", session_id);
    
    // Server generates SDP offer with crypto attributes
    info!("🏢 Server generating SDP offer...");
    let sdp_offer = server_session.generate_offer().await
        .map_err(|e| ExampleError(format!("Failed to generate offer: {}", e)))?;
    
    info!("Server generated SDP offer:");
    for (i, line) in sdp_offer.iter().enumerate() {
        info!("  {}: {}", i + 1, line);
    }
    
    // Create SDES client (representing the remote SIP endpoint)
    let client_config = SdesClientConfig {
        supported_profiles: vec![
            SrtpProfile::AesCm128HmacSha1_80,
            SrtpProfile::AesCm128HmacSha1_32,
        ],
        strict_validation: true,
        max_crypto_attributes: 8,
    };
    
    let sdes_client = SdesClient::new(client_config);
    info!("Created SDES client representing remote SIP endpoint");
    
    // Client processes offer and generates answer
    info!("📱 Client processing SDP offer and generating answer...");
    let sdp_answer = sdes_client.process_offer(&sdp_offer).await
        .map_err(|e| ExampleError(format!("Failed to process offer: {}", e)))?;
    
    info!("Client generated SDP answer:");
    for (i, line) in sdp_answer.iter().enumerate() {
        info!("  {}: {}", i + 1, line);
    }
    
    // Server processes answer to complete key exchange
    info!("🏢 Server processing SDP answer...");
    let exchange_complete = server_session.process_answer(&sdp_answer).await
        .map_err(|e| ExampleError(format!("Failed to process answer: {}", e)))?;
    
    if exchange_complete {
        info!("✅ SDES key exchange completed successfully!");
        
        // Show the selected crypto
        if let Some(server_crypto) = server_session.get_selected_crypto_info().await {
            info!("Selected crypto attribute: {}", server_crypto);
        }
        
        if let Some(client_crypto) = sdes_client.get_selected_crypto_info().await {
            info!("Client crypto info: {}", client_crypto);
        }
        
        // Verify both sides are ready
        info!("Server session completed: {}", server_session.is_completed().await);
        info!("Client completed: {}", sdes_client.is_completed().await);
    } else {
        return Err(ExampleError("Key exchange not completed".to_string()));
    }
    
    // Clean up
    sdes_server.remove_session(&session_id).await;
    info!("Cleaned up server session");
    
    info!("✅ Basic SDES exchange demo complete");
    info!("");
    Ok(())
}

/// Demonstrate multiple concurrent client sessions
async fn demo_multi_client_sessions() -> Result<(), ExampleError> {
    info!("👥 Demo 2: Multiple Client Sessions");
    info!("----------------------------------");
    
    // Create SDES server
    let sdes_server = SdesServer::from_security_config(&SecurityConfig::sip_operator());
    info!("Created SDES server with SIP operator configuration");
    
    // Simulate multiple SIP calls
    let call_sessions = vec![
        "alice-to-bob-audio",
        "charlie-to-dave-video", 
        "eve-to-frank-conference",
    ];
    
    for (i, session_name) in call_sessions.iter().enumerate() {
        info!("📞 Processing SIP call {}: {}", i + 1, session_name);
        
        // Create server session
        let server_session = sdes_server.create_session(session_name.to_string()).await
            .map_err(|e| ExampleError(format!("Failed to create session {}: {}", session_name, e)))?;
        
        // Generate offer
        let offer = server_session.generate_offer().await
            .map_err(|e| ExampleError(format!("Failed to generate offer for {}: {}", session_name, e)))?;
        
        info!("  Generated {} crypto attributes for {}", offer.len(), session_name);
        
        // Create client for this call
        let client = SdesClient::from_security_config(&SecurityConfig::sip_operator());
        
        // Process offer -> answer
        let answer = client.process_offer(&offer).await
            .map_err(|e| ExampleError(format!("Failed to process offer for {}: {}", session_name, e)))?;
        
        // Complete exchange
        let completed = server_session.process_answer(&answer).await
            .map_err(|e| ExampleError(format!("Failed to process answer for {}: {}", session_name, e)))?;
        
        if completed {
            info!("  ✅ Key exchange completed for {}", session_name);
        } else {
            warn!("  ⚠️ Key exchange incomplete for {}", session_name);
        }
    }
    
    // Show server statistics
    let session_count = sdes_server.session_count().await;
    let session_ids = sdes_server.get_session_ids().await;
    
    info!("Server managing {} active sessions:", session_count);
    for session_id in &session_ids {
        info!("  - {}", session_id);
    }
    
    // Clean up all sessions
    for session_id in &session_ids {
        sdes_server.remove_session(session_id).await;
    }
    
    info!("All sessions cleaned up. Final count: {}", sdes_server.session_count().await);
    
    info!("✅ Multi-client sessions demo complete");
    info!("");
    Ok(())
}

/// Demonstrate unified security context with SDES
async fn demo_unified_sdes_context() -> Result<(), ExampleError> {
    info!("🔄 Demo 3: Unified Security Context with SDES");
    info!("---------------------------------------------");
    
    // Create unified security context for SDES
    let sdes_context = SecurityContextFactory::create_sdes_context()
        .map_err(|e| ExampleError(format!("Failed to create SDES context: {}", e)))?;
    
    info!("Created unified security context for SDES");
    info!("Method: {:?}", sdes_context.get_method());
    info!("Initial state: {:?}", sdes_context.get_state().await);
    
    // Initialize the context
    sdes_context.initialize().await
        .map_err(|e| ExampleError(format!("Failed to initialize SDES context: {}", e)))?;
    
    info!("After initialization:");
    info!("State: {:?}", sdes_context.get_state().await);
    info!("Established: {}", sdes_context.is_established().await);
    
    // Create security context manager with SDES preference
    let security_config = SecurityConfig::sip_operator();
    let sdes_preference = vec![
        KeyExchangeMethod::Sdes,
        KeyExchangeMethod::PreSharedKey,
    ];
    
    let security_manager = SecurityContextManager::with_method_preference(security_config, sdes_preference);
    info!("Created security manager with SDES preference");
    
    // Initialize and auto-negotiate
    security_manager.initialize().await
        .map_err(|e| ExampleError(format!("Failed to initialize security manager: {}", e)))?;
    
    let available_methods = security_manager.list_available_methods().await;
    info!("Available security methods: {:?}", available_methods);
    
    // Auto-negotiate (should select SDES)
    if let Ok(selected_method) = security_manager.auto_negotiate(NegotiationStrategy::FirstAvailable).await {
        info!("Auto-negotiated method: {:?}", selected_method);
        
        if selected_method == KeyExchangeMethod::Sdes {
            info!("✅ SDES correctly selected as preferred method");
        } else {
            warn!("⚠️ Expected SDES but got {:?}", selected_method);
        }
    }
    
    info!("✅ Unified SDES context demo complete");
    info!("");
    Ok(())
}

/// Demonstrate SDP integration examples
async fn demo_sdp_integration() -> Result<(), ExampleError> {
    info!("📄 Demo 4: SDP Integration Examples");
    info!("-----------------------------------");
    
    // Example 1: Parse realistic SDP offer
    info!("Example 1: Parsing realistic SDP offer");
    let realistic_sdp_offer = vec![
        "v=0".to_string(),
        "o=alice 2890844526 2890844527 IN IP4 host.atlanta.com".to_string(),
        "s=".to_string(),
        "c=IN IP4 host.atlanta.com".to_string(),
        "t=0 0".to_string(),
        "m=audio 49170 RTP/SAVP 0".to_string(),
        "a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:WVNfX19zZW1jdGwgKiI7NTc1BqOA1Q6YhLbGTKPC+o6yP9hZRZ6f".to_string(),
        "a=crypto:2 AES_CM_128_HMAC_SHA1_32 inline:WVNfX19zZW1jdGwgKiI7NTc1BqOA1Q6YhLbGTKPC+o6yP9hZRZ6f".to_string(),
    ];
    
    // Parse crypto attributes using client utility
    match SdesClient::parse_crypto_attributes(&realistic_sdp_offer) {
        Ok(crypto_attrs) => {
            info!("Successfully parsed {} crypto attributes:", crypto_attrs.len());
            for (i, attr) in crypto_attrs.iter().enumerate() {
                info!("  Crypto {}: tag={}, suite={}", i + 1, attr.tag, attr.crypto_suite);
            }
        },
        Err(e) => {
            warn!("Failed to parse crypto attributes: {}", e);
        }
    }
    
    // Example 2: Generate SIP-compatible offer
    info!("");
    info!("Example 2: Generate SIP-compatible offer");
    
    let sip_server = SdesServer::from_security_config(&SecurityConfig::sip_enterprise());
    let session = sip_server.create_session("sip-demo-session".to_string()).await
        .map_err(|e| ExampleError(format!("Failed to create SIP session: {}", e)))?;
    
    let sip_offer = session.generate_offer().await
        .map_err(|e| ExampleError(format!("Failed to generate SIP offer: {}", e)))?;
    
    info!("Generated SIP-compatible crypto offer:");
    for line in &sip_offer {
        info!("  {}", line);
    }
    
    // Show how this would be integrated into full SDP
    info!("");
    info!("Full SDP integration example:");
    info!("v=0");
    info!("o=sip-server 123456 654321 IN IP4 server.example.com");
    info!("s=SIP Call");
    info!("c=IN IP4 server.example.com");
    info!("t=0 0");
    info!("m=audio 5004 RTP/SAVP 0 8");
    for line in &sip_offer {
        info!("{}", line);
    }
    info!("a=sendrecv");
    
    // Example 3: Configuration for different SIP scenarios
    info!("");
    info!("Example 3: Configuration for different SIP scenarios");
    
    let scenarios = vec![
        ("Enterprise PBX", SecurityConfig::sip_enterprise()),
        ("Service Provider", SecurityConfig::sip_operator()),
        ("P2P Calling", SecurityConfig::sip_peer_to_peer()),
        ("WebRTC Bridge", SecurityConfig::sip_webrtc_bridge()),
    ];
    
    for (name, config) in scenarios {
        info!("Scenario: {}", name);
        info!("  Security mode: {:?}", config.mode);
        info!("  Profile: {:?}", config.profile);
        info!("  SRTP profiles: {} supported", config.srtp_profiles.len());
    }
    
    info!("✅ SDP integration examples complete");
    info!("");
    Ok(())
} 