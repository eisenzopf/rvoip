//! Simple VoIP Client Example
//!
//! This example demonstrates how to use the rvoip-simple crate to create
//! a basic VoIP client that can make and receive calls.

use rvoip_simple::*;
use tracing::{info, error, warn};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🚀 Starting Simple VoIP Client Example");

    // Example 1: Basic client setup
    example_basic_client().await?;
    
    // Example 2: Mobile-optimized client
    example_mobile_client().await?;
    
    // Example 3: P2P calling
    example_p2p_calling().await?;
    
    // Example 4: Event handling
    example_event_handling().await?;

    info!("✅ All examples completed successfully!");
    Ok(())
}

/// Example 1: Basic VoIP client setup and connection
async fn example_basic_client() -> Result<(), SimpleVoipError> {
    info!("📞 Example 1: Basic VoIP Client");
    
    // Create and configure a simple VoIP client
    let client = SimpleVoipClient::new("alice@example.com", "secure_password")
        .with_display_name("Alice Smith")
        .with_registrar("sip.example.com")
        .with_auto_answer(false)
        .with_security(SecurityConfig::Auto)
        .connect().await?;

    info!("✅ Client connected successfully!");
    info!("   State: {:?}", client.state());
    info!("   Active calls: {}", client.active_calls().len());

    // Simulate some activity
    sleep(Duration::from_millis(500)).await;

    info!("📞 Example 1 completed\n");
    Ok(())
}

/// Example 2: Mobile-optimized client configuration
async fn example_mobile_client() -> Result<(), SimpleVoipError> {
    info!("📱 Example 2: Mobile-Optimized Client");
    
    // Create a mobile-optimized client
    let client = SimpleVoipClient::mobile("bob@mobile.com", "mobile_pass")
        .with_display_name("Bob Mobile")
        .with_call_timeout(Duration::from_secs(15)) // Shorter timeout for mobile
        .connect().await?;

    info!("✅ Mobile client connected!");
    info!("   Optimized for: Bandwidth efficiency");
    info!("   Audio quality: Bandwidth-optimized");
    info!("   Call timeout: 15 seconds");

    // Show that we can make a call (placeholder)
    info!("📞 Attempting to make a call...");
    match client.make_call("carol@example.com").await {
        Ok(call) => {
            info!("✅ Call initiated: {} -> {}", call.id, call.remote_party);
            info!("   Call state: {:?}", call.state);
            info!("   Direction: {:?}", call.direction);
        }
        Err(e) => {
            warn!("⚠️  Call failed (expected in demo): {}", e);
        }
    }

    info!("📱 Example 2 completed\n");
    Ok(())
}

/// Example 3: Peer-to-peer calling without servers
async fn example_p2p_calling() -> Result<(), SimpleVoipError> {
    info!("🔒 Example 3: P2P Secure Calling");
    
    // Create a P2P client (no server required)
    let p2p_client = SimpleVoipClient::p2p()
        .with_display_name("P2P User")
        .with_media(MediaConfig::high_quality())
        .connect().await?;

    info!("✅ P2P client ready!");
    info!("   Security: ZRTP (end-to-end encryption)");
    info!("   Mode: Peer-to-peer (no central server)");
    info!("   Media: High quality audio/video");

    // P2P calls would use direct addressing or discovery mechanisms
    info!("🔍 P2P calling would use:");
    info!("   • mDNS service discovery");
    info!("   • Direct IP addressing");
    info!("   • ZRTP for automatic security");
    info!("   • SAS verification for trust");

    info!("🔒 Example 3 completed\n");
    Ok(())
}

/// Example 4: Advanced event handling
async fn example_event_handling() -> Result<(), SimpleVoipError> {
    info!("📡 Example 4: Event Handling");
    
    // Create client with event subscription
    let mut client = SimpleVoipClient::desktop("david@events.com", "event_pass")
        .with_display_name("David Events")
        .connect().await?;

    // Subscribe to client events
    let mut events = client.subscribe_events();

    info!("✅ Client ready with event subscription");

    // Create an event listener
    let event_listener = EventListener::new(
        |event| {
            match event {
                ClientEvent::StateChanged(state) => {
                    info!("🔄 Client state changed: {:?}", state);
                }
                ClientEvent::IncomingCall(call) => {
                    info!("📞 Incoming call from: {} ({})", 
                          call.caller, 
                          call.caller_display_name.unwrap_or_default());
                }
                ClientEvent::RegistrationSuccess => {
                    info!("✅ Registration successful");
                }
                ClientEvent::RegistrationFailed(reason) => {
                    error!("❌ Registration failed: {}", reason);
                }
                ClientEvent::NetworkError(error) => {
                    error!("🌐 Network error: {}", error);
                }
            }
        },
        |event| {
            match event {
                CallEvent::StateChanged(call_id, state) => {
                    info!("📞 Call {} state: {:?}", call_id, state);
                }
                CallEvent::Answered => {
                    info!("📞 Call answered!");
                }
                CallEvent::Ended => {
                    info!("📞 Call ended");
                }
                CallEvent::MediaConnected(call_id) => {
                    info!("🎵 Media connected for call {}", call_id);
                }
                CallEvent::QualityChanged(call_id, quality) => {
                    info!("📊 Call {} quality - MOS: {:.2}, Loss: {:.1}%", 
                          call_id, quality.mos_score, quality.packet_loss);
                }
                CallEvent::DtmfReceived(call_id, digit) => {
                    info!("🔢 DTMF '{}' received on call {}", digit, call_id);
                }
                _ => {}
            }
        }
    );

    // Simulate incoming call and events
    info!("🎭 Simulating VoIP scenarios...");
    
    // Simulate making a call and handling events
    match client.make_call("echo@test.com").await {
        Ok(mut call) => {
            info!("📞 Call initiated to echo service");
            
            // Simulate call progression
            sleep(Duration::from_millis(100)).await;
            
            // Show call information
            info!("📋 Call details:");
            info!("   ID: {}", call.id);
            info!("   Remote: {}", call.remote_party);
            info!("   State: {:?}", call.state);
            info!("   Active: {}", call.is_active());
            
            // Simulate answering the call
            if call.direction == CallDirection::Incoming {
                call.answer().await?;
                info!("✅ Call answered");
            }
            
            // Simulate some call activity
            sleep(Duration::from_millis(200)).await;
            
            // Example DTMF sending
            if call.is_active() {
                call.send_dtmf_string("123#").await?;
                info!("🔢 Sent DTMF: 123#");
            }
            
            // End the call
            call.hangup().await?;
            info!("📞 Call ended normally");
        }
        Err(e) => {
            warn!("⚠️  Call failed (expected in demo): {}", e);
        }
    }

    // Show event statistics
    let mut stats = EventStats::default();
    stats.record_client_event(&ClientEvent::RegistrationSuccess);
    stats.record_call_event(&CallEvent::Answered);
    stats.record_call_event(&CallEvent::Ended);
    
    info!("📊 Event Statistics:");
    info!("   Total events: {}", stats.total_events());
    info!("   Client events: {}", stats.client_events);
    info!("   Call events: {}", stats.call_events);
    info!("   Error rate: {:.1}%", stats.error_rate());

    info!("📡 Example 4 completed\n");
    Ok(())
}

/// Demonstrate configuration presets
#[allow(dead_code)]
async fn example_configuration_presets() -> Result<(), SimpleVoipError> {
    info!("⚙️  Configuration Presets Example");
    
    // Security configurations
    let _webrtc_security = SecurityConfig::webrtc();
    let _sip_security = SecurityConfig::sip();
    let _p2p_security = SecurityConfig::p2p();
    
    // Media configurations
    let _mobile_media = MediaConfig::mobile();
    let _desktop_media = MediaConfig::desktop();
    let _voice_only = MediaConfig::voice_only();
    let _conferencing = MediaConfig::conferencing();
    let _low_bandwidth = MediaConfig::low_bandwidth();
    let _high_quality = MediaConfig::high_quality();
    
    info!("✅ All configuration presets available");
    info!("   Security: WebRTC, SIP, P2P, Enterprise PSK/PKE");
    info!("   Media: Mobile, Desktop, Voice-only, Conferencing");
    info!("   Quality: Low-bandwidth, Balanced, High-quality");
    
    Ok(())
}

/// Demonstrate error handling patterns
#[allow(dead_code)]
async fn example_error_handling() -> Result<(), SimpleVoipError> {
    info!("🚨 Error Handling Example");
    
    // Try to create a client with invalid configuration
    let result = SimpleVoipClient::new("invalid-uri", "")
        .with_registrar("non-existent-server.com")
        .connect().await;
    
    match result {
        Ok(_) => {
            warn!("Unexpected success with invalid config");
        }
        Err(e) => {
            info!("✅ Error handling working:");
            info!("   Error: {}", e);
            info!("   Recoverable: {}", e.is_recoverable());
            info!("   Config error: {}", e.is_configuration_error());
        }
    }
    
    // Show different error types
    let errors = vec![
        SimpleVoipError::network("Connection failed"),
        SimpleVoipError::sip("Invalid SIP message"),
        SimpleVoipError::security("Certificate validation failed"),
        SimpleVoipError::timeout("Operation timed out"),
    ];
    
    for error in errors {
        info!("   📋 {}: Recoverable={}", error, error.is_recoverable());
    }
    
    Ok(())
} 