//! Clean UAS Server Example - Best Practices
//! 
//! This example demonstrates the recommended way to build a SIP User Agent Server (UAS)
//! using only the public session-core API. No internal implementation details are accessed.
//!
//! Key patterns demonstrated:
//! - Using only `api::*` imports
//! - Leveraging MediaControl trait methods instead of direct access
//! - Proper SDP handling with the new API methods
//! - Clean separation of concerns

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::api::*;  // Single, clean import - everything we need
use std::sync::Arc;
use std::time::{Duration, Instant};
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

/// Statistics tracking for the UAS
#[derive(Debug, Default)]
struct UasStats {
    calls_received: usize,
    calls_accepted: usize,
    calls_rejected: usize,
    calls_active: usize,
    total_duration: Duration,
}

/// Clean UAS handler using only the public API
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
                                
                                // Start monitoring statistics
                                if let Err(e) = MediaControl::start_statistics_monitoring(
                                    coord, 
                                    session.id(), 
                                    Duration::from_secs(5)
                                ).await {
                                    warn!("Failed to start statistics monitoring: {}", e);
                                }
                            }
                            Err(e) => error!("Failed to establish media flow: {}", e),
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse SDP: {}", e);
                    }
                }
            }
            
            // Monitor call quality using the API
            let session_id = session.id().clone();
            let coord_clone = coord.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                
                loop {
                    interval.tick().await;
                    
                    // Use MediaControl API to get statistics
                    match MediaControl::get_media_statistics(&coord_clone, &session_id).await {
                        Ok(Some(stats)) => {
                            if let Some(quality) = &stats.quality_metrics {
                                info!("📊 Call {} - Loss: {:.1}%, Jitter: {:.1}ms, MOS: {:.1}",
                                    session_id,
                                    quality.packet_loss_percent,
                                    quality.jitter_ms,
                                    quality.mos_score.unwrap_or(0.0)
                                );
                            }
                        }
                        Ok(None) => break, // Session ended
                        Err(_) => break,   // Error or session ended
                    }
                }
            });
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
            if let Ok(Some(final_stats)) = MediaControl::get_media_statistics(coord, session.id()).await {
                info!("📊 Final call statistics:");
                if let Some(rtp) = &final_stats.rtp_stats {
                    info!("  Packets - Sent: {}, Received: {}, Lost: {}", 
                        rtp.packets_sent, rtp.packets_received, rtp.packets_lost);
                }
                if let Some(quality) = &final_stats.quality_metrics {
                    info!("  Quality - Loss: {:.1}%, MOS: {:.1}", 
                        quality.packet_loss_percent, 
                        quality.mos_score.unwrap_or(0.0));
                }
            }
        }
    }
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
    
    info!("✅ Clean UAS Server ready and listening!");
    info!("📡 This server demonstrates best practices:");
    info!("  - Uses only public API (no internal access)");
    info!("  - Clean SDP handling with new API methods");
    info!("  - Proper error handling and logging");
    info!("  - Statistics monitoring via API");
    
    // Run until interrupted
    tokio::signal::ctrl_c().await?;
    
    info!("🛑 Shutting down...");
    
    // Stop the server
    SessionControl::stop(&coordinator).await?;
    
    // Print final statistics
    let handler_stats = handler.stats.lock().await;
    info!("📊 Final Server Statistics:");
    info!("  Total calls received: {}", handler_stats.calls_received);
    info!("  Calls accepted: {}", handler_stats.calls_accepted);
    info!("  Calls rejected: {}", handler_stats.calls_rejected);
    info!("  Total call duration: {:?}", handler_stats.total_duration);
    
    if handler_stats.calls_accepted > 0 {
        let avg_duration = handler_stats.total_duration.as_secs() / handler_stats.calls_accepted as u64;
        info!("  Average call duration: {} seconds", avg_duration);
    }
    
    info!("👋 Clean UAS Server shutdown complete");
    
    Ok(())
} 