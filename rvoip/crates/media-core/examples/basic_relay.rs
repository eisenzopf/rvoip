//! Basic Media Relay Example
//!
//! This example demonstrates how session-core would use the MediaSessionController
//! to create a basic call relay between two SIP clients.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::collections::HashMap;
use rvoip_media_core::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Create a media session controller (this would be done in session-core)
    let controller = MediaSessionController::with_port_range(10000, 20000);
    
    // Get event receiver for monitoring
    let mut events = controller.take_event_receiver().await
        .expect("Event receiver should be available");
    
    println!("🎵 Starting Basic Media Relay Example");
    println!("=====================================");
    
    // Simulate SIP call scenario: Alice calls Bob through our server
    
    // 1. Alice initiates call - session-core creates media session for Alice
    let alice_config = MediaConfig {
        local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0),
        remote_addr: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 5004)),
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    };
    
    controller.start_media("alice_dialog".to_string(), alice_config).await?;
    println!("📞 Alice's media session started");
    
    // 2. Session-core routes call to Bob - creates media session for Bob  
    let bob_config = MediaConfig {
        local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0),
        remote_addr: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)), 5004)),
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    };
    
    controller.start_media("bob_dialog".to_string(), bob_config).await?;
    println!("📞 Bob's media session started");
    
    // 3. Session-core establishes media relay between Alice and Bob
    controller.create_relay("alice_dialog".to_string(), "bob_dialog".to_string()).await?;
    println!("🔄 Media relay established between Alice and Bob");
    
    // 4. Monitor media session events (in real usage, this would run in background)
    println!("\n📊 Media Session Events:");
    let mut event_count = 0;
    while let Ok(event) = tokio::time::timeout(
        std::time::Duration::from_millis(100), 
        events.recv()
    ).await {
        if let Some(event) = event {
            match event {
                MediaSessionEvent::SessionStarted { dialog_id, local_addr } => {
                    println!("✅ Session started: {} on {}", dialog_id, local_addr);
                },
                MediaSessionEvent::RemoteAddressUpdated { dialog_id, remote_addr } => {
                    println!("🔄 Remote address updated: {} -> {}", dialog_id, remote_addr);
                },
                MediaSessionEvent::SessionEnded { dialog_id, reason } => {
                    println!("❌ Session ended: {} ({})", dialog_id, reason);
                },
                MediaSessionEvent::SessionFailed { dialog_id, error } => {
                    println!("💥 Session failed: {} - {}", dialog_id, error);
                },
            }
            event_count += 1;
        }
        
        // Stop after processing a few events for demo
        if event_count >= 10 {
            break;
        }
    }
    
    // 5. Show session information
    println!("\n📋 Current Sessions:");
    let sessions = controller.get_all_sessions().await;
    for session in sessions {
        println!("  • Dialog: {} | Status: {:?} | Local: {} | Remote: {:?}", 
                 session.dialog_id,
                 session.status,
                 session.config.local_addr,
                 session.config.remote_addr);
    }
    
    // 6. Simulate call ending - session-core stops media sessions
    controller.stop_media("alice_dialog".to_string()).await?;
    controller.stop_media("bob_dialog".to_string()).await?;
    println!("\n📞 Call ended - media sessions stopped");
    
    println!("\n🎉 Basic relay example completed successfully!");
    println!("    ▶ Alice and Bob could now exchange audio through the relay");
    println!("    ▶ RTP packets would be forwarded bidirectionally");
    println!("    ▶ Session-core would handle SIP signaling while media-core handles audio");
    
    Ok(())
}

/// Example of how session-core might integrate with media-core
/// This shows the typical workflow for a SIP server
#[allow(dead_code)]
async fn session_core_integration_example() -> Result<()> {
    let controller = MediaSessionController::new();
    
    // Session-core receives INVITE from Alice
    // 1. Create media session for Alice
    let alice_config = MediaConfig {
        local_addr: "0.0.0.0:0".parse().unwrap(),
        remote_addr: None, // Will be updated when Alice sends RTP
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    };
    controller.start_media("alice-session-123".to_string(), alice_config).await?;
    
    // Session-core routes call to Bob
    // 2. Create media session for Bob
    let bob_config = MediaConfig {
        local_addr: "0.0.0.0:0".parse().unwrap(),
        remote_addr: None, // Will be updated when Bob sends RTP
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    };
    controller.start_media("bob-session-456".to_string(), bob_config).await?;
    
    // 3. When both parties answer, create relay
    controller.create_relay("alice-session-123".to_string(), "bob-session-456".to_string()).await?;
    
    // 4. Update remote addresses when RTP starts flowing
    controller.update_media("alice-session-123".to_string(), MediaConfig {
        local_addr: "0.0.0.0:10000".parse().unwrap(),
        remote_addr: Some("192.168.1.10:5004".parse().unwrap()),
        preferred_codec: Some("PCMU".to_string()),
        parameters: HashMap::new(),
    }).await?;
    
    // 5. When call ends, stop media sessions
    controller.stop_media("alice-session-123".to_string()).await?;
    controller.stop_media("bob-session-456".to_string()).await?;
    
    Ok(())
} 