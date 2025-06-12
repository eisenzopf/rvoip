//! UAC Client Example
//! 
//! This example demonstrates a simple SIP User Agent Client (UAC)
//! that makes calls to a UAS server.

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
    #[arg(short, long, default_value = "3")]
    pub calls: usize,
    
    /// Call duration in seconds
    #[arg(short, long, default_value = "5")]
    pub duration: u64,
}

/// Statistics tracking for the UAC
#[derive(Debug, Default)]
struct UacStats {
    calls_initiated: usize,
    calls_completed: usize,
    calls_failed: usize,
    total_duration: Duration,
}

/// UAC Call Handler
#[derive(Debug)]
struct UacHandler {
    stats: Arc<Mutex<UacStats>>,
}

impl UacHandler {
    fn new(stats: Arc<Mutex<UacStats>>) -> Self {
        Self { stats }
    }
}

#[async_trait::async_trait]
impl CallHandler for UacHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // UAC doesn't expect incoming calls
        warn!("UAC received unexpected incoming call from {}", call.from);
        CallDecision::Reject("UAC does not accept incoming calls".to_string())
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("Call {} ended: {}", call.id, reason);
        
        let mut stats = self.stats.lock().await;
        if reason.contains("success") || reason.contains("normal") {
            stats.calls_completed += 1;
        } else {
            stats.calls_failed += 1;
        }
        
        if let Some(started_at) = call.started_at {
            stats.total_duration += started_at.elapsed();
        }
    }
}

async fn make_test_calls(
    session_manager: Arc<SessionCoordinator>,
    target: String,
    num_calls: usize,
    call_duration: Duration,
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
        
        // Make the call
        let to_uri = format!("sip:uas@{}", target);
        match session_manager.create_outgoing_call(
            &format!("sip:uac@127.0.0.1"),
            &to_uri,
            None
        ).await {
            Ok(call) => {
                info!("Call {} initiated successfully", call.id);
                
                // Wait for call duration
                tokio::time::sleep(call_duration).await;
                
                // Terminate the call
                info!("Terminating call {} after {:?}", call.id, call_duration);
                if let Err(e) = session_manager.terminate_session(&call.id).await {
                    error!("Failed to terminate call {}: {}", call.id, e);
                }
            }
            Err(e) => {
                error!("Failed to make call: {}", e);
                let mut s = stats.lock().await;
                s.calls_failed += 1;
            }
        }
        
        // Brief pause between calls
        if i < num_calls - 1 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    let args = Args::parse();
    let port = args.port;
    
    info!("Starting UAC client on port {}", port);
    
    // Create stats tracker
    let stats = Arc::new(Mutex::new(UacStats::default()));
    
    // Create session manager
    let session_manager = SessionManagerBuilder::new()
        .with_sip_port(port)
        .with_local_address(format!("sip:uac@127.0.0.1:{}", port))
        .with_handler(Arc::new(UacHandler::new(stats.clone())))
        .build()
        .await?;
    
    info!("UAC client initialized on {}", session_manager.get_bound_address());
    
    // Make test calls
    let call_task = make_test_calls(
        session_manager.clone(),
        args.target,
        args.calls,
        Duration::from_secs(args.duration),
        stats.clone(),
    );
    
    // Wait for calls to complete or Ctrl+C
    tokio::select! {
        result = call_task => {
            if let Err(e) = result {
                error!("Call task error: {}", e);
            }
        }
        _ = signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
        }
    }
    
    // Wait a bit for any pending operations
    info!("Waiting for pending operations...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Print final statistics
    let final_stats = stats.lock().await;
    info!("=== Final Statistics ===");
    info!("Calls initiated: {}", final_stats.calls_initiated);
    info!("Calls completed: {}", final_stats.calls_completed);
    info!("Calls failed: {}", final_stats.calls_failed);
    info!("Total call duration: {:?}", final_stats.total_duration);
    
    // Shutdown
    session_manager.stop().await?;
    info!("UAC client shutdown complete");
    
    Ok(())
} 