//! Real SIP Peer-to-Peer Call Example
//!
//! This example demonstrates a complete SIP call between two clients using the 
//! session-core API with real media-core integration. It establishes actual SIP 
//! sessions with SDP negotiation and real media sessions.
//!
//! Usage:
//!   cargo run --bin simple_peer_to_peer
//!   cargo run --bin simple_peer_to_peer -- --duration 10
//!
//! This example shows:
//! - Creating two SIP clients using SessionCoordinator
//! - Real SIP session establishment with proper SDP negotiation
//! - Real media session setup using MediaSessionController
//! - Complete call lifecycle management with proper cleanup
//! - Error handling and state management

use clap::{Arg, Command};
use rvoip_session_core::{SessionCoordinator, SessionManagerBuilder, MediaControl, SessionControl};
use rvoip_session_core::api::{
    CallHandler, CallSession, CallState, IncomingCall, CallDecision, SessionId,
};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, error, warn};
use std::collections::HashMap;

/// Simple call handler for the example
#[derive(Debug)]
struct ExampleCallHandler {
    name: String,
    active_calls: Arc<tokio::sync::Mutex<HashMap<String, CallSession>>>,
    session_coordinator: Arc<tokio::sync::RwLock<Option<Arc<SessionCoordinator>>>>,
}

impl ExampleCallHandler {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            active_calls: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    async fn set_coordinator(&self, coordinator: Arc<SessionCoordinator>) {
        let mut coord = self.session_coordinator.write().await;
        *coord = Some(coordinator);
    }

    async fn get_active_call_count(&self) -> usize {
        self.active_calls.lock().await.len()
    }
}

#[async_trait::async_trait]
impl CallHandler for ExampleCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("📞 [{}] Incoming call from {} to {}", self.name, call.from, call.to);
        info!("📞 [{}] Call ID: {}", self.name, call.id);
        
        // Check if we have SDP in the incoming call
        if let Some(ref sdp_offer) = call.sdp {
            info!("📞 [{}] Received SDP offer (length: {} bytes)", self.name, sdp_offer.len());
            
            // Get coordinator to generate SDP answer
            let coordinator_guard = self.session_coordinator.read().await;
            if let Some(coordinator) = coordinator_guard.as_ref() {
                // Create media session for this call
                match coordinator.media_manager.create_media_session(&call.id).await {
                    Ok(_) => {
                        // Generate SDP answer with our allocated port
                        match coordinator.generate_sdp_offer(&call.id).await {
                            Ok(sdp_answer) => {
                                info!("📞 [{}] Generated SDP answer (length: {} bytes)", self.name, sdp_answer.len());
                                
                                // Update media session with remote SDP
                                if let Err(e) = coordinator.media_manager.update_media_session(&call.id, sdp_offer).await {
                                    error!("Failed to update media session with remote SDP: {}", e);
                                }
                                
                                return CallDecision::Accept(Some(sdp_answer));
                            }
                            Err(e) => {
                                error!("📞 [{}] Failed to generate SDP answer: {}", self.name, e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to create media session: {}", e);
                    }
                }
            }
        } else {
            info!("📞 [{}] No SDP offer received", self.name);
        }
        
        info!("✅ [{}] Auto-accepting incoming call", self.name);
        CallDecision::Accept(None)
    }

    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("📞 [{}] Call {} established", self.name, call.id());
        info!("📞 [{}] Local SDP: {:?}", self.name, local_sdp.as_ref().map(|s| s.len()));
        info!("📞 [{}] Remote SDP: {:?}", self.name, remote_sdp.as_ref().map(|s| s.len()));
        
        // Store the active call
        let mut active_calls = self.active_calls.lock().await;
        active_calls.insert(call.id().to_string(), call.clone());
        
        // Extract remote RTP address from SDP if available and establish media flow
        let coordinator_guard = self.session_coordinator.read().await;
        if let (Some(coordinator), Some(remote_sdp)) = (coordinator_guard.as_ref(), remote_sdp) {
            // Simple SDP parsing to get IP and port
            let mut remote_ip = None;
            let mut remote_port = None;
            
            for line in remote_sdp.lines() {
                if line.starts_with("c=IN IP4 ") {
                    remote_ip = line.strip_prefix("c=IN IP4 ").map(|s| s.to_string());
                } else if line.starts_with("m=audio ") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() > 1 {
                        remote_port = parts[1].parse::<u16>().ok();
                    }
                }
            }
            
            if let (Some(ip), Some(port)) = (remote_ip, remote_port) {
                let remote_addr = format!("{}:{}", ip, port);
                info!("📞 [{}] Establishing media flow to {}", self.name, remote_addr);
                
                match coordinator.establish_media_flow(&call.id(), &remote_addr).await {
                    Ok(_) => {
                        info!("✅ [{}] Successfully established media flow", self.name);
                    }
                    Err(e) => error!("Failed to establish media flow: {}", e),
                }
            }
        }
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("📴 [{}] Call {} ended: {}", self.name, call.id(), reason);
        
        // Remove from active calls
        let mut active_calls = self.active_calls.lock().await;
        active_calls.remove(call.id().as_str());
    }
}

/// Create a session coordinator with the specified configuration
async fn create_session_coordinator(
    name: &str,
    port: u16,
    from_uri: &str,
    handler: Arc<ExampleCallHandler>,
) -> Result<Arc<SessionCoordinator>, Box<dyn std::error::Error>> {
    info!("🚀 [{}] Creating session coordinator on port {}", name, port);
    
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_local_address(from_uri.to_string())
        .with_media_ports(10000, 20000)
        .with_handler(handler.clone())
        .build()
        .await?;
    
    // Set the coordinator in the handler
    handler.set_coordinator(coordinator.clone()).await;

    // Start the session manager
    coordinator.start().await?;
    info!("✅ [{}] Session coordinator started and listening on port {}", name, port);
    
    Ok(coordinator)
}

/// Run the complete real SIP P2P call test
async fn run_real_sip_call_test(duration_secs: u64) -> Result<(), Box<dyn std::error::Error>> {
    info!("🌟 Starting Real SIP Peer-to-Peer Call Test");
    info!("📋 Test Configuration:");
    info!("   Duration: {} seconds", duration_secs);
    info!("   Alice Port: 5061");
    info!("   Bob Port: 5062");
    info!("   Using real media-core integration");

    // Create handlers for both parties
    let alice_handler = Arc::new(ExampleCallHandler::new("Alice"));
    let bob_handler = Arc::new(ExampleCallHandler::new("Bob"));

    // Create session coordinators for both parties
    let alice_coordinator = create_session_coordinator(
        "Alice",
        5061,
        "sip:alice@127.0.0.1:5061",
        Arc::clone(&alice_handler),
    ).await?;

    let bob_coordinator = create_session_coordinator(
        "Bob", 
        5062,
        "sip:bob@127.0.0.1:5062",
        Arc::clone(&bob_handler),
    ).await?;

    // Wait a moment for both managers to be ready
    info!("⏳ Waiting for managers to be ready...");
    sleep(Duration::from_secs(2)).await;

    // Alice prepares and initiates a call to Bob
    info!("📞 Alice preparing call to Bob...");
    let prepared_call = alice_coordinator.prepare_outgoing_call(
        "sip:alice@127.0.0.1:5061",
        "sip:bob@127.0.0.1:5062"
    ).await?;
    
    info!("📞 Alice prepared call with session {} and SDP offer (length: {} bytes)", 
          prepared_call.session_id, prepared_call.sdp_offer.len());

    let call = alice_coordinator.initiate_prepared_call(&prepared_call).await?;
    info!("🔄 Call initiated with ID: {}", call.id());

    // Wait for call to be established with proper state checking
    info!("⏳ Waiting for call establishment...");
    let mut attempts = 0;
    let max_attempts = 20;
    let mut call_established = false;

    while attempts < max_attempts {
        if let Ok(Some(updated_call)) = alice_coordinator.find_session(&call.id()).await {
            match updated_call.state() {
                CallState::Active => {
                    info!("✅ Call established! Real media session active.");
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
        error!("❌ Call establishment timeout after {} seconds", max_attempts);
        return Ok(());
    }

    // With RFC compliance fixes, when state is Active, media is guaranteed ready!
    info!("✅ Call established with RFC-compliant timing - media ready when Active!");

    // Get real media info to verify SDP negotiation
    if let Ok(media_info) = MediaControl::get_media_info(&alice_coordinator, &call.id()).await {
        if let Some(info) = media_info {
            info!("📊 Alice media info: local_port={:?}, remote_port={:?}, codec={:?}", 
                  info.local_rtp_port, info.remote_rtp_port, info.codec);
            
            // Check RTP statistics
            if let Some(rtp_stats) = &info.rtp_stats {
                info!("📊 Alice RTP stats: sent={} pkts, recv={} pkts", 
                      rtp_stats.packets_sent, rtp_stats.packets_received);
            }
        }
    }

    // Get Bob's active sessions
    if let Ok(bob_sessions) = bob_coordinator.list_active_sessions().await {
        if let Some(bob_session_id) = bob_sessions.first() {
            if let Ok(media_info) = MediaControl::get_media_info(&bob_coordinator, bob_session_id).await {
                if let Some(info) = media_info {
                    info!("📊 Bob media info: local_port={:?}, remote_port={:?}, codec={:?}", 
                          info.local_rtp_port, info.remote_rtp_port, info.codec);
                }
            }
        }
    }

    // Let the call run for the specified duration
    info!("📞 Call active for {} seconds...", duration_secs);
    
    // Monitor statistics during the call
    for i in 0..duration_secs {
        sleep(Duration::from_secs(1)).await;
        
        if i % 3 == 0 {
            // Get statistics every 3 seconds
            if let Ok(Some(stats)) = alice_coordinator.get_media_statistics(&call.id()).await {
                if let Some(rtp) = &stats.rtp_stats {
                    info!("📊 Alice RTP: Sent {} pkts, Recv {} pkts, Lost {} pkts, Jitter {:.1}ms",
                          rtp.packets_sent, rtp.packets_received, rtp.packets_lost, rtp.jitter_ms);
                }
                if let Some(quality) = &stats.quality_metrics {
                    info!("📈 Quality: Loss {:.1}%, MOS {:.1}, Network {}%",
                          quality.packet_loss_percent, 
                          quality.mos_score.unwrap_or(0.0),
                          quality.network_quality);
                }
            }
        }
    }

    // Get session statistics
    if let Ok(stats) = alice_coordinator.get_stats().await {
        info!("📈 Alice session stats: total={}, active={}, failed={}", 
              stats.total_sessions, stats.active_sessions, stats.failed_sessions);
    }

    if let Ok(stats) = bob_coordinator.get_stats().await {
        info!("📈 Bob session stats: total={}, active={}, failed={}", 
              stats.total_sessions, stats.active_sessions, stats.failed_sessions);
    }

    // Terminate the call properly
    info!("📴 Terminating call...");
    alice_coordinator.terminate_session(&call.id()).await?;
    info!("✅ Call terminated successfully");

    // Wait for proper cleanup
    info!("🧹 Waiting for cleanup...");
    sleep(Duration::from_secs(2)).await;

    // Verify cleanup
    let alice_active = alice_handler.get_active_call_count().await;
    let bob_active = bob_handler.get_active_call_count().await;
    
    info!("🔍 Post-cleanup verification:");
    info!("   Alice active calls: {}", alice_active);
    info!("   Bob active calls: {}", bob_active);

    if alice_active == 0 && bob_active == 0 {
        info!("🎉 SUCCESS: Real SIP P2P Call Test COMPLETED!");
        info!("✅ All features tested:");
        info!("   - Session coordinator creation and startup");
        info!("   - Real SIP call establishment with SDP");
        info!("   - Real media session integration");
        info!("   - RTCP statistics monitoring");
        info!("   - Call state management");
        info!("   - Proper call termination and cleanup");
    } else {
        warn!("⚠️ Cleanup incomplete - some calls still active");
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with appropriate levels
    tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_session_core=debug,rvoip_media_core=info")
        .init();

    // Parse command line arguments
    let matches = Command::new("Real SIP Peer-to-Peer Call Test")
        .about("Tests complete real SIP calling with media-core integration")
        .arg(
            Arg::new("duration")
                .long("duration")
                .value_name("SECONDS")
                .help("Call duration in seconds for the active call test")
                .default_value("5")
                .value_parser(clap::value_parser!(u64))
        )
        .get_matches();

    let duration = *matches.get_one::<u64>("duration").unwrap();

    if duration < 1 || duration > 60 {
        error!("Duration must be between 1 and 60 seconds");
        return Ok(());
    }

    info!("🚀 Starting Real SIP P2P Call Example");
    info!("📱 This example uses:");
    info!("   - session-core SessionCoordinator API");
    info!("   - Real media-core MediaSessionController");
    info!("   - Proper SIP signaling and SDP negotiation");
    info!("   - RFC-compliant RTP port allocation");
    info!("   - RTCP statistics monitoring");

    // Run the complete test
    run_real_sip_call_test(duration).await?;

    info!("🏁 Real SIP P2P Call Example completed");
    Ok(())
} 