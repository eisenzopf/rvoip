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
    pub calls: u32,
    
    /// Call duration in seconds
    #[arg(short, long, default_value = "10")]
    pub duration: u64,
    
    /// Log level
    #[arg(short, long, default_value = "info")]
    pub log_level: String,
}

/// Call statistics for monitoring
#[derive(Debug, Clone, Default)]
pub struct CallStats {
    pub total_calls: u32,
    pub active_calls: u32,
    pub successful_calls: u32,
    pub failed_calls: u32,
    pub total_duration: Duration,
}

impl CallStats {
    pub fn success_rate(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            (self.successful_calls as f64 / self.total_calls as f64) * 100.0
        }
    }
    
    pub fn average_duration(&self) -> Duration {
        if self.successful_calls == 0 {
            Duration::ZERO
        } else {
            self.total_duration / self.successful_calls
        }
    }
}

/// Call handler for the UAC client
#[derive(Debug)]
pub struct UacCallHandler {
    stats: Arc<Mutex<CallStats>>,
    name: String,
}

impl UacCallHandler {
    pub fn new(stats: Arc<Mutex<CallStats>>) -> Self {
        Self {
            stats,
            name: "UAC-Client".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for UacCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        warn!("⚠️ [{}] Unexpected incoming call from {} to {}", self.name, call.from, call.to);
        warn!("⚠️ [{}] This is a UAC client and shouldn't receive calls", self.name);
        
        // Reject unexpected calls
        CallDecision::Reject("UAC client does not accept calls".to_string())
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("📴 [{}] Call {} ended: {}", self.name, call.id(), reason);
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.active_calls = stats.active_calls.saturating_sub(1);
            
            if reason.contains("normal") || reason.contains("200 OK") {
                stats.successful_calls += 1;
                if let Some(started_at) = call.started_at {
                    let duration = started_at.elapsed();
                    stats.total_duration += duration;
                    info!("⏱️ [{}] Call duration: {:.2}s", self.name, duration.as_secs_f64());
                }
            } else {
                stats.failed_calls += 1;
            }
        }
    }
}

/// UAC Client implementation
pub struct UacClient {
    session_manager: Arc<SessionManager>,
    stats: Arc<Mutex<CallStats>>,
    start_time: Instant,
    port: u16,
    target: String,
}

impl UacClient {
    /// Create a new UAC client
    pub async fn new(port: u16, target: String) -> Result<Self> {
        info!("🚀 Starting UAC Client on port {}", port);
        
        let stats = Arc::new(Mutex::new(CallStats::default()));
        let handler = Arc::new(UacCallHandler::new(Arc::clone(&stats)));
        
        // Create session manager with session-core
        let session_manager = SessionManagerBuilder::new()
            .with_sip_port(port)
            .with_from_uri(format!("sip:uac@127.0.0.1:{}", port))
            .with_sip_bind_address("127.0.0.1".to_string())
            .with_media_ports(20000, 30000)
            .p2p_mode()
            .with_handler(handler)
            .build()
            .await?;
        
        let client = Self {
            session_manager,
            stats,
            start_time: Instant::now(),
            port,
            target,
        };
        
        Ok(client)
    }
    
    /// Start the client and make calls
    pub async fn run(&self, num_calls: u32, call_duration: Duration) -> Result<()> {
        info!("🚀 Starting UAC Client session manager...");
        
        // Start the session manager
        self.session_manager.start().await?;
        
        info!("✅ UAC Client ready on port {}", self.port);
        info!("🎯 Target: {}", self.target);
        info!("📞 Making {} calls with duration {}s each", num_calls, call_duration.as_secs());
        
        // Make the specified number of calls
        for i in 0..num_calls {
            info!("📞 Making call {}/{}", i + 1, num_calls);
            self.make_call(call_duration).await?;
            
            // Small delay between calls
            if i < num_calls - 1 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        
        // Wait for all calls to complete
        self.wait_for_calls_to_complete().await?;
        
        info!("🛑 UAC Client shutting down");
        self.print_final_stats().await;
        
        Ok(())
    }
    
    /// Make a single call
    async fn make_call(&self, duration: Duration) -> Result<()> {
        // Create SDP offer for the call
        let sdp_offer = generate_sdp_offer("127.0.0.1", 20000);
        
        // Update stats before making the call
        {
            let mut stats = self.stats.lock().await;
            stats.total_calls += 1;
            stats.active_calls += 1;
        }
        
        // Make the outgoing call using the session manager's public API
        // The API expects from, to, and sdp parameters
        let from = format!("sip:uac@127.0.0.1:{}", self.port);
        let to = format!("sip:uas@{}", self.target);
        
        // Use the create_outgoing_call method from the SessionManager
        let call_result = self.session_manager.create_outgoing_call(
            &from,
            &to,
            Some(sdp_offer),
        ).await;
        
        match call_result {
            Ok(call) => {
                info!("📞 Call initiated with ID: {}", call.id());
                
                // Keep the call active for the specified duration
                info!("⏱️ Keeping call active for {}s", duration.as_secs());
                tokio::time::sleep(duration).await;
                
                // End the call using terminate_session
                info!("📴 Ending call {}", call.id());
                if let Err(e) = self.session_manager.terminate_session(&call.id()).await {
                    error!("❌ Failed to end call: {}", e);
                }
                
                Ok(())
            },
            Err(e) => {
                error!("❌ Failed to make call: {}", e);
                
                // Update stats for failed call
                {
                    let mut stats = self.stats.lock().await;
                    stats.active_calls = stats.active_calls.saturating_sub(1);
                    stats.failed_calls += 1;
                }
                
                Err(e.into())
            }
        }
    }
    
    /// Wait for all calls to complete
    async fn wait_for_calls_to_complete(&self) -> Result<()> {
        info!("⏳ Waiting for all calls to complete...");
        
        // Check every 500ms if all calls are done
        loop {
            let active_calls = {
                let stats = self.stats.lock().await;
                stats.active_calls
            };
            
            if active_calls == 0 {
                info!("✅ All calls completed");
                break;
            }
            
            info!("⏳ Still waiting for {} active calls to complete", active_calls);
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        
        Ok(())
    }
    
    /// Print final statistics
    async fn print_final_stats(&self) {
        let stats = self.stats.lock().await;
        let uptime = self.start_time.elapsed();
        
        info!("📊 Final Statistics:");
        info!("  ⏱️  Uptime: {:.2} seconds", uptime.as_secs_f64());
        info!("  📞 Total calls: {}", stats.total_calls);
        info!("  ✅ Successful calls: {}", stats.successful_calls);
        info!("  ❌ Failed calls: {}", stats.failed_calls);
        info!("  🔄 Active calls: {}", stats.active_calls);
        info!("  📈 Success rate: {:.1}%", stats.success_rate());
        
        if stats.successful_calls > 0 {
            let avg_duration = stats.average_duration();
            info!("  ⏱️ Average call duration: {:.2}s", avg_duration.as_secs_f64());
        }
    }
}

/// Generate SDP offer for the call
fn generate_sdp_offer(local_ip: &str, port: u16) -> String {
    format!(
        "v=0\r\n\
         o=uac 123456 654321 IN IP4 {}\r\n\
         s=UAC Test Call\r\n\
         c=IN IP4 {}\r\n\
         t=0 0\r\n\
         m=audio {} RTP/AVP 0 8\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=sendrecv\r\n",
        local_ip, local_ip, port
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(&args.log_level)
        .with_target(false)
        .init();
    
    info!("🧪 UAC Client starting...");
    info!("🔧 Configuration:");
    info!("  📡 Port: {}", args.port);
    info!("  🎯 Target: {}", args.target);
    info!("  📞 Calls: {}", args.calls);
    info!("  ⏱️  Duration: {}s", args.duration);
    
    // Create and run client
    let client = UacClient::new(args.port, args.target).await?;
    client.run(args.calls, Duration::from_secs(args.duration)).await?;
    
    info!("👋 UAC Client shutdown complete");
    Ok(())
} 