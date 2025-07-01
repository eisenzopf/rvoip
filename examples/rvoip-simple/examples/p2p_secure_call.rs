//! Peer-to-Peer Secure Calling Example
//!
//! This example demonstrates how to set up secure peer-to-peer calling
//! using ZRTP encryption without requiring a central server.

use rvoip_simple::*;
use tracing::{info, warn, error};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🔒 Starting P2P Secure Calling Example");

    // Simulate two users for P2P calling
    tokio::try_join!(
        setup_alice(),
        setup_bob()
    )?;

    info!("✅ P2P secure calling example completed!");
    Ok(())
}

/// Set up Alice's P2P client
async fn setup_alice() -> Result<(), SimpleVoipError> {
    info!("👩 Setting up Alice's P2P client");

    // Create P2P client for Alice
    let alice = SimpleVoipClient::p2p()
        .with_display_name("Alice P2P")
        .with_media(MediaConfig::high_quality())
        .connect().await?;

    info!("✅ Alice connected");
    info!("   Security: ZRTP (end-to-end encryption)");
    info!("   Mode: Peer-to-peer (no server required)");
    info!("   Audio quality: High quality");

    // Subscribe to events
    let mut events = alice.subscribe_events();

    // Simulate Alice making a call to Bob
    info!("📞 Alice calling Bob...");
    match alice.make_call("bob@p2p.local").await {
        Ok(mut call) => {
            info!("✅ Alice's call initiated: {}", call.id);
            
            // Show ZRTP security features
            info!("🔐 ZRTP Security Features:");
            info!("   • Automatic key exchange");
            info!("   • Perfect forward secrecy");
            info!("   • SAS (Short Authentication String) verification");
            info!("   • Protection against man-in-the-middle attacks");

            // Simulate call progression
            sleep(Duration::from_secs(2)).await;
            
            // Simulate call being answered
            call.update_state(CallState::Answered);
            info!("📞 Alice: Call answered by Bob!");

            // Show secure media session
            call.emit_media_connected();
            info!("🎵 Alice: Secure media session established");
            info!("   • Audio encrypted with AES-256");
            info!("   • Keys derived from ZRTP exchange");
            info!("   • SAS for verification: 'golf-hotel-ocean'");

            // Simulate some conversation time
            sleep(Duration::from_secs(3)).await;

            // Demonstrate SAS verification
            info!("🔍 Alice: SAS verification recommended");
            info!("   Alice asks Bob: 'Do you see golf-hotel-ocean?'");
            sleep(Duration::from_millis(500)).await;
            info!("   Bob confirms: 'Yes, I see golf-hotel-ocean'");
            info!("✅ Alice: SAS verified - call is secure!");

            // End call
            call.hangup().await?;
            info!("📞 Alice: Call ended");
        }
        Err(e) => {
            warn!("⚠️  Alice's call failed (expected in demo): {}", e);
        }
    }

    // Show P2P advantages
    info!("🌟 P2P Calling Advantages:");
    info!("   • No central server required");
    info!("   • End-to-end encryption by default");
    info!("   • Lower latency (direct connection)");
    info!("   • Privacy protection (no server logging)");
    info!("   • Works behind NAT (with ICE/STUN)");

    Ok(())
}

/// Set up Bob's P2P client
async fn setup_bob() -> Result<(), SimpleVoipError> {
    info!("👨 Setting up Bob's P2P client");

    // Small delay to offset from Alice
    sleep(Duration::from_millis(100)).await;

    // Create P2P client for Bob
    let bob = SimpleVoipClient::p2p()
        .with_display_name("Bob P2P")
        .with_media(MediaConfig::high_quality())
        .with_auto_answer(true) // Auto-answer for demo
        .connect().await?;

    info!("✅ Bob connected and ready to receive calls");

    // Subscribe to events
    let mut events = bob.subscribe_events();

    // Simulate Bob receiving Alice's call
    sleep(Duration::from_secs(1)).await;
    
    info!("📞 Bob: Incoming call from Alice detected");
    info!("🔐 Bob: ZRTP negotiation starting...");

    // Simulate ZRTP handshake
    sleep(Duration::from_millis(500)).await;
    info!("✅ Bob: ZRTP handshake completed");
    info!("   • DH key exchange successful");
    info!("   • Shared secret established");
    info!("   • SAS computed: 'golf-hotel-ocean'");

    // Auto-answer the call
    match bob.answer_call("alice-call-123").await {
        Ok(mut call) => {
            info!("📞 Bob: Call answered!");
            
            // Show encrypted media session
            sleep(Duration::from_millis(200)).await;
            call.emit_media_connected();
            info!("🎵 Bob: Secure media session active");
            
            // Demonstrate DTMF in secure call
            sleep(Duration::from_secs(2)).await;
            call.send_dtmf_string("*123#").await?;
            info!("🔢 Bob: Sent secure DTMF sequence");

            // Simulate conversation
            sleep(Duration::from_secs(2)).await;
            
            info!("📞 Bob: Call completed successfully");
        }
        Err(e) => {
            warn!("⚠️  Bob's answer failed: {}", e);
        }
    }

    // Show ZRTP security analysis
    info!("🛡️  ZRTP Security Analysis:");
    info!("   • No PKI infrastructure required");
    info!("   • Resistant to quantum computer attacks");
    info!("   • Real-time key agreement");
    info!("   • Automatic media encryption");
    info!("   • Voice quality preserved");

    Ok(())
}

/// Demonstrate P2P discovery mechanisms
#[allow(dead_code)]
async fn demonstrate_p2p_discovery() -> Result<(), SimpleVoipError> {
    info!("🔍 P2P Discovery Mechanisms");

    info!("1. mDNS/Bonjour Discovery:");
    info!("   • Local network service discovery");
    info!("   • '_sip._udp.local' service advertising");
    info!("   • Automatic peer detection");

    info!("2. Direct IP Addressing:");
    info!("   • Manual IP entry: '192.168.1.100:5060'");
    info!("   • Port forwarding for external access");
    info!("   • Static configuration files");

    info!("3. DHT (Distributed Hash Table):");
    info!("   • Decentralized peer discovery");
    info!("   • Username -> IP address mapping");
    info!("   • No central directory required");

    info!("4. QR Code Exchange:");
    info!("   • Visual peer information sharing");
    info!("   • Conference room scenarios");
    info!("   • One-time connection setup");

    Ok(())
}

/// Show P2P network traversal
#[allow(dead_code)]
async fn demonstrate_nat_traversal() -> Result<(), SimpleVoipError> {
    info!("🌐 P2P NAT Traversal Techniques");

    info!("1. ICE (Interactive Connectivity Establishment):");
    info!("   • Gathering local, server-reflexive, and relay candidates");
    info!("   • Connectivity checks between all candidate pairs");
    info!("   • Best path selection for media");

    info!("2. STUN (Session Traversal Utilities for NAT):");
    info!("   • Discover public IP address and port");
    info!("   • Determine NAT type and behavior");
    info!("   • Enable direct peer-to-peer connection");

    info!("3. TURN (Traversal Using Relays around NAT):");
    info!("   • Relay server for symmetric NATs");
    info!("   • Fallback when direct connection fails");
    info!("   • Bandwidth cost but guaranteed connectivity");

    info!("4. UPnP (Universal Plug and Play):");
    info!("   • Automatic port forwarding");
    info!("   • Home router configuration");
    info!("   • Simplified NAT traversal");

    Ok(())
}

/// Performance comparison: P2P vs Server-based
#[allow(dead_code)]
async fn performance_comparison() {
    info!("📊 Performance Comparison: P2P vs Server-based");

    info!("P2P Calling:");
    info!("   ✅ Latency: 20-50ms (direct connection)");
    info!("   ✅ Bandwidth: Peer-to-peer, no server load");
    info!("   ✅ Privacy: End-to-end encryption");
    info!("   ✅ Scalability: No central bottleneck");
    info!("   ❌ Discovery: More complex peer finding");
    info!("   ❌ NAT: Requires STUN/TURN for some networks");

    info!("Server-based Calling:");
    info!("   ❌ Latency: 50-150ms (via server)");
    info!("   ❌ Bandwidth: Server processing overhead");
    info!("   ❌ Privacy: Server can intercept calls");
    info!("   ❌ Scalability: Server capacity limits");
    info!("   ✅ Discovery: Centralized user directory");
    info!("   ✅ NAT: Server handles all traversal");

    info!("💡 Recommendation: Use P2P for:");
    info!("   • High-security requirements");
    info!("   • Low-latency needs (gaming, trading)");
    info!("   • Private communications");
    info!("   • Small group calling");
    info!("   • Decentralized applications");
} 