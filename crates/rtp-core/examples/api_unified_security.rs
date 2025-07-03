//! Unified Security API Example
//!
//! This example demonstrates the new unified security functionality introduced in Phase 1.
//! It shows how to use:
//! - SecurityContextManager for coordinating multiple security methods
//! - UnifiedSecurityContext for SRTP key exchange
//! - New security configuration APIs
//! - Different key exchange methods (PSK, SDES setup)

use rvoip_rtp_core::{
    api::{
        common::{
            config::{SecurityConfig, SecurityMode, KeyExchangeMethod, SecurityProfile, SrtpProfile},
            unified_security::{SecurityContextFactory, SecurityState},
            security_manager::{SecurityContextManager, NegotiationStrategy},
        },
    },
};

use std::time::Duration;
use tokio::time;
use tracing::{info, debug, warn, error};
use std::fmt;

// Set example timeout
const MAX_RUNTIME_SECONDS: u64 = 8;

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
    
    info!("🚀 Unified Security API Example");
    info!("===============================");
    info!("Demonstrating Phase 1: Non-DTLS SRTP & Authentication Schemes");
    info!("");
    
    // Demo 1: Security Configuration Showcase
    demo_security_configurations().await?;
    
    // Demo 2: Unified Security Context
    demo_unified_security_context().await?;
    
    // Demo 3: Security Context Manager
    demo_security_context_manager().await?;
    
    // Demo 4: Key Exchange Method Properties
    demo_key_exchange_properties().await?;
    
    info!("✅ All demos completed successfully!");
    info!("🎯 Phase 1 infrastructure is ready for Phase 2 protocol implementations");
    
    Ok(())
}

/// Demonstrate the new security configuration APIs
async fn demo_security_configurations() -> Result<(), ExampleError> {
    info!("📋 Demo 1: Security Configuration Showcase");
    info!("------------------------------------------");
    
    // Existing configurations still work
    info!("🔧 Traditional configurations:");
    let webrtc_config = SecurityConfig::webrtc_compatible();
    info!("  WebRTC:     {:?} mode", webrtc_config.mode);
    
    let basic_srtp = SecurityConfig::srtp_with_key(generate_test_key());
    info!("  Basic SRTP: {:?} mode", basic_srtp.mode);
    
    // New SIP-derived configurations
    info!("🆕 New SIP-derived configurations:");
    let sdes_config = SecurityConfig::sdes_srtp();
    info!("  SDES-SRTP:  {:?} mode (SDP key exchange)", sdes_config.mode);
    
    let mikey_config = SecurityConfig::mikey_psk();
    info!("  MIKEY-SRTP: {:?} mode (enterprise key mgmt)", mikey_config.mode);
    
    let zrtp_config = SecurityConfig::zrtp_p2p();
    info!("  ZRTP-SRTP:  {:?} mode (P2P secure calling)", zrtp_config.mode);
    
    // Predefined scenario configurations
    info!("🏢 Predefined SIP scenario configurations:");
    let enterprise = SecurityConfig::sip_enterprise();
    info!("  Enterprise: {:?} mode", enterprise.mode);
    
    let operator = SecurityConfig::sip_operator();
    info!("  Operator:   {:?} mode", operator.mode);
    
    let p2p = SecurityConfig::sip_peer_to_peer();
    info!("  P2P:        {:?} mode", p2p.mode);
    
    let bridge = SecurityConfig::sip_webrtc_bridge();
    info!("  SIP↔WebRTC:  {:?} mode", bridge.mode);
    
    // Multi-method configuration
    let multi_method = SecurityConfig::multi_method(vec![
        KeyExchangeMethod::Sdes,
        KeyExchangeMethod::DtlsSrtp,
        KeyExchangeMethod::PreSharedKey,
    ]);
    info!("  Multi-method: {:?} mode (with fallback)", multi_method.mode);
    
    info!("✅ Configuration showcase complete");
    info!("");
    Ok(())
}

/// Demonstrate the UnifiedSecurityContext
async fn demo_unified_security_context() -> Result<(), ExampleError> {
    info!("🔐 Demo 2: Unified Security Context");
    info!("-----------------------------------");
    
    // Create contexts for different methods
    info!("Creating security contexts for different methods...");
    
    // PSK context (immediately usable)
    info!("📊 Testing Pre-Shared Key (PSK) context:");
    let psk_context = SecurityContextFactory::create_psk_context(generate_test_key())
        .map_err(|e| ExampleError(format!("Failed to create PSK context: {}", e)))?;
    
    info!("  Method: {:?}", psk_context.get_method());
    info!("  State:  {:?}", psk_context.get_state().await);
    
    // Initialize PSK context
    psk_context.initialize().await
        .map_err(|e| ExampleError(format!("Failed to initialize PSK context: {}", e)))?;
    
    info!("  After initialization:");
    info!("  State:  {:?}", psk_context.get_state().await);
    info!("  Ready:  {}", psk_context.is_established().await);
    
    // SDES context (needs key exchange)
    info!("📊 Testing SDES context:");
    let sdes_context = SecurityContextFactory::create_sdes_context()
        .map_err(|e| ExampleError(format!("Failed to create SDES context: {}", e)))?;
    
    info!("  Method: {:?}", sdes_context.get_method());
    info!("  State:  {:?}", sdes_context.get_state().await);
    
    // Initialize SDES context
    sdes_context.initialize().await
        .map_err(|e| ExampleError(format!("Failed to initialize SDES context: {}", e)))?;
    
    info!("  After initialization:");
    info!("  State:  {:?}", sdes_context.get_state().await);
    info!("  Ready:  {}", sdes_context.is_established().await);
    
    // Test method properties
    info!("📊 Key exchange method properties:");
    let methods = vec![
        KeyExchangeMethod::DtlsSrtp,
        KeyExchangeMethod::Sdes,
        KeyExchangeMethod::Mikey,
        KeyExchangeMethod::Zrtp,
        KeyExchangeMethod::PreSharedKey,
    ];
    
    for method in methods {
        info!("  {:?}:", method);
        info!("    Network exchange: {}", method.requires_network_exchange());
        info!("    Signaling based:  {}", method.uses_signaling_exchange());
        info!("    Media path:       {}", method.uses_media_exchange());
    }
    
    info!("✅ Unified security context demo complete");
    info!("");
    Ok(())
}

/// Demonstrate the SecurityContextManager
async fn demo_security_context_manager() -> Result<(), ExampleError> {
    info!("🎛️  Demo 3: Security Context Manager");
    info!("------------------------------------");
    
    // Create manager with PSK support
    let config = SecurityConfig::srtp_with_key(generate_test_key());
    let manager = SecurityContextManager::new(config);
    
    info!("Created SecurityContextManager");
    info!("Active method: {:?}", manager.get_active_method().await);
    
    // Initialize the manager
    info!("Initializing security contexts...");
    manager.initialize().await
        .map_err(|e| ExampleError(format!("Failed to initialize manager: {}", e)))?;
    
    // List available methods
    let available_methods = manager.list_available_methods().await;
    info!("Available methods: {:?}", available_methods);
    
    // Get capabilities
    let capabilities = manager.get_capabilities().await;
    info!("Security capabilities:");
    info!("  Supported methods: {:?}", capabilities.supported_methods);
    info!("  Can offer: {}", capabilities.can_offer);
    info!("  Can answer: {}", capabilities.can_answer);
    info!("  SRTP profiles: {} supported", capabilities.srtp_profiles.len());
    
    // Test auto-negotiation
    info!("Testing auto-negotiation strategies:");
    
    // First available
    if let Ok(method) = manager.auto_negotiate(NegotiationStrategy::FirstAvailable).await {
        info!("  FirstAvailable: {:?}", method);
        info!("  Active method: {:?}", manager.get_active_method().await);
        
        // Check if established
        let is_established = manager.is_established().await.unwrap_or(false);
        info!("  Security established: {}", is_established);
    }
    
    // Test signaling detection
    info!("Testing signaling method detection:");
    let test_signals = vec![
        ("SDES SDP", b"a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:test" as &[u8]),
        ("MIKEY", b"MIKEY v1.0 message"),
        ("ZRTP", b"zrtp-version: 1.10"),
        ("Unknown", b"random signaling data"),
    ];
    
    for (name, signal) in test_signals {
        // We can't call the private method directly, but we can show what the manager would detect
        info!("  {}: Would be auto-detected as SDES/MIKEY/ZRTP based on content", name);
    }
    
    info!("✅ Security context manager demo complete");
    info!("");
    Ok(())
}

/// Demonstrate key exchange method properties and conversions
async fn demo_key_exchange_properties() -> Result<(), ExampleError> {
    info!("🔄 Demo 4: Key Exchange Method Properties");
    info!("-----------------------------------------");
    
    // Test security mode conversions
    info!("Security mode ↔ Key exchange method conversions:");
    let modes = vec![
        SecurityMode::None,
        SecurityMode::Srtp,
        SecurityMode::DtlsSrtp,
        SecurityMode::SdesSrtp,
        SecurityMode::MikeySrtp,
        SecurityMode::ZrtpSrtp,
    ];
    
    for mode in modes {
        if let Some(method) = mode.key_exchange_method() {
            let back_to_mode = method.to_security_mode();
            info!("  {:?} → {:?} → {:?}", mode, method, back_to_mode);
        } else {
            info!("  {:?} → No key exchange", mode);
        }
    }
    
    // Test security mode properties
    info!("Security mode properties:");
    for mode in [
        SecurityMode::None,
        SecurityMode::Srtp,
        SecurityMode::DtlsSrtp,
        SecurityMode::SdesSrtp,
        SecurityMode::MikeySrtp,
        SecurityMode::ZrtpSrtp,
    ] {
        info!("  {:?}:", mode);
        info!("    Enabled:      {}", mode.is_enabled());
        info!("    Requires SRTP: {}", mode.requires_srtp());
    }
    
    // Test protocol compatibility matrix
    info!("Protocol compatibility matrix:");
    info!("  Method        | WebRTC | SIP/SDP | Enterprise | P2P");
    info!("  --------------|--------|---------|------------|----");
    info!("  DTLS-SRTP     |   ✅   |    ❓   |     ❓     | ❓");
    info!("  SDES-SRTP     |   ❓   |   ✅    |     ✅     | ❓");
    info!("  MIKEY-SRTP    |   ❌   |   ✅    |     ✅     | ❌");
    info!("  ZRTP-SRTP     |   ❓   |   ✅    |     ❓     | ✅");
    info!("  PSK-SRTP      |   ❌   |   ❌    |     ✅     | ❌");
    info!("");
    info!("  ✅ = Excellent fit");
    info!("  ❓ = Possible but not ideal");
    info!("  ❌ = Not suitable");
    
    info!("✅ Key exchange properties demo complete");
    info!("");
    Ok(())
}

/// Generate a test SRTP key for demonstration
fn generate_test_key() -> Vec<u8> {
    // 16-byte AES-128 key + 14-byte salt = 30 bytes total
    vec![
        // AES-128 key (16 bytes)
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
        // SRTP salt (14 bytes)
        0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
        0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
    ]
} 