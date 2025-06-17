//! SIP Conference Server - Multi-party conferencing simulation
//! 
//! This is a simplified conference server that accepts multiple concurrent calls
//! to simulate conference behavior. In a real implementation, you would use
//! the media-core conference functionality.
//!
//! NOTE: This example uses some internal implementation details.
//! For the recommended approach using only the public API, see:
//! `examples/api_best_practices/uas_server_clean.rs`

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::{SessionCoordinator, SessionManagerBuilder, MediaControl};
use rvoip_session_core::api::{
    CallHandler, CallSession, CallState, IncomingCall, CallDecision, SessionId,
};
use sipp_tests::CallStats;

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    signal,
    sync::Mutex,
    time::timeout,
};
use tracing::{info, warn, error, debug};

/// 🎪 SIP Conference Server - Simplified multi-party simulation
#[derive(Parser, Debug)]
#[command(name = "sip_conference_server")]
#[command(about = "SIP Conference Server for multi-party call testing")]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "5064")]
    port: u16,
    
    /// Maximum number of conference participants
    #[arg(short, long, default_value = "10")]
    max_participants: usize,
    
    /// Conference timeout in seconds (0 = no timeout)
    #[arg(short, long, default_value = "0")]
    timeout: u64,
    
    /// Verbose logging
    #[arg(short, long)]
    verbose: bool,
}

/// Conference room tracking
#[derive(Debug)]
struct ConferenceRoom {
    id: String,
    participants: HashMap<String, SessionId>,
    created_at: Instant,
}

/// Conference call handler - simplified version
#[derive(Debug)]
pub struct SipConferenceHandler {
    rooms: Arc<Mutex<HashMap<String, ConferenceRoom>>>,
    stats: Arc<Mutex<CallStats>>,
    max_participants: usize,
    name: String,
}

impl SipConferenceHandler {
    pub fn new(max_participants: usize, stats: Arc<Mutex<CallStats>>) -> Self {
        info!("🎪 Creating Simplified SIP Conference Handler (max participants: {})", max_participants);
        
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            stats,
            max_participants,
            name: "SIPp-Conference-Simple".to_string(),
        }
    }
    
    /// Extract conference ID from SIP To header
    fn extract_conference_id(&self, to_uri: &str) -> String {
        to_uri.split('@').next()
            .and_then(|user_part| user_part.strip_prefix("sip:"))
            .unwrap_or("default")
            .to_string()
    }
    
    /// Extract participant ID from SIP From header
    fn extract_participant_id(&self, from_uri: &str) -> String {
        from_uri.split('@').next()
            .and_then(|user_part| user_part.strip_prefix("sip:"))
            .unwrap_or("unknown")
            .to_string()
    }
}

#[async_trait::async_trait]
impl CallHandler for SipConferenceHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("🎪 [{}] Conference INVITE from {} to {}", 
              self.name, call.from, call.to);
        info!("🎪 [{}] Call ID: {}", self.name, call.id);
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.total_calls += 1;
            stats.active_calls += 1;
        }
        
        // Extract conference and participant IDs
        let conference_id = self.extract_conference_id(&call.to);
        let participant_id = self.extract_participant_id(&call.from);
        
        info!("🎪 Participant {} wants to join conference {}", 
              participant_id, conference_id);
        
        // Check/create conference room
        let mut rooms = self.rooms.lock().await;
        let room = rooms.entry(conference_id.clone()).or_insert_with(|| {
            info!("🎪 Creating new conference room: {}", conference_id);
            ConferenceRoom {
                id: conference_id.clone(),
                participants: HashMap::new(),
                created_at: Instant::now(),
            }
        });
        
        // Check capacity
        if room.participants.len() >= self.max_participants {
            warn!("❌ Conference {} is full ({}/{})", 
                  conference_id, room.participants.len(), self.max_participants);
            return CallDecision::Reject("Conference room full".to_string());
        }
        
        // Add participant
        room.participants.insert(participant_id.clone(), call.id.clone());
        
        info!("✅ Participant {} joined conference {} ({}/{} participants)", 
              participant_id, conference_id, room.participants.len(), self.max_participants);
        
        // Generate simple SDP answer
        let sdp_answer = if let Some(ref _sdp_offer) = call.sdp {
            info!("🔍 Generating conference SDP answer for participant {}", participant_id);
            // In a real implementation, this would setup media mixing
            // For now, just return a simple SDP answer
            Some(format!(
                "v=0\r\n\
                o=conference 0 0 IN IP4 127.0.0.1\r\n\
                s=Conference Session\r\n\
                c=IN IP4 127.0.0.1\r\n\
                t=0 0\r\n\
                m=audio 15000 RTP/AVP 0 8\r\n\
                a=rtpmap:0 PCMU/8000\r\n\
                a=rtpmap:8 PCMA/8000\r\n\
                a=sendrecv\r\n"
            ))
        } else {
            None
        };
        
        CallDecision::Accept(sdp_answer)
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("🚪 [{}] Conference call {} ended: {}", self.name, call.id(), reason);
        
        let session_id = call.id().to_string();
        
        // Find and remove participant from conference
        let mut rooms = self.rooms.lock().await;
        let mut empty_rooms = Vec::new();
        
        for (room_id, room) in rooms.iter_mut() {
            // Remove participant if found in this room
            if room.participants.values().any(|id| id.as_str() == session_id) {
                // Find and remove the participant
                room.participants.retain(|_, id| id.as_str() != session_id);
                
                info!("📉 Participant left conference {} ({} participants remaining)", 
                      room_id, room.participants.len());
                
                // Mark room for removal if empty
                if room.participants.is_empty() {
                    empty_rooms.push(room_id.clone());
                }
                break;
            }
        }
        
        // Remove empty rooms
        for room_id in empty_rooms {
            rooms.remove(&room_id);
            info!("🗑️ Removed empty conference room: {}", room_id);
        }
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.active_calls = stats.active_calls.saturating_sub(1);
            stats.successful_calls += 1;
        }
    }
}

/// SIP Conference Server - simplified version
pub struct SipConferenceServer {
    session_coordinator: Arc<SessionCoordinator>,
    conference_handler: Arc<SipConferenceHandler>,
    stats: Arc<Mutex<CallStats>>,
    start_time: Instant,
    port: u16,
    max_participants: usize,
}

impl SipConferenceServer {
    /// Create a new SIP conference server
    pub async fn new(port: u16, max_participants: usize) -> Result<Self> {
        info!("🎪 Starting SIP Conference Server (simplified)");
        info!("📡 Port: {}, Max participants: {}", port, max_participants);
        
        let stats = Arc::new(Mutex::new(CallStats::default()));
        let handler = Arc::new(
            SipConferenceHandler::new(max_participants, Arc::clone(&stats))
        );
        
        // Create session coordinator with session-core
        let session_coordinator = SessionManagerBuilder::new()
            .with_sip_port(port)
            .with_local_address(format!("sip:conference@127.0.0.1:{}", port))
            .with_media_ports(10000, 20000)
            .with_handler(handler.clone())
            .build()
            .await?;
        
        let server = Self {
            session_coordinator,
            conference_handler: handler,
            stats,
            start_time: Instant::now(),
            port,
            max_participants,
        };
        
        Ok(server)
    }
    
    /// Start the server and handle events
    pub async fn run(&self) -> Result<()> {
        info!("🎪 Starting SIP Conference Server session coordinator...");
        
        // Start the session coordinator - this actually binds to the SIP port!
        self.session_coordinator.start().await?;
        
        info!("✅ SIP Conference Server ready and listening on port {}", self.port);
        info!("👥 Max participants per conference: {}", self.max_participants);
        info!("🎯 Ready to handle multi-party conference calls");
        info!("📡 Real SIP conference server now active - SIPp can connect!");
        
        // Wait for shutdown signal
        self.wait_for_shutdown().await?;
        
        info!("🛑 SIP Conference Server shutting down");
        self.print_final_stats().await;
        
        Ok(())
    }
    
    /// Wait for shutdown signal
    async fn wait_for_shutdown(&self) -> Result<()> {
        // Handle Ctrl+C
        let ctrl_c = async {
            signal::ctrl_c().await.expect("Failed to install Ctrl+C handler");
            info!("📡 Received Ctrl+C signal");
        };
        
        // Handle SIGTERM
        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to install signal handler")
                .recv()
                .await;
            info!("📡 Received SIGTERM signal");
        };
        
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();
        
        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
        
        Ok(())
    }
    
    /// Print final statistics
    async fn print_final_stats(&self) {
        let stats = self.stats.lock().await;
        let uptime = self.start_time.elapsed();
        
        info!("📊 Final Conference Statistics:");
        info!("  ⏱️  Uptime: {:.2} seconds", uptime.as_secs_f64());
        info!("  🎪 Total conference participants: {}", stats.total_calls);
        info!("  ✅ Successful participants: {}", stats.successful_calls);
        info!("  ❌ Failed participants: {}", stats.failed_calls);
        info!("  🔄 Active participants: {}", stats.active_calls);
        info!("  📈 Success rate: {:.1}%", stats.success_rate());
        
        if stats.total_calls > 0 {
            let participants_per_second = stats.total_calls as f64 / uptime.as_secs_f64();
            info!("  🚀 Average participant rate: {:.2} participants/second", participants_per_second);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize logging
    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(log_level)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();
    
    info!("🎪 Starting SIP Conference Server (simplified)");
    info!("📡 Listening on port: {}", args.port);
    info!("👥 Max participants per conference: {}", args.max_participants);
    
    if args.timeout > 0 {
        info!("⏰ Conference timeout: {}s", args.timeout);
    } else {
        info!("⏰ No conference timeout (manual control)");
    }
    
    // Create conference server
    let conference_server = SipConferenceServer::new(
        args.port, 
        args.max_participants
    ).await?;
    
    // Run with timeout if specified
    if args.timeout > 0 {
        info!("⏰ Running with timeout: {} seconds", args.timeout);
        timeout(
            Duration::from_secs(args.timeout),
            conference_server.run()
        ).await??;
    } else {
        info!("🔄 Running indefinitely (script-controlled)");
        conference_server.run().await?;
    }
    
    info!("🛑 SIP Conference Server shutdown complete");
    Ok(())
}