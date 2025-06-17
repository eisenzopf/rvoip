//! UAS Server Example
//! 
//! This example demonstrates a simple SIP User Agent Server (UAS)
//! that accepts incoming calls from UAC clients.
//!
//! NOTE: This example uses some internal implementation details for demonstration.
//! For the recommended approach using only the public API, see:
//! `examples/api_best_practices/uas_server_clean.rs`

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::{
    SessionCoordinator,
    MediaControl,
    api::{
        CallHandler, CallSession, CallState, IncomingCall, CallDecision,
        SessionManagerConfig,
    },
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{info, error, warn};
use tracing_subscriber;

#[derive(Parser, Debug)]
#[command(author, version, about = "SIP UAS Server - Auto-answers incoming calls", long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "5062")]
    port: u16,
    
    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,
    
    /// Auto-shutdown after N seconds (0 = disabled)
    #[arg(short, long, default_value = "0")]
    auto_shutdown: u64,
}

/// Statistics for the UAS server
#[derive(Debug, Default)]
struct UasStats {
    calls_received: usize,
    calls_accepted: usize,
    calls_rejected: usize,
    calls_active: usize,
    total_duration: Duration,
}

/// UAS server handler
#[derive(Debug)]
struct UasHandler {
    stats: Arc<Mutex<UasStats>>,
    auto_accept: bool,
    max_calls: usize,
    session_coordinator: Arc<tokio::sync::RwLock<Option<Arc<SessionCoordinator>>>>,
}

impl UasHandler {
    fn new(stats: Arc<Mutex<UasStats>>, auto_accept: bool, max_calls: usize) -> Self {
        Self { 
            stats, 
            auto_accept,
            max_calls,
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    async fn set_coordinator(&self, coordinator: Arc<SessionCoordinator>) {
        let mut coord = self.session_coordinator.write().await;
        *coord = Some(coordinator);
    }
}

#[async_trait::async_trait]
impl CallHandler for UasHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("Incoming call from {} to {}", call.from, call.to);
        
        let mut stats = self.stats.lock().await;
        stats.calls_received += 1;
        
        // Check if we should accept the call
        if !self.auto_accept {
            stats.calls_rejected += 1;
            return CallDecision::Reject("Manual mode - rejecting call".to_string());
        }
        
        if stats.calls_active >= self.max_calls {
            stats.calls_rejected += 1;
            return CallDecision::Reject("Maximum concurrent calls reached".to_string());
        }
        
        stats.calls_accepted += 1;
        stats.calls_active += 1;
        drop(stats);
        
        // If we have a coordinator and the incoming call has SDP, prepare our answer
        let coordinator_guard = self.session_coordinator.read().await;
        if let (Some(coordinator), Some(remote_sdp)) = (coordinator_guard.as_ref(), &call.sdp) {
            info!("Incoming call has SDP offer, preparing answer...");
            
            // Create media session for this call
            match coordinator.media_manager.create_media_session(&call.id).await {
                Ok(_) => {
                    // Generate SDP answer with our allocated port
                    match coordinator.generate_sdp_offer(&call.id).await {
                        Ok(sdp_answer) => {
                            info!("Generated SDP answer with dynamic port allocation");
                            
                            // Update media session with remote SDP
                            if let Err(e) = coordinator.media_manager.update_media_session(&call.id, remote_sdp).await {
                                error!("Failed to update media session with remote SDP: {}", e);
                            }
                            
                            // Update stats
                            {
                                let mut stats = self.stats.lock().await;
                                stats.calls_accepted += 1;
                            }
                            
                            return CallDecision::Accept(Some(sdp_answer));
                        }
                        Err(e) => {
                            error!("Failed to generate SDP answer: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to create media session: {}", e);
                }
            }
        }
        
        // Accept without SDP if we couldn't generate it
        CallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, session: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("Call {} established", session.id);
        info!("Local SDP: {:?}", local_sdp.as_ref().map(|s| s.len()));
        info!("Remote SDP: {:?}", remote_sdp.as_ref().map(|s| s.len()));
        
        // Extract remote RTP address from SDP if available
        let coordinator_guard = self.session_coordinator.read().await;
        if let (Some(coordinator), Some(remote_sdp)) = (coordinator_guard.as_ref(), remote_sdp) {
            info!("Have coordinator and remote SDP, parsing...");
            
            // Simple SDP parsing to get IP and port
            let mut remote_ip = None;
            let mut remote_port = None;
            
            for line in remote_sdp.lines() {
                if line.starts_with("c=IN IP4 ") {
                    remote_ip = line.strip_prefix("c=IN IP4 ").map(|s| s.to_string());
                    info!("Found IP in SDP: {:?}", remote_ip);
                } else if line.starts_with("m=audio ") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() > 1 {
                        remote_port = parts[1].parse::<u16>().ok();
                        info!("Found port in SDP: {:?}", remote_port);
                    }
                }
            }
            
            if let (Some(ip), Some(port)) = (remote_ip, remote_port) {
                let remote_addr = format!("{}:{}", ip, port);
                info!("Establishing media flow to {}", remote_addr);
                
                // Establish media flow (this also starts audio transmission)
                match coordinator.establish_media_flow(&session.id, &remote_addr).await {
                    Ok(_) => {
                        info!("✅ Successfully established media flow for session {}", session.id);
                        info!("✅ Audio transmission (440Hz sine wave) started automatically");
                        
                        // Start statistics monitoring for this specific session
                        let stats_session_id = session.id.clone();
                        let stats_coordinator = coordinator.clone();
                        
                        // Start statistics monitoring task
                        match stats_coordinator.start_statistics_monitoring(&stats_session_id, Duration::from_secs(2)).await {
                            Ok(_) => {
                                info!("✅ Started statistics monitoring for session {}", stats_session_id);
                                
                                // Also spawn a task to periodically fetch and log statistics
                                tokio::spawn(async move {
                                    let mut interval = tokio::time::interval(Duration::from_secs(3));
                                    let mut update_count = 0;
                                    
                                    loop {
                                        interval.tick().await;
                                        update_count += 1;
                                        
                                        match stats_coordinator.get_media_statistics(&stats_session_id).await {
                                            Ok(Some(stats)) => {
                                                info!("📊 Statistics Update #{} for session {}", update_count, stats_session_id);
                                                
                                                if let Some(rtp) = &stats.rtp_stats {
                                                    info!("  RTP - Sent: {} pkts ({} bytes), Recv: {} pkts ({} bytes)",
                                                          rtp.packets_sent, rtp.bytes_sent, 
                                                          rtp.packets_received, rtp.bytes_received);
                                                    info!("  RTP - Lost: {} pkts, Jitter: {:.1}ms",
                                                          rtp.packets_lost, rtp.jitter_ms);
                                                }
                                                
                                                if let Some(quality) = &stats.quality_metrics {
                                                    info!("  Quality - Loss: {:.1}%, MOS: {:.1}, Network: {}%",
                                                          quality.packet_loss_percent, 
                                                          quality.mos_score.unwrap_or(0.0),
                                                          quality.network_quality);
                                                }
                                                
                                                // Alert on quality degradation
                                                if let Some(quality) = &stats.quality_metrics {
                                                    if quality.packet_loss_percent > 5.0 || quality.jitter_ms > 50.0 {
                                                        warn!("⚠️ Quality degradation detected for call {}: Loss: {:.1}%, Jitter: {:.1}ms",
                                                              stats_session_id, quality.packet_loss_percent, quality.jitter_ms);
                                                    }
                                                }
                                            }
                                            Ok(None) => {
                                                info!("No statistics available yet for session {}", stats_session_id);
                                                // Session might have ended, break the loop
                                                break;
                                            }
                                            Err(e) => {
                                                error!("Failed to get statistics for session {}: {}", stats_session_id, e);
                                                // Session might have ended, break the loop
                                                break;
                                            }
                                        }
                                    }
                                    
                                    info!("Statistics monitoring task ended for session {}", stats_session_id);
                                });
                            }
                            Err(e) => {
                                error!("Failed to start statistics monitoring: {}", e);
                            }
                        }
                    }
                    Err(e) => error!("Failed to establish media flow: {}", e),
                }
            } else {
                warn!("Could not extract IP/port from remote SDP");
            }
        } else {
            warn!("No coordinator or remote SDP available");
        }
    }
    
    async fn on_call_ended(&self, session: CallSession, reason: &str) {
        info!("Call {} ended: {}", session.id, reason);
        
        let mut stats = self.stats.lock().await;
        if stats.calls_active > 0 {
            stats.calls_active -= 1;
        }
        
        if let Some(started_at) = session.started_at {
            stats.total_duration += started_at.elapsed();
        }
        
        // Get final statistics for the call
        let coordinator_guard = self.session_coordinator.read().await;
        if let Some(coordinator) = coordinator_guard.as_ref() {
            // Get final media statistics
            match coordinator.get_media_statistics(&session.id).await {
                Ok(Some(final_stats)) => {
                    info!("📊 Final call statistics for session {}:", session.id);
                    
                    if let Some(rtp) = &final_stats.rtp_stats {
                        info!("  Total packets sent: {}", rtp.packets_sent);
                        info!("  Total packets received: {}", rtp.packets_received);
                        info!("  Total bytes sent: {}", rtp.bytes_sent);
                        info!("  Total bytes received: {}", rtp.bytes_received);
                        info!("  Packets lost: {}", rtp.packets_lost);
                        info!("  Final jitter: {:.1}ms", rtp.jitter_ms);
                    }
                    
                    if let Some(quality) = &final_stats.quality_metrics {
                        info!("  Final packet loss: {:.1}%", quality.packet_loss_percent);
                        info!("  Final MOS score: {:.1}", quality.mos_score.unwrap_or(0.0));
                        info!("  Final network quality: {}%", quality.network_quality);
                    }
                    
                    info!("  Call duration: {:?}", final_stats.session_duration);
                }
                Ok(None) => {
                    info!("No final statistics available for session {}", session.id);
                }
                Err(e) => {
                    error!("Failed to get final statistics: {}", e);
                }
            }
            
            // Check if audio was transmitted
            match coordinator.is_audio_transmission_active(&session.id).await {
                Ok(was_active) => {
                    if was_active {
                        info!("✅ Audio transmission was active during the call");
                    } else {
                        info!("ℹ️ Audio transmission status at call end: inactive");
                    }
                }
                Err(e) => {
                    error!("Failed to check audio transmission status: {}", e);
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Initialize logging with the specified level
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
    
    info!("Starting UAS Server on port {}", args.port);
    
    // Create the call handler
    let handler = Arc::new(UasHandler::new(
        Arc::new(Mutex::new(UasStats::default())),
        true,
        10,
    ));
    
    // Create session configuration
    let config = SessionManagerConfig {
        sip_port: args.port,
        local_address: format!("sip:uas@127.0.0.1:{}", args.port),
        media_port_start: 15000,
        media_port_end: 20000,
        ..Default::default()
    };
    
    // Create the session coordinator
    let coordinator = SessionCoordinator::new(config, Some(handler.clone())).await?;
    
    // Set the coordinator in the handler
    handler.set_coordinator(coordinator.clone()).await;
    
    info!("UAS Server ready and listening on port {}", args.port);
    info!("📊 RTP/RTCP statistics monitoring is enabled (updates every 3 seconds during calls)");
    
    // If auto-shutdown is enabled, set up a timer
    if args.auto_shutdown > 0 {
        let shutdown_duration = Duration::from_secs(args.auto_shutdown);
        tokio::spawn(async move {
            sleep(shutdown_duration).await;
            info!("Auto-shutdown timer expired, shutting down...");
            std::process::exit(0);
        });
    }
    
    // Keep the server running
    loop {
        sleep(Duration::from_secs(10)).await;
        
        // Log server statistics periodically
        let stats = handler.stats.lock().await;
        if stats.calls_received > 0 || stats.calls_active > 0 {
            info!(
                "Server stats - Calls received: {}, Active: {}, Accepted: {}, Rejected: {}",
                stats.calls_received, stats.calls_active, stats.calls_accepted, stats.calls_rejected
            );
        }
    }
} 