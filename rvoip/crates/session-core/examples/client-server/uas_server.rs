//! UAS Server Example
//! 
//! This example demonstrates a simple SIP User Agent Server (UAS)
//! that accepts incoming calls from UAC clients.

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
#[command(name = "uas_server")]
#[command(about = "SIP UAS Server Example")]
pub struct Args {
    /// SIP listening port for the server
    #[arg(short, long, default_value = "5062")]
    pub port: u16,
    
    /// Auto-accept incoming calls
    #[arg(short, long, default_value = "true")]
    pub auto_accept: bool,
    
    /// Maximum concurrent calls
    #[arg(short, long, default_value = "10")]
    pub max_calls: usize,
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
        
        // Extract remote RTP address from SDP if available
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
                info!("Establishing media flow to {}", remote_addr);
                
                // Start audio transmission
                if let Err(e) = coordinator.establish_media_flow(&session.id, &remote_addr).await {
                    error!("Failed to establish media flow: {}", e);
                }
            }
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
        
        // Check if audio was transmitted
        let coordinator_guard = self.session_coordinator.read().await;
        if let Some(coordinator) = coordinator_guard.as_ref() {
            match coordinator.media_manager.get_media_info(&session.id).await {
                Ok(Some(info)) => {
                    if info.local_rtp_port.is_some() && info.remote_rtp_port.is_some() {
                        info!("✅ Audio transmission was active during the call");
                    }
                }
                _ => {}
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();
    
    // Parse command line arguments
    let args = Args::parse();
    
    info!("Starting UAS server on port {}", args.port);
    info!("Auto-accept: {}", args.auto_accept);
    info!("Max concurrent calls: {}", args.max_calls);
    
    // Create stats tracker
    let stats = Arc::new(Mutex::new(UasStats::default()));
    
    // Create handler
    let handler = Arc::new(UasHandler::new(
        stats.clone(),
        args.auto_accept,
        args.max_calls,
    ));
    
    // Create session manager with dynamic port allocation
    let session_coordinator = SessionManagerBuilder::new()
        .with_sip_port(args.port)
        .with_local_address(format!("sip:uas@127.0.0.1:{}", args.port))
        .with_media_ports(15000, 20000)  // UAS uses ports 15000-20000
        .with_handler(handler.clone())
        .build()
        .await?;
    
    // Set the coordinator in the handler
    handler.set_coordinator(session_coordinator.clone()).await;
    
    // Start the session manager
    session_coordinator.start().await?;
    
    info!("UAS server listening on {}", session_coordinator.get_bound_address());
    info!("Media ports: 15000-20000 (dynamically allocated)");
    info!("Ready to accept calls...");
    
    // Wait for Ctrl+C
    signal::ctrl_c().await?;
    info!("Shutting down UAS server...");
    
    // Stop the session manager
    session_coordinator.stop().await?;
    
    // Print final statistics
    let final_stats = stats.lock().await;
    info!("=== Final Statistics ===");
    info!("Calls received: {}", final_stats.calls_received);
    info!("Calls accepted: {}", final_stats.calls_accepted);
    info!("Calls rejected: {}", final_stats.calls_rejected);
    info!("Total call duration: {:?}", final_stats.total_duration);
    
    Ok(())
} 