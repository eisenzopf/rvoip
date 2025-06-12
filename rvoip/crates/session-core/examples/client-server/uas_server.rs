//! UAS Server Example
//! 
//! This example demonstrates a simple SIP User Agent Server (UAS)
//! that accepts incoming calls from UAC clients.

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::api::*;
use rvoip_session_core::coordinator::SessionCoordinator;
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

/// Statistics tracking for the UAS
#[derive(Debug, Default)]
struct UasStats {
    calls_received: usize,
    calls_accepted: usize,
    calls_rejected: usize,
    calls_completed: usize,
    total_duration: Duration,
}

/// UAS Call Handler
#[derive(Debug)]
struct UasHandler {
    stats: Arc<Mutex<UasStats>>,
    auto_accept: bool,
    max_calls: usize,
}

impl UasHandler {
    fn new(stats: Arc<Mutex<UasStats>>, auto_accept: bool, max_calls: usize) -> Self {
        Self { stats, auto_accept, max_calls }
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
            warn!("Auto-accept disabled, rejecting call");
            stats.calls_rejected += 1;
            return CallDecision::Reject("Auto-accept disabled".to_string());
        }
        
        // Check max concurrent calls
        let active_calls = stats.calls_accepted - stats.calls_completed;
        if active_calls >= self.max_calls {
            warn!("Max concurrent calls reached ({}), rejecting call", self.max_calls);
            stats.calls_rejected += 1;
            return CallDecision::Reject("Server busy".to_string());
        }
        
        // Accept the call
        info!("Accepting call from {}", call.from);
        stats.calls_accepted += 1;
        
        // Generate SDP answer if needed
        if let Some(offer_sdp) = call.sdp {
            match generate_sdp_answer(&offer_sdp, "127.0.0.1", 30000) {
                Ok(answer) => {
                    info!("Generated SDP answer for call");
                    CallDecision::Accept(Some(answer))
                }
                Err(e) => {
                    error!("Failed to generate SDP answer: {}", e);
                    stats.calls_rejected += 1;
                    CallDecision::Reject("Failed to generate media answer".to_string())
                }
            }
        } else {
            CallDecision::Accept(None)
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("Call {} ended: {}", call.id, reason);
        
        let mut stats = self.stats.lock().await;
        stats.calls_completed += 1;
        
        if let Some(started_at) = call.started_at {
            stats.total_duration += started_at.elapsed();
        }
    }
}

async fn run_server(
    session_manager: Arc<SessionCoordinator>,
    stats: Arc<Mutex<UasStats>>,
) -> Result<()> {
    info!("UAS server ready and waiting for calls...");
    
    // Print stats periodically
    let stats_clone = stats.clone();
    let stats_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let s = stats_clone.lock().await;
            info!("=== Server Statistics ===");
            info!("Calls received: {}", s.calls_received);
            info!("Calls accepted: {}", s.calls_accepted);
            info!("Calls rejected: {}", s.calls_rejected);
            info!("Calls completed: {}", s.calls_completed);
            info!("Active calls: {}", s.calls_accepted - s.calls_completed);
            info!("Total call duration: {:?}", s.total_duration);
        }
    });
    
    // Wait for shutdown signal
    signal::ctrl_c().await?;
    info!("Received shutdown signal");
    
    stats_task.abort();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    let args = Args::parse();
    let port = args.port;
    
    info!("Starting UAS server on port {}", port);
    
    // Create stats tracker
    let stats = Arc::new(Mutex::new(UasStats::default()));
    
    // Create session manager
    let session_manager = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_local_address(format!("sip:uas@127.0.0.1:{}", port))
        .with_handler(Arc::new(UasHandler::new(
            stats.clone(),
            args.auto_accept,
            args.max_calls,
        )))
        .build()
        .await?;
    
    info!("UAS server initialized on {}", session_manager.get_bound_address());
    
    // Run the server
    run_server(session_manager.clone(), stats.clone()).await?;
    
    // Print final statistics
    let final_stats = stats.lock().await;
    info!("=== Final Statistics ===");
    info!("Calls received: {}", final_stats.calls_received);
    info!("Calls accepted: {}", final_stats.calls_accepted);
    info!("Calls rejected: {}", final_stats.calls_rejected);
    info!("Calls completed: {}", final_stats.calls_completed);
    info!("Total call duration: {:?}", final_stats.total_duration);
    
    // Shutdown
    session_manager.stop().await?;
    info!("UAS server shutdown complete");
    
    Ok(())
}

/// Generate SDP answer from offer
fn generate_sdp_answer(offer: &str, local_ip: &str, port: u16) -> Result<String> {
    // Simple SDP answer generation - in a real app, you'd parse the offer and respond accordingly
    let answer = format!(
        "v=0\r\n\
         o=uas 123456 654321 IN IP4 {}\r\n\
         s=UAS Test Call\r\n\
         c=IN IP4 {}\r\n\
         t=0 0\r\n\
         m=audio {} RTP/AVP 0 8\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=sendrecv\r\n",
        local_ip, local_ip, port
    );
    
    Ok(answer)
} 