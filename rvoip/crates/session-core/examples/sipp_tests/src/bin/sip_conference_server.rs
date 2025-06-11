//! SIP Conference Server - Multi-party conferencing with session-core Conference Module
//! 
//! Uses session-core's built-in Conference API for proper multi-party coordination.
//! This leverages the production-ready conference infrastructure.

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::api::*;
// Use the new conference module from session-core
use rvoip_session_core::conference::prelude::*;
use sipp_tests::CallStats;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    signal,
    sync::Mutex,
    time::timeout,
};
use tracing::{info, warn, error, debug};

/// 🎪 SIP Conference Server using session-core Conference Module
#[derive(Parser, Debug)]
#[command(name = "sip_conference_server")]
#[command(about = "SIP Conference Server using session-core conference infrastructure")]
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
    
    /// Enable real audio mixing (default: true)
    #[arg(long, default_value = "true")]
    enable_audio_mixing: bool,
    
    /// RTP port range base
    #[arg(long, default_value = "10000")]
    rtp_port_base: u16,
}

/// Conference call handler using session-core Conference API
pub struct SipConferenceHandler {
    conference_manager: Arc<ConferenceManager>,
    stats: Arc<Mutex<CallStats>>,
    default_config: ConferenceConfig,
    name: String,
}

impl std::fmt::Debug for SipConferenceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SipConferenceHandler")
            .field("default_config", &self.default_config)
            .field("name", &self.name)
            .finish()
    }
}

impl SipConferenceHandler {
    pub async fn new(
        max_participants: usize, 
        stats: Arc<Mutex<CallStats>>,
        audio_mixing_enabled: bool,
        rtp_port_base: u16
    ) -> Result<Self> {
        info!("🎪 Creating SIP Conference Handler with session-core Conference API (audio mixing: {})", audio_mixing_enabled);
        
        // Create conference manager from session-core
        let conference_manager = Arc::new(ConferenceManager::new());
        
        // Default conference configuration
        let default_config = ConferenceConfig {
            max_participants,
            audio_mixing_enabled,
            audio_sample_rate: 8000,
            audio_channels: 1,
            rtp_port_range: Some((rtp_port_base, rtp_port_base + 1000)),
            timeout: None,
            name: "SIPp Conference Room".to_string(),
        };
        
        Ok(Self {
            conference_manager,
            stats,
            default_config,
            name: "SIPp-Conference-SessionCore".to_string(),
        })
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
    
    async fn print_conference_stats(&self) {
        match self.conference_manager.list_conferences().await {
            Ok(conference_ids) => {
                if !conference_ids.is_empty() {
                    info!("📊 Conference Statistics (session-core Conference API):");
                    for conference_id in conference_ids {
                        match self.conference_manager.get_conference_stats(&conference_id).await {
                            Ok(stats) => {
                                info!("  🎪 Conference {}: {} participants ({} active, mixing: {})", 
                                      conference_id, stats.total_participants, stats.active_participants, stats.audio_mixing_enabled);
                                
                                if let Ok(participants) = self.conference_manager.list_participants(&conference_id).await {
                                    for participant in participants {
                                        let duration = participant.joined_at.elapsed().as_secs();
                                        let audio_status = if participant.audio_active { "🎵" } else { "🔇" };
                                        info!("    👤 {}: {}s {} (status: {:?})", 
                                              participant.session_id, duration, audio_status, participant.status);
                                    }
                                }
                            },
                            Err(e) => warn!("Failed to get stats for conference {}: {}", conference_id, e),
                        }
                    }
                }
            },
            Err(e) => warn!("Failed to list conferences: {}", e),
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for SipConferenceHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("🎪 [{}] Conference INVITE from {} to {} (session-core Conference API)", 
              self.name, call.from, call.to);
        info!("🎪 [{}] Call ID: {}", self.name, call.id);
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.total_calls += 1;
            stats.active_calls += 1;
        }
        
        // Extract conference and participant IDs
        let conference_id = ConferenceId::from_name(&self.extract_conference_id(&call.to));
        let participant_id = self.extract_participant_id(&call.from);
        
        info!("🎪 Participant {} wants to join conference {} (session-core)", 
              participant_id, conference_id);
        
        // Create session ID for this participant
        let session_id = call.id.clone(); // Use the call ID as session ID
        
        // Ensure conference exists
        if !self.conference_manager.conference_exists(&conference_id).await {
            info!("🎪 Creating new conference {} with session-core", conference_id);
            match self.conference_manager.create_named_conference(conference_id.clone(), self.default_config.clone()).await {
                Ok(()) => {
                    info!("✅ Conference {} created with session-core", conference_id);
                },
                Err(e) => {
                    error!("❌ Failed to create conference {}: {}", conference_id, e);
                    return CallDecision::Reject("Conference setup failed".to_string());
                }
            }
        }
        
        // Join participant to conference using session-core API
        match self.conference_manager.join_conference(&conference_id, &session_id).await {
            Ok(_participant_info) => {
                // Get updated conference stats
                let stats = match self.conference_manager.get_conference_stats(&conference_id).await {
                    Ok(stats) => stats,
                    Err(e) => {
                        warn!("Failed to get conference stats: {}", e);
                        return CallDecision::Reject("Conference error".to_string());
                    }
                };
                
                info!("✅ Participant {} joined conference {} via session-core ({}/{}) - Audio participants: {}", 
                      participant_id, conference_id, stats.total_participants, self.default_config.max_participants, stats.audio_participants);
                
                // Generate conference SDP using session-core
                match self.conference_manager.generate_conference_sdp(&conference_id, &session_id).await {
                    Ok(conference_sdp) => {
                        info!("🔍 Generated conference SDP via session-core for participant {}", participant_id);
                        debug!("📋 SDP content: {}", conference_sdp);
                        CallDecision::Accept(Some(conference_sdp))
                    },
                    Err(e) => {
                        error!("❌ Failed to generate conference SDP: {}", e);
                        // Try to remove the participant since we can't proceed
                        let _ = self.conference_manager.leave_conference(&conference_id, &session_id).await;
                        CallDecision::Reject("SDP generation failed".to_string())
                    }
                }
            },
            Err(e) => {
                warn!("❌ Failed to join participant to conference: {}", e);
                if e.to_string().contains("full") || e.to_string().contains("capacity") {
                    CallDecision::Reject("Conference room full".to_string())
                } else {
                    CallDecision::Reject("Conference join failed".to_string())
                }
            }
        }
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("🚪 [{}] Conference call {} ended: {}", self.name, call.id(), reason);
        
        let session_id = call.id().clone();
        
        // Find and remove participant from conference using session-core API
        match self.conference_manager.list_conferences().await {
            Ok(conference_ids) => {
                for conference_id in conference_ids {
                    // Try to remove participant from this conference
                    match self.conference_manager.leave_conference(&conference_id, &session_id).await {
                        Ok(()) => {
                            info!("📉 Participant {} left conference {} via session-core", session_id, conference_id);
                            
                            // Get updated stats
                            match self.conference_manager.get_conference_stats(&conference_id).await {
                                Ok(stats) => {
                                    info!("📊 Conference {} now has {} participants ({} active)", 
                                          conference_id, stats.total_participants, stats.active_participants);
                                    
                                    // Auto-terminate empty conferences
                                    if stats.total_participants == 0 {
                                        info!("🗑️ Conference {} is empty, terminating via session-core", conference_id);
                                        if let Err(e) = self.conference_manager.terminate_conference(&conference_id).await {
                                            warn!("Failed to terminate empty conference {}: {}", conference_id, e);
                                        } else {
                                            info!("🧹 Empty conference {} terminated via session-core", conference_id);
                                        }
                                    }
                                },
                                Err(e) => warn!("Failed to get conference stats after participant left: {}", e),
                            }
                            break; // Found the conference, stop searching
                        },
                        Err(_) => {
                            // Participant not in this conference, continue searching
                            continue;
                        }
                    }
                }
            },
            Err(e) => {
                warn!("Failed to list conferences for participant removal: {}", e);
            }
        }
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.active_calls = stats.active_calls.saturating_sub(1);
            stats.successful_calls += 1;
        }
    }
}

/// SIP Conference Server using session-core Conference Module
pub struct SipConferenceServer {
    session_manager: Arc<SessionManager>,
    conference_handler: Arc<SipConferenceHandler>,
    stats: Arc<Mutex<CallStats>>,
    start_time: Instant,
    port: u16,
    config: ConferenceConfig,
}

impl SipConferenceServer {
    /// Create a new SIP conference server using session-core Conference API
    pub async fn new(port: u16, max_participants: usize, audio_mixing_enabled: bool, rtp_port_base: u16) -> Result<Self> {
        info!("🎪 Starting SIP Conference Server with session-core Conference Module");
        info!("📡 Port: {}, Max participants: {}, Audio mixing: {}", port, max_participants, audio_mixing_enabled);
        
        let stats = Arc::new(Mutex::new(CallStats::default()));
        let handler = Arc::new(
            SipConferenceHandler::new(max_participants, Arc::clone(&stats), audio_mixing_enabled, rtp_port_base).await?
        );
        
        // Create session manager with session-core
        let session_manager = SessionManagerBuilder::new()
            .with_sip_port(port)
            .with_from_uri(format!("sip:conference@127.0.0.1:{}", port))
            .with_sip_bind_address("127.0.0.1".to_string())
            .with_media_ports(rtp_port_base, rtp_port_base + 1000)
            .p2p_mode()
            .with_handler(handler.clone())
            .build()
            .await?;
        
        let config = ConferenceConfig {
            max_participants,
            audio_mixing_enabled,
            audio_sample_rate: 8000,
            audio_channels: 1,
            rtp_port_range: Some((rtp_port_base, rtp_port_base + 1000)),
            timeout: None,
            name: "SIPp Conference Server".to_string(),
        };
        
        let server = Self {
            session_manager,
            conference_handler: handler,
            stats,
            start_time: Instant::now(),
            port,
            config,
        };
        
        Ok(server)
    }
    
    /// Start the server and handle events
    pub async fn run(&self) -> Result<()> {
        info!("🎪 Starting SIP Conference Server session manager...");
        
        // Start the session manager - this actually binds to the SIP port!
        self.session_manager.start().await?;
        
        info!("✅ SIP Conference Server ready and listening on port {}", self.port);
        info!("👥 Max participants per conference: {}", self.config.max_participants);
        info!("🎵 Audio mixing enabled: {}", self.config.audio_mixing_enabled);
        info!("🎯 Ready to handle multi-party conference calls with session-core Conference API");
        info!("📡 Real SIP conference server now active - SIPp can connect!");
        
        // Spawn stats reporting task
        let stats_handler = Arc::clone(&self.conference_handler);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                stats_handler.print_conference_stats().await;
            }
        });
        
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
        
        info!("📊 Final Conference Statistics (session-core Conference API):");
        info!("  ⏱️  Uptime: {:.2} seconds", uptime.as_secs_f64());
        info!("  🎪 Total conference participants: {}", stats.total_calls);
        info!("  ✅ Successful participants: {}", stats.successful_calls);
        info!("  ❌ Failed participants: {}", stats.failed_calls);
        info!("  🔄 Active participants: {}", stats.active_calls);
        info!("  🎵 Audio mixing enabled: {}", self.config.audio_mixing_enabled);
        info!("  📈 Success rate: {:.1}%", stats.success_rate());
        
        if stats.total_calls > 0 {
            let participants_per_second = stats.total_calls as f64 / uptime.as_secs_f64();
            info!("  🚀 Average participant rate: {:.2} participants/second", participants_per_second);
        }
        
        // Print final conference stats
        self.conference_handler.print_conference_stats().await;
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
    
    info!("🎪 Starting SIP Conference Server with session-core Conference Module");
    info!("📡 Listening on port: {}", args.port);
    info!("👥 Max participants per conference: {}", args.max_participants);
    info!("🎵 Audio mixing enabled: {}", args.enable_audio_mixing);
    info!("📡 RTP port base: {}", args.rtp_port_base);
    
    if args.timeout > 0 {
        info!("⏰ Conference timeout: {}s", args.timeout);
    } else {
        info!("⏰ No conference timeout (manual control)");
    }
    
    // Create conference server using session-core Conference API
    let conference_server = SipConferenceServer::new(
        args.port, 
        args.max_participants, 
        args.enable_audio_mixing,
        args.rtp_port_base
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