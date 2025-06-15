//! UAC Client Example
//! 
//! This example demonstrates a simple SIP User Agent Client (UAC)
//! that makes calls to a UAS server.

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::api::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::sync::Mutex;
use tracing::{info, error, warn};
use tracing_subscriber;

#[derive(Parser, Debug)]
#[command(name = "uac_client")]
#[command(about = "SIP UAC Client Example")]
pub struct Args {
    /// SIP listening port for the client
    #[arg(short, long, default_value = "5061")]
    pub port: u16,
    
    /// Target SIP server address (IP:port)
    #[arg(short, long, default_value = "127.0.0.1:5062")]
    pub target: String,
    
    /// Number of calls to make
    #[arg(short, long, default_value = "1")]
    pub calls: usize,
    
    /// Duration of each call in seconds
    #[arg(short, long, default_value = "30")]
    pub duration: u64,
    
    /// Delay between calls in seconds
    #[arg(short = 'w', long, default_value = "2")]
    pub delay: u64,
    
    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    pub log_level: String,
}

/// Statistics for the UAC client
#[derive(Debug, Default)]
struct UacStats {
    calls_initiated: usize,
    calls_connected: usize,
    calls_failed: usize,
    total_duration: Duration,
}

/// UAC client handler
#[derive(Debug)]
struct UacHandler {
    stats: Arc<Mutex<UacStats>>,
    session_coordinator: Arc<tokio::sync::RwLock<Option<Arc<SessionCoordinator>>>>,
}

impl UacHandler {
    fn new(stats: Arc<Mutex<UacStats>>) -> Self {
        Self { 
            stats,
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    async fn set_coordinator(&self, coordinator: Arc<SessionCoordinator>) {
        let mut coord = self.session_coordinator.write().await;
        *coord = Some(coordinator);
    }
}

#[async_trait::async_trait]
impl CallHandler for UacHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        warn!("UAC received unexpected incoming call from {}", call.from);
        CallDecision::Reject("UAC does not accept incoming calls".to_string())
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
                    }
                    Err(e) => error!("Failed to establish media flow: {}", e),
                }
            } else {
                warn!("Could not extract IP/port from remote SDP");
            }
        } else {
            warn!("No coordinator or remote SDP available");
        }
        
        let mut stats = self.stats.lock().await;
        stats.calls_connected += 1;
    }
    
    async fn on_call_ended(&self, session: CallSession, reason: &str) {
        info!("Call {} ended: {}", session.id, reason);
        
        let mut stats = self.stats.lock().await;
        if let Some(started_at) = session.started_at {
            stats.total_duration += started_at.elapsed();
        }
        
        // Check if audio was transmitted
        let coordinator_guard = self.session_coordinator.read().await;
        if let Some(coordinator) = coordinator_guard.as_ref() {
            match coordinator.is_audio_transmission_active(&session.id).await {
                Ok(true) => {
                    info!("✅ Audio transmission was active during the call");
                }
                Ok(false) => {
                    warn!("⚠️ Audio transmission was NOT active during the call");
                }
                Err(e) => {
                    error!("Failed to check audio transmission status: {}", e);
                }
            }
        }
    }
}

async fn make_test_calls(
    session_coordinator: Arc<SessionCoordinator>,
    target: String,
    num_calls: usize,
    call_duration: Duration,
    call_delay: Duration,
    stats: Arc<Mutex<UacStats>>,
) -> Result<()> {
    info!("Starting {} test calls to {}", num_calls, target);
    
    for i in 0..num_calls {
        info!("Making call {} of {}", i + 1, num_calls);
        
        // Update stats
        {
            let mut s = stats.lock().await;
            s.calls_initiated += 1;
        }
        
        // Prepare the call (allocates media port and generates SDP)
        let from_uri = "sip:uac@127.0.0.1";
        let to_uri = format!("sip:uas@{}", target);
        
        match session_coordinator.prepare_outgoing_call(from_uri, &to_uri).await {
            Ok(prepared_call) => {
                info!("Prepared call with session {} on RTP port {}", 
                    prepared_call.session_id, prepared_call.local_rtp_port);
                
                // Now initiate the call with the prepared SDP
                match session_coordinator.initiate_prepared_call(&prepared_call).await {
                    Ok(session) => {
                        info!("Call {} initiated successfully", session.id);
                        
                        // Wait for call duration
                        info!("Call active for {} seconds...", call_duration.as_secs());
                        tokio::time::sleep(call_duration).await;
                        
                        // Terminate the call
                        info!("Terminating call {}", session.id);
                        if let Err(e) = session_coordinator.terminate_session(&session.id).await {
                            error!("Failed to terminate call: {}", e);
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
            info!("Waiting {} seconds before next call...", call_delay.as_secs());
            tokio::time::sleep(call_delay).await;
        }
    }
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments first
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
        .with_target(false)
        .init();
    
    info!("Starting UAC client on port {}", args.port);
    
    // Create stats tracker
    let stats = Arc::new(Mutex::new(UacStats::default()));
    
    // Create handler
    let handler = Arc::new(UacHandler::new(stats.clone()));
    
    // Create session manager with dynamic port allocation
    let session_coordinator = SessionManagerBuilder::new()
        .with_sip_port(args.port)
        .with_local_address(format!("sip:uac@127.0.0.1:{}", args.port))
        .with_media_ports(10000, 15000)  // UAC uses ports 10000-15000
        .with_handler(handler.clone())
        .build()
        .await?;
    
    // Set the coordinator in the handler
    handler.set_coordinator(session_coordinator.clone()).await;
    
    // Start the session manager
    session_coordinator.start().await?;
    
    // Give it a moment to bind
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Make test calls
    let calls_handle = tokio::spawn(make_test_calls(
        session_coordinator.clone(),
        args.target,
        args.calls,
        Duration::from_secs(args.duration),
        Duration::from_secs(args.delay),
        stats.clone(),
    ));
    
    // Wait for calls to complete OR Ctrl+C
    tokio::select! {
        result = calls_handle => {
            match result {
                Ok(Ok(())) => info!("All calls completed successfully"),
                Ok(Err(e)) => error!("Call task error: {}", e),
                Err(e) => error!("Call task panicked: {}", e),
            }
        }
        _ = signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
        }
    }
    
    // Give a moment for any pending operations
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Stop the session manager
    session_coordinator.stop().await?;
    
    // Print final statistics
    let final_stats = stats.lock().await;
    info!("=== Final Statistics ===");
    info!("Calls initiated: {}", final_stats.calls_initiated);
    info!("Calls connected: {}", final_stats.calls_connected);
    info!("Calls failed: {}", final_stats.calls_failed);
    info!("Total call duration: {:?}", final_stats.total_duration);
    
    info!("UAC client shutdown complete");
    
    Ok(())
} 