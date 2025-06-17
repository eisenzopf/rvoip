//! Clean UAC Client Example - Best Practices
//! 
//! This example demonstrates the recommended way to build a SIP User Agent Client (UAC)
//! using only the public session-core API. This shows that UAC implementations are
//! already clean and follow best practices.
//!
//! Key patterns demonstrated:
//! - Using only `api::*` imports (UAC already does this well)
//! - Leveraging SessionControl and MediaControl traits
//! - Proper call lifecycle management
//! - Clean error handling

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::api::*;  // Single, clean import - everything we need
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{info, error, warn};

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

/// Statistics for the UAC
#[derive(Debug, Default)]
struct UacStats {
    calls_initiated: usize,
    calls_connected: usize,
    calls_failed: usize,
    total_duration: Duration,
}

/// Clean UAC handler using only the public API
#[derive(Debug)]
struct CleanUacHandler {
    stats: Arc<Mutex<UacStats>>,
    // Store coordinator reference for API calls
    coordinator: Arc<tokio::sync::RwLock<Option<Arc<SessionCoordinator>>>>,
}

impl CleanUacHandler {
    fn new() -> Self {
        Self {
            stats: Arc::new(Mutex::new(UacStats::default())),
            coordinator: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    async fn set_coordinator(&self, coordinator: Arc<SessionCoordinator>) {
        *self.coordinator.write().await = Some(coordinator);
    }
}

#[async_trait::async_trait]
impl CallHandler for CleanUacHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // UAC typically doesn't accept incoming calls
        warn!("UAC received unexpected incoming call from {}", call.from);
        CallDecision::Reject("UAC does not accept incoming calls".to_string())
    }
    
    async fn on_call_established(&self, session: CallSession, _local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("✅ Call {} established", session.id());
        
        // Update statistics
        {
            let mut stats = self.stats.lock().await;
            stats.calls_connected += 1;
        }
        
        let coordinator = self.coordinator.read().await;
        if let Some(coord) = coordinator.as_ref() {
            // Parse and establish media flow using the clean API
            if let Some(sdp) = remote_sdp {
                match parse_sdp_connection(&sdp) {
                    Ok(sdp_info) => {
                        let remote_addr = format!("{}:{}", sdp_info.ip, sdp_info.port);
                        info!("📡 Establishing media flow to {}", remote_addr);
                        
                        // Use MediaControl API - clean and consistent!
                        match MediaControl::establish_media_flow(coord, session.id(), &remote_addr).await {
                            Ok(_) => {
                                info!("✅ Media flow established, audio transmission active");
                                
                                // Start monitoring
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
                    Err(e) => error!("Failed to parse SDP: {}", e),
                }
            }
            
            // Monitor call quality
            let session_id = session.id().clone();
            let coord_clone = coord.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    
                    match MediaControl::get_media_statistics(&coord_clone, &session_id).await {
                        Ok(Some(stats)) => {
                            if let Some(rtp) = &stats.rtp_stats {
                                info!("📊 RTP - Sent: {} pkts, Recv: {} pkts, Lost: {}",
                                    rtp.packets_sent, rtp.packets_received, rtp.packets_lost);
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
            });
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
}

/// Make test calls using the clean API
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
        info!("Making call {} of {}", i + 1, num_calls);
        
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
                            Ok(_) => {
                                info!("✅ Call answered successfully");
                                
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
    
    // Store coordinator reference
    handler.set_coordinator(coordinator.clone()).await;
    
    // Start the session manager
    SessionControl::start(&coordinator).await?;
    
    info!("✅ Clean UAC Client ready!");
    info!("📡 This client demonstrates that UAC already follows best practices:");
    info!("  - Uses only public API");
    info!("  - Clean call lifecycle management");
    info!("  - Proper error handling");
    
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
    info!("  Total call duration: {:?}", final_stats.total_duration);
    
    if final_stats.calls_connected > 0 {
        let avg_duration = final_stats.total_duration.as_secs() / final_stats.calls_connected as u64;
        info!("  Average call duration: {} seconds", avg_duration);
    }
    
    info!("👋 Clean UAC Client shutdown complete");
    
    Ok(())
} 