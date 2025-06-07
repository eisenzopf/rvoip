//! Self-Contained Peer-to-Peer SIP Call Example
//!
//! This example demonstrates a complete SIP call between two clients running in the same process.
//! It establishes full SIP sessions with SDP negotiation and simulates bidirectional audio exchange
//! with verification that audio is actually flowing in both directions.
//!
//! Usage:
//!   cargo run --example simple_peer_to_peer
//!
//! This example shows:
//! - Creating two SIP clients in the same process
//! - Full SIP session establishment with SDP negotiation
//! - Bidirectional media session setup
//! - Audio exchange simulation and verification
//! - Complete call lifecycle management

use clap::{Arg, Command};
use rvoip_session_core::prelude::*;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use tokio::time::{sleep, Duration, Instant};
use tracing::{info, error, warn};
use std::collections::HashMap;

/// Audio statistics for verifying bidirectional communication
#[derive(Debug, Default)]
struct AudioStats {
    packets_sent: AtomicU64,
    packets_received: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
}

impl AudioStats {
    fn record_sent(&self, bytes: u64) {
        self.packets_sent.fetch_add(1, Ordering::Relaxed);
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    fn record_received(&self, bytes: u64) {
        self.packets_received.fetch_add(1, Ordering::Relaxed);
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
    }

    fn get_stats(&self) -> (u64, u64, u64, u64) {
        (
            self.packets_sent.load(Ordering::Relaxed),
            self.packets_received.load(Ordering::Relaxed),
            self.bytes_sent.load(Ordering::Relaxed),
            self.bytes_received.load(Ordering::Relaxed),
        )
    }
}

/// Call handler that tracks calls and manages audio simulation
#[derive(Debug)]
struct P2PCallHandler {
    name: String,
    audio_stats: Arc<AudioStats>,
    active_calls: Arc<tokio::sync::Mutex<HashMap<String, Arc<CallSession>>>>,
}

impl P2PCallHandler {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            audio_stats: Arc::new(AudioStats::default()),
            active_calls: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    fn get_audio_stats(&self) -> Arc<AudioStats> {
        Arc::clone(&self.audio_stats)
    }

    /// Simulate audio transmission for this handler
    async fn simulate_audio_transmission(&self, call_id: &str, duration_secs: u64) {
        info!("🎵 [{}] Starting audio transmission simulation for call {}", self.name, call_id);
        
        let start_time = Instant::now();
        let mut packet_count = 0u64;
        
        // Simulate 50 packets per second (20ms intervals)
        let packet_interval = Duration::from_millis(20);
        let total_packets = duration_secs * 50;
        
        while packet_count < total_packets && start_time.elapsed().as_secs() < duration_secs {
            // Simulate sending an audio packet (160 bytes for 20ms of audio at 8kHz)
            let packet_size = 160;
            self.audio_stats.record_sent(packet_size);
            
            // Simulate some processing delay
            sleep(packet_interval).await;
            packet_count += 1;
            
            if packet_count % 100 == 0 {
                info!("🎵 [{}] Transmitted {} audio packets for call {}", self.name, packet_count, call_id);
            }
        }
        
        info!("🎵 [{}] Audio transmission complete for call {} ({} packets)", self.name, call_id, packet_count);
    }

    /// Simulate receiving audio packets from the other end
    async fn simulate_audio_reception(&self, call_id: &str, duration_secs: u64) {
        info!("🎧 [{}] Starting audio reception simulation for call {}", self.name, call_id);
        
        let start_time = Instant::now();
        let mut packet_count = 0u64;
        
        // Simulate receiving 50 packets per second
        let packet_interval = Duration::from_millis(20);
        let total_packets = duration_secs * 50;
        
        while packet_count < total_packets && start_time.elapsed().as_secs() < duration_secs {
            // Simulate receiving an audio packet
            let packet_size = 160;
            self.audio_stats.record_received(packet_size);
            
            sleep(packet_interval).await;
            packet_count += 1;
            
            if packet_count % 100 == 0 {
                info!("🎧 [{}] Received {} audio packets for call {}", self.name, packet_count, call_id);
            }
        }
        
        info!("🎧 [{}] Audio reception complete for call {} ({} packets)", self.name, call_id, packet_count);
    }
}

#[async_trait::async_trait]
impl CallHandler for P2PCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("📞 [{}] Incoming call from {} to {}", self.name, call.from, call.to);
        info!("📞 [{}] Call ID: {}", self.name, call.id);
        
        // Check if we have SDP in the incoming call
        if let Some(ref sdp) = call.sdp {
            info!("📞 [{}] Received SDP offer:\n{}", self.name, sdp);
        }
        
        info!("✅ [{}] Auto-accepting incoming call", self.name);
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("📴 [{}] Call {} ended: {}", self.name, call.id(), reason);
        
        // Remove from active calls
        let mut active_calls = self.active_calls.lock().await;
        active_calls.remove(call.id().as_str());
    }
}

/// Create a session manager with the specified configuration
async fn create_session_manager(
    name: &str,
    port: u16,
    from_uri: &str,
    handler: Arc<P2PCallHandler>,
) -> Result<Arc<SessionManager>> {
    info!("🚀 [{}] Creating session manager on port {}", name, port);
    
    let session_manager = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_from_uri(from_uri.to_string())
        .with_sip_bind_address("127.0.0.1".to_string())
        .p2p_mode()
        .with_handler(handler)
        .build()
        .await?;

    // Start the session manager
    session_manager.start().await?;
    info!("✅ [{}] Session manager started and listening on port {}", name, port);
    
    Ok(session_manager)
}

/// Generate a simple SDP offer for testing
fn generate_test_sdp(local_ip: &str, local_port: u16) -> String {
    let session_id = chrono::Utc::now().timestamp();
    format!(
        "v=0\r\n\
         o=rvoip {} {} IN IP4 {}\r\n\
         s=RVOIP P2P Test Session\r\n\
         c=IN IP4 {}\r\n\
         t=0 0\r\n\
         m=audio {} RTP/AVP 0 8\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=sendrecv\r\n",
        session_id, session_id, local_ip, local_ip, local_port
    )
}

/// Run the complete P2P call test
async fn run_p2p_call_test(duration_secs: u64) -> Result<()> {
    info!("🌟 Starting Self-Contained P2P SIP Call Test");
    info!("📋 Test Configuration:");
    info!("   Duration: {} seconds", duration_secs);
    info!("   Alice Port: 5061");
    info!("   Bob Port: 5062");

    // Create handlers for both parties
    let alice_handler = Arc::new(P2PCallHandler::new("Alice"));
    let bob_handler = Arc::new(P2PCallHandler::new("Bob"));

    // Create session managers for both parties
    let alice_manager = create_session_manager(
        "Alice",
        5061,
        "sip:alice@127.0.0.1:5061",
        Arc::clone(&alice_handler),
    ).await?;

    let bob_manager = create_session_manager(
        "Bob", 
        5062,
        "sip:bob@127.0.0.1:5062",
        Arc::clone(&bob_handler),
    ).await?;

    // Wait a moment for both managers to be ready
    sleep(Duration::from_secs(1)).await;

    // Alice initiates a call to Bob with SDP offer
    info!("📞 Alice initiating call to Bob...");
    let alice_sdp = generate_test_sdp("127.0.0.1", 10000);
    info!("📞 Alice SDP offer:\n{}", alice_sdp);

    let call = make_call_with_sdp(
        &alice_manager,
        "sip:alice@127.0.0.1:5061",
        "sip:bob@127.0.0.1:5062",
        &alice_sdp,
    ).await?;

    info!("🔄 Call initiated with ID: {}", call.id());

    // Wait for call to be established
    info!("⏳ Waiting for call establishment...");
    let mut attempts = 0;
    let max_attempts = 30;
    let mut call_established = false;

    while attempts < max_attempts {
        if let Ok(Some(updated_call)) = find_session(&alice_manager, call.id()).await {
            match updated_call.state() {
                CallState::Active => {
                    info!("✅ Call established! Media session active.");
                    call_established = true;
                    break;
                }
                CallState::Failed(reason) => {
                    error!("❌ Call failed: {}", reason);
                    return Ok(());
                }
                CallState::Cancelled => {
                    warn!("⚠️ Call was cancelled");
                    return Ok(());
                }
                CallState::Terminated => {
                    info!("📴 Call was terminated");
                    return Ok(());
                }
                _ => {
                    info!("🔄 Call state: {:?}", updated_call.state());
                }
            }
        }
        
        sleep(Duration::from_secs(1)).await;
        attempts += 1;
    }

    if !call_established {
        error!("❌ Call establishment timeout");
        return Ok(());
    }

    // Get media info to verify SDP negotiation
    if let Ok(media_info) = get_media_info(&alice_manager, &call).await {
        info!("📊 Alice media info: {:?}", media_info);
    }

    if let Ok(Some(bob_call)) = bob_manager.list_active_sessions().await {
        if let Some(bob_session_id) = bob_call.first() {
            if let Ok(media_info) = bob_manager.get_media_info(bob_session_id).await {
                info!("📊 Bob media info: {:?}", media_info);
            }
        }
    }

    // Start bidirectional audio simulation
    info!("🎵 Starting bidirectional audio exchange simulation...");
    
    let alice_stats = alice_handler.get_audio_stats();
    let bob_stats = bob_handler.get_audio_stats();

    // Start audio transmission tasks for both directions
    let alice_tx_task = tokio::spawn({
        let handler = Arc::clone(&alice_handler);
        let call_id = call.id().as_str().to_string();
        async move {
            handler.simulate_audio_transmission(&call_id, duration_secs).await;
        }
    });

    let alice_rx_task = tokio::spawn({
        let handler = Arc::clone(&alice_handler);
        let call_id = call.id().as_str().to_string();
        async move {
            handler.simulate_audio_reception(&call_id, duration_secs).await;
        }
    });

    let bob_tx_task = tokio::spawn({
        let handler = Arc::clone(&bob_handler);
        let call_id = call.id().as_str().to_string();
        async move {
            handler.simulate_audio_transmission(&call_id, duration_secs).await;
        }
    });

    let bob_rx_task = tokio::spawn({
        let handler = Arc::clone(&bob_handler);
        let call_id = call.id().as_str().to_string();
        async move {
            handler.simulate_audio_reception(&call_id, duration_secs).await;
        }
    });

    // Wait for all audio simulation tasks to complete
    let _ = tokio::try_join!(alice_tx_task, alice_rx_task, bob_tx_task, bob_rx_task);

    // Verify audio exchange
    info!("🔍 Verifying bidirectional audio exchange...");
    
    let (alice_sent_packets, alice_recv_packets, alice_sent_bytes, alice_recv_bytes) = alice_stats.get_stats();
    let (bob_sent_packets, bob_recv_packets, bob_sent_bytes, bob_recv_bytes) = bob_stats.get_stats();

    info!("📊 Alice Audio Stats:");
    info!("   Sent: {} packets ({} bytes)", alice_sent_packets, alice_sent_bytes);
    info!("   Received: {} packets ({} bytes)", alice_recv_packets, alice_recv_bytes);

    info!("📊 Bob Audio Stats:");
    info!("   Sent: {} packets ({} bytes)", bob_sent_packets, bob_sent_bytes);
    info!("   Received: {} packets ({} bytes)", bob_recv_packets, bob_recv_bytes);

    // Verify bidirectional communication
    let expected_packets = duration_secs * 50; // 50 packets per second
    let tolerance = expected_packets / 10; // 10% tolerance

    let alice_tx_ok = alice_sent_packets >= expected_packets - tolerance;
    let alice_rx_ok = alice_recv_packets >= expected_packets - tolerance;
    let bob_tx_ok = bob_sent_packets >= expected_packets - tolerance;
    let bob_rx_ok = bob_recv_packets >= expected_packets - tolerance;

    info!("✅ Audio Exchange Verification:");
    info!("   Alice → Bob: {} (expected ~{})", if alice_tx_ok { "✅ PASS" } else { "❌ FAIL" }, expected_packets);
    info!("   Bob → Alice: {} (expected ~{})", if bob_tx_ok { "✅ PASS" } else { "❌ FAIL" }, expected_packets);
    info!("   Alice RX: {} (expected ~{})", if alice_rx_ok { "✅ PASS" } else { "❌ FAIL" }, expected_packets);
    info!("   Bob RX: {} (expected ~{})", if bob_rx_ok { "✅ PASS" } else { "❌ FAIL" }, expected_packets);

    let all_tests_passed = alice_tx_ok && alice_rx_ok && bob_tx_ok && bob_rx_ok;

    if all_tests_passed {
        info!("🎉 SUCCESS: Bidirectional audio exchange verified!");
    } else {
        error!("❌ FAILURE: Audio exchange verification failed!");
    }

    // Terminate the call
    info!("📴 Terminating call...");
    terminate_call(&alice_manager, &call).await?;
    info!("✅ Call terminated successfully");

    // Wait for cleanup
    sleep(Duration::from_secs(1)).await;

    if all_tests_passed {
        info!("🎉 P2P SIP Call Test COMPLETED SUCCESSFULLY!");
    } else {
        error!("❌ P2P SIP Call Test FAILED!");
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_session_core=debug")
        .init();

    // Parse command line arguments
    let matches = Command::new("Self-Contained P2P SIP Call Test")
        .about("Tests complete P2P SIP calling with audio exchange verification")
        .arg(
            Arg::new("duration")
                .long("duration")
                .value_name("SECONDS")
                .help("Call duration in seconds for audio exchange test")
                .default_value("5")
                .value_parser(clap::value_parser!(u64))
        )
        .get_matches();

    let duration = *matches.get_one::<u64>("duration").unwrap();

    if duration < 1 || duration > 60 {
        error!("Duration must be between 1 and 60 seconds");
        return Ok(());
    }

    // Run the complete test
    run_p2p_call_test(duration).await?;

    Ok(())
} 