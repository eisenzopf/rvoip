//! Clean UAC Client Example - Best Practices
//! 
//! This example demonstrates the recommended way to build a SIP User Agent Client (UAC)
//! using the session-core API with the new extended event callbacks and improved
//! statistics methods.
//!
//! Key patterns demonstrated:
//! - Using extended CallHandler callbacks for richer event handling
//! - Leveraging new convenience statistics methods
//! - Proper call lifecycle management with detailed state tracking
//! - Media quality monitoring with automatic alerts

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::api::{
    CallHandler, CallDecision, IncomingCall, CallSession, CallState,
    SessionManagerBuilder, SessionControl, MediaControl,
    SessionCoordinator, parse_sdp_connection, SessionId,
    // New imports from the refactor
    MediaQualityAlertLevel, MediaFlowDirection, WarningCategory,
    QualityThresholds,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, error, warn};

/// Enhanced UAC handler demonstrating new API features
#[derive(Debug)]
struct CleanUacHandler {
    stats: Arc<Mutex<UacStats>>,
}

impl CleanUacHandler {
    fn new() -> Self {
        Self {
            stats: Arc::new(Mutex::new(UacStats::default())),
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for CleanUacHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // UAC typically doesn't accept incoming calls
        warn!("UAC received unexpected incoming call from {}", call.from);
        CallDecision::Reject("UAC does not accept incoming calls".to_string())
    }
    
    async fn on_call_established(&self, session: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
        info!("✅ Call {} established", session.id());
        
        // Update statistics
        {
            let mut stats = self.stats.lock().await;
            stats.calls_connected += 1;
        }
    }
    
    async fn on_call_ended(&self, session: CallSession, reason: &str) {
        info!("📴 Call {} ended: {}", session.id(), reason);
        
        // Update statistics
        let mut stats = self.stats.lock().await;
        if let Some(started_at) = session.started_at {
            stats.total_duration += started_at.elapsed();
        }
    }
    
    async fn on_call_state_changed(&self, session_id: &SessionId, old_state: &CallState, new_state: &CallState, reason: Option<&str>) {
        info!("🔄 Call {} state changed: {:?} → {:?} ({})", 
            session_id, old_state, new_state, reason.unwrap_or("no reason"));
        
        // Track specific state transitions
        let mut stats = self.stats.lock().await;
        match (old_state, new_state) {
            (CallState::Initiating, CallState::Ringing) => stats.calls_ringing += 1,
            (_, CallState::Active) => {
                // Already handled in on_call_established
            }
            (_, CallState::Failed(_)) => {
                // Update failed count only if not already counted
                if !matches!(old_state, CallState::Failed(_)) {
                    stats.calls_failed += 1;
                }
            }
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
        
        // Track quality issues
        if matches!(alert_level, MediaQualityAlertLevel::Poor | MediaQualityAlertLevel::Critical) {
            let mut stats = self.stats.lock().await;
            stats.quality_warnings += 1;
        }
    }
    
    async fn on_dtmf(&self, session_id: &SessionId, digit: char, duration_ms: u32) {
        info!("☎️  Call {} DTMF digit '{}' for {}ms", session_id, digit, duration_ms);
    }
    
    async fn on_media_flow(&self, session_id: &SessionId, direction: MediaFlowDirection, active: bool, codec: &str) {
        let status = if active { "started" } else { "stopped" };
        info!("🎵 Call {} media {:?} {} using codec {}", session_id, direction, status, codec);
    }
    
    async fn on_warning(&self, session_id: Option<&SessionId>, category: WarningCategory, message: &str) {
        let session_str = session_id.map(|s| s.to_string()).unwrap_or_else(|| "global".to_string());
        warn!("⚠️  Warning [{}] {:?}: {}", session_str, category, message);
    }
}

/// Make test calls using the enhanced API
async fn make_test_calls(
    coordinator: Arc<SessionCoordinator>,
    target: String,
    num_calls: usize,
    call_duration: Duration,
    call_delay: Duration,
    stats: Arc<Mutex<UacStats>>,
) -> Result<()> {
    info!("📞 Starting {} test calls to {}", num_calls, target);
    
    for i in 0..num_calls {
        info!("\n=== Making call {} of {} ===", i + 1, num_calls);
        
        // Update statistics
        {
            let mut s = stats.lock().await;
            s.calls_initiated += 1;
        }
        
        let from_uri = "sip:uac@127.0.0.1";
        let to_uri = format!("sip:uas@{}", target);
        
        // Use the clean API to prepare and initiate calls
        match SessionControl::prepare_outgoing_call(&coordinator, from_uri, &to_uri).await {
            Ok(prepared_call) => {
                info!("📋 Prepared call {} with RTP port {}", 
                    prepared_call.session_id, prepared_call.local_rtp_port);
                
                // Initiate the call
                match SessionControl::initiate_prepared_call(&coordinator, &prepared_call).await {
                    Ok(session) => {
                        info!("🔄 Call initiated, waiting for answer...");
                        
                        // Wait for the call to be answered
                        match SessionControl::wait_for_answer(
                            &coordinator, 
                            session.id(), 
                            Duration::from_secs(30)
                        ).await {
                            Ok(()) => {
                                info!("✅ Call answered successfully");
                                
                                // Get media info to establish flow
                                if let Ok(Some(media_info)) = MediaControl::get_media_info(&coordinator, session.id()).await {
                                    if let Some(remote_sdp) = media_info.remote_sdp {
                                        match parse_sdp_connection(&remote_sdp) {
                                            Ok(sdp_info) => {
                                                let remote_addr = format!("{}:{}", sdp_info.ip, sdp_info.port);
                                                info!("📡 Establishing media flow to {}", remote_addr);
                                                
                                                // Establish media flow
                                                match MediaControl::establish_media_flow(&coordinator, session.id(), &remote_addr).await {
                                                    Ok(_) => {
                                                        info!("✅ Media flow established");
                                                        
                                                        // Set up quality monitoring with thresholds
                                                        let thresholds = QualityThresholds {
                                                            min_mos: 3.0,
                                                            max_packet_loss: 5.0,
                                                            max_jitter_ms: 50.0,
                                                            check_interval: Duration::from_secs(5),
                                                        };
                                                        
                                                        if let Err(e) = MediaControl::monitor_call_quality(
                                                            &coordinator,
                                                            session.id(),
                                                            thresholds
                                                        ).await {
                                                            warn!("Failed to start quality monitoring: {}", e);
                                                        }
                                                    }
                                                    Err(e) => error!("Failed to establish media flow: {}", e),
                                                }
                                            }
                                            Err(e) => error!("Failed to parse SDP: {}", e),
                                        }
                                    }
                                }
                                
                                // Periodically log comprehensive statistics using new convenience methods
                                let session_id = session.id().clone();
                                let coord_clone = coordinator.clone();
                                let stats_clone = stats.clone();
                                tokio::spawn(async move {
                                    let mut interval = tokio::time::interval(Duration::from_secs(10));
                                    
                                    loop {
                                        interval.tick().await;
                                        
                                        // Use the new get_call_statistics convenience method
                                        match MediaControl::get_call_statistics(&coord_clone, &session_id).await {
                                            Ok(Some(call_stats)) => {
                                                info!("\n📊 Call Statistics Update:");
                                                info!("  Duration: {:?}", call_stats.duration.unwrap_or_default());
                                                info!("  State: {:?}", call_stats.state);
                                                
                                                // Media info
                                                info!("  Media:");
                                                info!("    Codec: {}", call_stats.media.codec.as_deref().unwrap_or("unknown"));
                                                info!("    Flowing: {}", call_stats.media.media_flowing);
                                                
                                                // RTP stats
                                                info!("  RTP:");
                                                info!("    Packets Sent: {}", call_stats.rtp.packets_sent);
                                                info!("    Packets Received: {}", call_stats.rtp.packets_received);
                                                info!("    Packets Lost: {}", call_stats.rtp.packets_lost);
                                                info!("    Bitrate: {} kbps", call_stats.rtp.current_bitrate_kbps);
                                                
                                                // Quality metrics
                                                info!("  Quality:");
                                                info!("    MOS Score: {:.1}", call_stats.quality.mos_score);
                                                info!("    Packet Loss: {:.1}%", call_stats.quality.packet_loss_rate);
                                                info!("    Jitter: {:.1}ms", call_stats.quality.jitter_ms);
                                                info!("    RTT: {:.0}ms", call_stats.quality.round_trip_ms);
                                                info!("    Acceptable: {}", call_stats.quality.is_acceptable);
                                                
                                                // Update stats if quality is poor
                                                if !call_stats.quality.is_acceptable {
                                                    let mut s = stats_clone.lock().await;
                                                    s.quality_warnings += 1;
                                                }
                                            }
                                            Ok(None) => break, // Session ended
                                            Err(_) => break,   // Error occurred
                                        }
                                    }
                                });
                                
                                // Also demonstrate individual convenience methods
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                
                                // Get individual metrics
                                if let Ok(Some(mos)) = MediaControl::get_call_quality_score(&coordinator, session.id()).await {
                                    info!("📈 Current MOS score: {:.1}", mos);
                                }
                                
                                if let Ok(Some(loss)) = MediaControl::get_packet_loss_rate(&coordinator, session.id()).await {
                                    info!("📉 Current packet loss: {:.1}%", loss);
                                }
                                
                                if let Ok(Some(bitrate)) = MediaControl::get_current_bitrate(&coordinator, session.id()).await {
                                    info!("📶 Current bitrate: {} kbps", bitrate);
                                }
                                
                                // Let the call run for the specified duration
                                info!("📞 Call active for {} seconds...", call_duration.as_secs());
                                tokio::time::sleep(call_duration).await;
                                
                                // Terminate the call
                                info!("Terminating call...");
                                if let Err(e) = SessionControl::terminate_session(&coordinator, session.id()).await {
                                    error!("Failed to terminate call: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Call was not answered: {}", e);
                                let mut s = stats.lock().await;
                                s.calls_failed += 1;
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to initiate call: {}", e);
                        let mut s = stats.lock().await;
                        s.calls_failed += 1;
                    }
                }
            }
            Err(e) => {
                error!("Failed to prepare call: {}", e);
                let mut s = stats.lock().await;
                s.calls_failed += 1;
            }
        }
        
        // Delay between calls
        if i < num_calls - 1 {
            info!("⏳ Waiting {} seconds before next call...", call_delay.as_secs());
            tokio::time::sleep(call_delay).await;
        }
    }
    
    Ok(())
}

#[derive(Parser, Debug)]
#[command(name = "uac_client_clean")]
#[command(about = "Clean UAC Client demonstrating API best practices")]
struct Args {
    /// SIP listening port
    #[arg(short, long, default_value = "5061")]
    port: u16,
    
    /// Target SIP server (IP:port)
    #[arg(short, long, default_value = "127.0.0.1:5062")]
    target: String,
    
    /// Number of calls to make
    #[arg(short = 'n', long, default_value = "1")]
    num_calls: usize,
    
    /// Call duration in seconds
    #[arg(short, long, default_value = "30")]
    duration: u64,
    
    /// Delay between calls in seconds
    #[arg(short = 'w', long, default_value = "2")]
    delay: u64,
    
    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,
}

/// Enhanced statistics for the UAC
#[derive(Debug, Default)]
struct UacStats {
    calls_initiated: usize,
    calls_connected: usize,
    calls_failed: usize,
    calls_ringing: usize,
    quality_warnings: usize,
    total_duration: Duration,
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
    
    info!("🚀 Starting Clean UAC Client on port {}", args.port);
    
    // Create handler and stats
    let handler = Arc::new(CleanUacHandler::new());
    let stats = handler.stats.clone();
    
    // Build session coordinator using the clean API
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(args.port)
        .with_local_address(format!("sip:uac@127.0.0.1:{}", args.port))
        .with_media_ports(10000, 15000)  // UAC uses different range than UAS
        .with_handler(handler.clone())
        .build()
        .await?;
    
    // Start the session manager
    SessionControl::start(&coordinator).await?;
    
    info!("✅ Enhanced UAC Client ready!");
    info!("📡 This client demonstrates new API features:");
    info!("  - Extended CallHandler event callbacks");
    info!("  - Convenient statistics methods");
    info!("  - Automatic quality monitoring");
    info!("  - Rich state tracking");
    
    // Give it a moment to fully start
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Make test calls
    let calls_handle = tokio::spawn(make_test_calls(
        coordinator.clone(),
        args.target,
        args.num_calls,
        Duration::from_secs(args.duration),
        Duration::from_secs(args.delay),
        stats.clone(),
    ));
    
    // Wait for completion or Ctrl+C
    tokio::select! {
        result = calls_handle => {
            match result {
                Ok(Ok(())) => info!("✅ All calls completed successfully"),
                Ok(Err(e)) => error!("Call task error: {}", e),
                Err(e) => error!("Call task panicked: {}", e),
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
        }
    }
    
    // Stop the session manager
    SessionControl::stop(&coordinator).await?;
    
    // Print final statistics
    let final_stats = stats.lock().await;
    info!("📊 Final Statistics:");
    info!("  Calls initiated: {}", final_stats.calls_initiated);
    info!("  Calls connected: {}", final_stats.calls_connected);
    info!("  Calls failed: {}", final_stats.calls_failed);
    info!("  Calls ringing: {}", final_stats.calls_ringing);
    info!("  Quality warnings: {}", final_stats.quality_warnings);
    info!("  Total call duration: {:?}", final_stats.total_duration);
    
    if final_stats.calls_connected > 0 {
        let avg_duration = final_stats.total_duration.as_secs() / final_stats.calls_connected as u64;
        info!("  Average call duration: {} seconds", avg_duration);
        
        if final_stats.quality_warnings > 0 {
            let warning_rate = (final_stats.quality_warnings as f32 / final_stats.calls_connected as f32) * 100.0;
            info!("  Quality warning rate: {:.1}%", warning_rate);
        }
    }
    
    info!("👋 Enhanced UAC Client shutdown complete");
    
    Ok(())
} 