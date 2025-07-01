//! Clean UAS Server Example - Best Practices
//! 
//! This example demonstrates the recommended way to build a SIP User Agent Server (UAS)
//! using the session-core API with the new extended event callbacks and improved
//! statistics methods.
//!
//! Key patterns demonstrated:
//! - Using extended CallHandler callbacks for comprehensive event handling
//! - Leveraging new convenience statistics methods
//! - Automatic media quality monitoring with alerts
//! - Rich state tracking and session management

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::api::{
    CallHandler, CallDecision, IncomingCall, CallSession, CallState,
    SessionManagerBuilder, SessionControl, MediaControl,
    SessionCoordinator, parse_sdp_connection, SessionId,
    // New imports from the refactor
    MediaQualityAlertLevel, MediaFlowDirection, WarningCategory,
    CallStatistics,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, error, warn};

#[derive(Parser, Debug)]
#[command(name = "uas_server_clean")]
#[command(about = "Clean UAS Server demonstrating API best practices")]
struct Args {
    /// SIP listening port
    #[arg(short, long, default_value = "5062")]
    port: u16,
    
    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,
    
    /// Auto-accept incoming calls
    #[arg(short, long, default_value = "true")]
    auto_accept: bool,
    
    /// Maximum concurrent calls
    #[arg(short, long, default_value = "10")]
    max_calls: usize,
}

/// Enhanced statistics tracking for the UAS
#[derive(Debug, Default)]
struct UasStats {
    calls_received: usize,
    calls_accepted: usize,
    calls_rejected: usize,
    calls_active: usize,
    total_duration: Duration,
    // New fields for extended tracking
    state_changes: usize,
    media_flow_events: usize,
    quality_alerts: usize,
    dtmf_received: usize,
    warnings_received: usize,
}

/// Enhanced UAS handler demonstrating new API features
#[derive(Debug)]
struct CleanUasHandler {
    stats: Arc<Mutex<UasStats>>,
    auto_accept: bool,
    max_calls: usize,
    // Store coordinator reference for API calls
    coordinator: Arc<tokio::sync::RwLock<Option<Arc<SessionCoordinator>>>>,
}

impl CleanUasHandler {
    fn new(auto_accept: bool, max_calls: usize) -> Self {
        Self {
            stats: Arc::new(Mutex::new(UasStats::default())),
            auto_accept,
            max_calls,
            coordinator: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    async fn set_coordinator(&self, coordinator: Arc<SessionCoordinator>) {
        *self.coordinator.write().await = Some(coordinator);
    }
}

#[async_trait::async_trait]
impl CallHandler for CleanUasHandler {
    // === Existing callbacks (enhanced) ===
    
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("📞 Incoming call from {} to {}", call.from, call.to);
        
        // Update statistics
        {
            let mut stats = self.stats.lock().await;
            stats.calls_received += 1;
            
            // Check limits
            if !self.auto_accept {
                stats.calls_rejected += 1;
                return CallDecision::Reject("Not accepting calls".to_string());
            }
            
            if stats.calls_active >= self.max_calls {
                stats.calls_rejected += 1;
                return CallDecision::Reject("Maximum calls reached".to_string());
            }
            
            stats.calls_active += 1;
            stats.calls_accepted += 1;
        }
        
        // Handle SDP offer/answer using the new clean API
        if let Some(sdp_offer) = &call.sdp {
            let coordinator = self.coordinator.read().await;
            if let Some(coord) = coordinator.as_ref() {
                info!("Generating SDP answer using clean API...");
                
                // Use the new generate_sdp_answer method - no internal access needed!
                match MediaControl::generate_sdp_answer(coord, &call.id, sdp_offer).await {
                    Ok(sdp_answer) => {
                        info!("✅ Generated SDP answer successfully");
                        return CallDecision::Accept(Some(sdp_answer));
                    }
                    Err(e) => {
                        error!("Failed to generate SDP answer: {}", e);
                        // Fall through to accept without SDP
                    }
                }
            }
        }
        
        // Accept without SDP if no offer or generation failed
        CallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, session: CallSession, _local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("✅ Call {} established", session.id());
        
        let coordinator = self.coordinator.read().await;
        if let Some(coord) = coordinator.as_ref() {
            // Parse remote SDP using the new helper function
            if let Some(sdp) = remote_sdp {
                match parse_sdp_connection(&sdp) {
                    Ok(sdp_info) => {
                        let remote_addr = format!("{}:{}", sdp_info.ip, sdp_info.port);
                        info!("📡 Establishing media flow to {}", remote_addr);
                        info!("🎵 Supported codecs: {:?}", sdp_info.codecs);
                        
                        // Use MediaControl API method - clean and simple!
                        match MediaControl::establish_media_flow(coord, session.id(), &remote_addr).await {
                            Ok(_) => {
                                info!("✅ Media flow established successfully");
                                
                                // Start periodic monitoring with the new convenience method
                                let session_id = session.id().clone();
                                let coord_clone = coord.clone();
                                let stats_clone = self.stats.clone();
                                
                                tokio::spawn(async move {
                                    let mut interval = tokio::time::interval(Duration::from_secs(10));
                                    
                                    loop {
                                        interval.tick().await;
                                        
                                        // Use the new get_call_statistics method for comprehensive data
                                        match MediaControl::get_call_statistics(&coord_clone, &session_id).await {
                                            Ok(Some(call_stats)) => {
                                                log_call_statistics(&session_id, &call_stats).await;
                                                
                                                // Track quality issues
                                                if !call_stats.quality.is_acceptable {
                                                    let mut stats = stats_clone.lock().await;
                                                    stats.quality_alerts += 1;
                                                }
                                            }
                                            Ok(None) => break, // Session ended
                                            Err(_) => break,   // Error occurred
                                        }
                                    }
                                });
                            }
                            Err(e) => error!("Failed to establish media flow: {}", e),
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse SDP: {}", e);
                    }
                }
            }
        }
    }
    
    async fn on_call_ended(&self, session: CallSession, reason: &str) {
        info!("📴 Call {} ended: {}", session.id(), reason);
        
        // Update statistics
        let mut stats = self.stats.lock().await;
        if stats.calls_active > 0 {
            stats.calls_active -= 1;
        }
        if let Some(started_at) = session.started_at {
            stats.total_duration += started_at.elapsed();
        }
        
        // Log final statistics using the API
        let coordinator = self.coordinator.read().await;
        if let Some(coord) = coordinator.as_ref() {
            if let Ok(Some(final_stats)) = MediaControl::get_call_statistics(coord, session.id()).await {
                info!("📊 Final call statistics:");
                info!("  Duration: {:?}", final_stats.duration.unwrap_or_default());
                info!("  Packets - Sent: {}, Received: {}, Lost: {}", 
                    final_stats.rtp.packets_sent, 
                    final_stats.rtp.packets_received, 
                    final_stats.rtp.packets_lost);
                info!("  Quality - MOS: {:.1}, Loss: {:.1}%", 
                    final_stats.quality.mos_score, 
                    final_stats.quality.packet_loss_rate);
            }
        }
    }
    
    // === New extended callbacks from refactor ===
    
    async fn on_call_state_changed(&self, session_id: &SessionId, old_state: &CallState, new_state: &CallState, reason: Option<&str>) {
        info!("🔄 Call {} state: {:?} → {:?} ({})", 
            session_id, old_state, new_state, reason.unwrap_or("no reason"));
        
        let mut stats = self.stats.lock().await;
        stats.state_changes += 1;
        
        // Log important state transitions
        match new_state {
            CallState::Ringing => info!("🔔 Call {} is ringing", session_id),
            CallState::Active => info!("📞 Call {} is now active", session_id),
            CallState::OnHold => info!("⏸️  Call {} is on hold", session_id),
            CallState::Failed(reason) => error!("❌ Call {} failed: {:?}", session_id, reason),
            _ => {}
        }
    }
    
    async fn on_media_quality(&self, session_id: &SessionId, mos_score: f32, packet_loss: f32, alert_level: MediaQualityAlertLevel) {
        let emoji = match alert_level {
            MediaQualityAlertLevel::Good => "🟢",
            MediaQualityAlertLevel::Fair => "🟡",
            MediaQualityAlertLevel::Poor => "🟠",
            MediaQualityAlertLevel::Critical => "🔴",
        };
        
        info!("{} Call {} quality - MOS: {:.1}, Loss: {:.1}%, Level: {:?}", 
            emoji, session_id, mos_score, packet_loss, alert_level);
        
        // Track significant quality issues
        if matches!(alert_level, MediaQualityAlertLevel::Poor | MediaQualityAlertLevel::Critical) {
            let mut stats = self.stats.lock().await;
            stats.quality_alerts += 1;
            
            // Log more details for poor quality
            warn!("⚠️  Poor quality detected on call {}: Consider network optimization", session_id);
        }
    }
    
    async fn on_dtmf(&self, session_id: &SessionId, digit: char, duration_ms: u32) {
        info!("☎️  Call {} received DTMF '{}' for {}ms", session_id, digit, duration_ms);
        
        let mut stats = self.stats.lock().await;
        stats.dtmf_received += 1;
        
        // Could implement DTMF-based features here (IVR, transfers, etc.)
        match digit {
            '#' => info!("  → Hash key detected - could trigger special action"),
            '*' => info!("  → Star key detected - could open menu"),
            _ => {}
        }
    }
    
    async fn on_media_flow(&self, session_id: &SessionId, direction: MediaFlowDirection, active: bool, codec: &str) {
        let status = if active { "started" } else { "stopped" };
        let arrow = match direction {
            MediaFlowDirection::Send => "→",
            MediaFlowDirection::Receive => "←",
            MediaFlowDirection::Both => "↔",
        };
        
        info!("🎵 Call {} media {} {} {} using {}", 
            session_id, arrow, direction_str(direction), status, codec);
        
        let mut stats = self.stats.lock().await;
        stats.media_flow_events += 1;
    }
    
    async fn on_warning(&self, session_id: Option<&SessionId>, category: WarningCategory, message: &str) {
        let session_str = session_id.map(|s| format!("Call {}", s))
            .unwrap_or_else(|| "Server".to_string());
        
        warn!("⚠️  {} warning ({:?}): {}", session_str, category, message);
        
        let mut stats = self.stats.lock().await;
        stats.warnings_received += 1;
        
        // Take action based on warning category
        match category {
            WarningCategory::Resource => {
                warn!("  → Consider increasing server resources");
            }
            WarningCategory::Network => {
                warn!("  → Check network connectivity and firewall rules");
            }
            _ => {}
        }
    }
}

// Helper function to format media direction
fn direction_str(direction: MediaFlowDirection) -> &'static str {
    match direction {
        MediaFlowDirection::Send => "send",
        MediaFlowDirection::Receive => "receive",
        MediaFlowDirection::Both => "bidirectional",
    }
}

// Helper function to log comprehensive call statistics
async fn log_call_statistics(session_id: &SessionId, stats: &CallStatistics) {
    info!("\n📊 Call {} Statistics:", session_id);
    info!("  State: {:?}", stats.state);
    info!("  Duration: {:?}", stats.duration.unwrap_or_default());
    
    // Media information
    info!("  Media:");
    info!("    Codec: {}", stats.media.codec.as_deref().unwrap_or("unknown"));
    info!("    Local: {}", stats.media.local_addr.as_deref().unwrap_or("not set"));
    info!("    Remote: {}", stats.media.remote_addr.as_deref().unwrap_or("not set"));
    info!("    Flowing: {}", if stats.media.media_flowing { "yes" } else { "no" });
    
    // RTP statistics
    info!("  RTP:");
    info!("    Sent: {} packets, {} bytes", stats.rtp.packets_sent, stats.rtp.bytes_sent);
    info!("    Received: {} packets, {} bytes", stats.rtp.packets_received, stats.rtp.bytes_received);
    info!("    Lost: {} packets", stats.rtp.packets_lost);
    info!("    Out of order: {}", stats.rtp.packets_out_of_order);
    info!("    Jitter buffer: {:.1}ms", stats.rtp.jitter_buffer_ms);
    info!("    Bitrate: {} kbps", stats.rtp.current_bitrate_kbps);
    
    // Quality metrics
    let quality_emoji = if stats.quality.is_acceptable { "✅" } else { "⚠️" };
    info!("  Quality {}:", quality_emoji);
    info!("    MOS Score: {:.1}/5.0", stats.quality.mos_score);
    info!("    Packet Loss: {:.1}%", stats.quality.packet_loss_rate);
    info!("    Jitter: {:.1}ms", stats.quality.jitter_ms);
    info!("    RTT: {:.0}ms", stats.quality.round_trip_ms);
    info!("    Network Effectiveness: {:.1}%", stats.quality.network_effectiveness * 100.0);
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize logging
    let log_level = match args.log_level.to_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    };
    
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .init();
    
    info!("🚀 Starting Clean UAS Server on port {}", args.port);
    info!("📋 Configuration:");
    info!("  Auto-accept: {}", args.auto_accept);
    info!("  Max calls: {}", args.max_calls);
    
    // Create handler
    let handler = Arc::new(CleanUasHandler::new(args.auto_accept, args.max_calls));
    
    // Build session coordinator using the clean API
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(args.port)
        .with_local_address(format!("sip:uas@127.0.0.1:{}", args.port))
        .with_media_ports(15000, 20000)  // Dynamic port allocation
        .with_handler(handler.clone())
        .build()
        .await?;
    
    // Store coordinator reference in handler
    handler.set_coordinator(coordinator.clone()).await;
    
    // Start the server using SessionControl trait
    SessionControl::start(&coordinator).await?;
    
    info!("✅ Enhanced UAS Server ready and listening!");
    info!("📡 This server demonstrates new API features:");
    info!("  - Extended CallHandler event callbacks");
    info!("  - Comprehensive call statistics");
    info!("  - Automatic quality monitoring");
    info!("  - Rich event handling for all aspects");
    
    // Run until interrupted
    tokio::signal::ctrl_c().await?;
    
    info!("🛑 Shutting down...");
    
    // Stop the server
    SessionControl::stop(&coordinator).await?;
    
    // Print final statistics
    let handler_stats = handler.stats.lock().await;
    info!("📊 Final Server Statistics:");
    info!("  === Call Metrics ===");
    info!("  Total calls received: {}", handler_stats.calls_received);
    info!("  Calls accepted: {}", handler_stats.calls_accepted);
    info!("  Calls rejected: {}", handler_stats.calls_rejected);
    info!("  Total call duration: {:?}", handler_stats.total_duration);
    
    info!("  === Event Metrics ===");
    info!("  State changes: {}", handler_stats.state_changes);
    info!("  Media flow events: {}", handler_stats.media_flow_events);
    info!("  DTMF digits received: {}", handler_stats.dtmf_received);
    info!("  Warnings received: {}", handler_stats.warnings_received);
    info!("  Quality alerts: {}", handler_stats.quality_alerts);
    
    if handler_stats.calls_accepted > 0 {
        let avg_duration = handler_stats.total_duration.as_secs() / handler_stats.calls_accepted as u64;
        info!("  === Averages ===");
        info!("  Average call duration: {} seconds", avg_duration);
        info!("  State changes per call: {:.1}", 
            handler_stats.state_changes as f32 / handler_stats.calls_accepted as f32);
        
        if handler_stats.quality_alerts > 0 {
            let alert_rate = (handler_stats.quality_alerts as f32 / handler_stats.calls_accepted as f32) * 100.0;
            info!("  Quality alert rate: {:.1}%", alert_rate);
        }
    }
    
    info!("👋 Enhanced UAS Server shutdown complete");
    
    Ok(())
} 