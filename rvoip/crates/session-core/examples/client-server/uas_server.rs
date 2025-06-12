//! UAS Server Example
//! 
//! This example demonstrates a simple SIP User Agent Server (UAS)
//! that receives calls from a UAC client.

use anyhow::Result;
use clap::Parser;
use rvoip_session_core::api::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::sync::Mutex;
use tracing::{info, error};
use tracing_subscriber;

#[derive(Parser, Debug)]
#[command(name = "uas_server")]
#[command(about = "SIP UAS Server Example")]
pub struct Args {
    /// SIP listening port
    #[arg(short, long, default_value = "5062")]
    pub port: u16,
    
    /// Log level
    #[arg(short, long, default_value = "info")]
    pub log_level: String,
    
    /// Auto-shutdown after N seconds of inactivity
    #[arg(long)]
    pub auto_shutdown: Option<u64>,
}

/// Call statistics for monitoring
#[derive(Debug, Clone, Default)]
pub struct CallStats {
    pub total_calls: u32,
    pub active_calls: u32,
    pub successful_calls: u32,
    pub failed_calls: u32,
}

impl CallStats {
    pub fn success_rate(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            (self.successful_calls as f64 / self.total_calls as f64) * 100.0
        }
    }
}

/// Call handler for the UAS server
#[derive(Debug)]
pub struct UasCallHandler {
    stats: Arc<Mutex<CallStats>>,
    name: String,
}

impl UasCallHandler {
    pub fn new(stats: Arc<Mutex<CallStats>>) -> Self {
        Self {
            stats,
            name: "UAS-Server".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for UasCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("📞 [{}] Incoming call from {} to {}", self.name, call.from, call.to);
        info!("📞 [{}] Call ID: {}", self.name, call.id);
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.total_calls += 1;
            stats.active_calls += 1;
        }
        
        info!("✅ [{}] Auto-answering call", self.name);
        
        // Generate SDP answer if we have an offer
        let sdp_answer = if let Some(ref sdp_offer) = call.sdp {
            info!("📞 [{}] Received SDP offer, generating answer", self.name);
            match generate_sdp_answer(sdp_offer, "127.0.0.1", 10001) {
                Ok(answer) => {
                    info!("📞 [{}] Generated SDP answer", self.name);
                    Some(answer)
                }
                Err(e) => {
                    error!("📞 [{}] SDP answer generation failed: {}", self.name, e);
                    None
                }
            }
        } else {
            None
        };
        
        CallDecision::Accept(sdp_answer)
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("📴 [{}] Call {} ended: {}", self.name, call.id(), reason);
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.active_calls = stats.active_calls.saturating_sub(1);
            stats.successful_calls += 1;
        }
    }
}

/// UAS Server implementation
pub struct UasServer {
    session_manager: Arc<SessionManager>,
    stats: Arc<Mutex<CallStats>>,
    start_time: Instant,
    port: u16,
}

impl UasServer {
    /// Create a new UAS server
    pub async fn new(port: u16) -> Result<Self> {
        info!("🚀 Starting UAS Server on port {}", port);
        
        let stats = Arc::new(Mutex::new(CallStats::default()));
        let handler = Arc::new(UasCallHandler::new(Arc::clone(&stats)));
        
        // Create session manager with session-core
        let session_manager = SessionManagerBuilder::new()
            .with_sip_port(port)
            .with_from_uri(format!("sip:uas@127.0.0.1:{}", port))
            .with_sip_bind_address("127.0.0.1".to_string())
            .with_media_ports(10000, 20000)
            .p2p_mode()
            .with_handler(handler)
            .build()
            .await?;
        
        let server = Self {
            session_manager,
            stats,
            start_time: Instant::now(),
            port,
        };
        
        Ok(server)
    }
    
    /// Start the server and handle events
    pub async fn run(&self) -> Result<()> {
        info!("🚀 Starting UAS Server session manager...");
        
        // Start the session manager - this actually binds to the SIP port!
        self.session_manager.start().await?;
        
        info!("✅ UAS Server ready and listening on port {}", self.port);
        info!("🔄 Waiting for incoming calls...");
        
        // Wait for shutdown signal
        self.wait_for_shutdown().await?;
        
        info!("🛑 UAS Server shutting down");
        self.print_final_stats().await;
        
        Ok(())
    }
    
    /// Wait for shutdown signal
    async fn wait_for_shutdown(&self) -> Result<()> {
        // Handle Ctrl+C
        let ctrl_c = async {
            signal::ctrl_c().await.expect("Failed to install Ctrl+C handler");
            info!("📡 Received Ctrl+C signal");
        };
        
        // Handle SIGTERM
        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to install signal handler")
                .recv()
                .await;
            info!("📡 Received SIGTERM signal");
        };
        
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();
        
        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
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
        
        if stats.total_calls > 0 {
            let calls_per_second = stats.total_calls as f64 / uptime.as_secs_f64();
            info!("  🚀 Average call rate: {:.2} calls/second", calls_per_second);
        }
    }
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(&args.log_level)
        .with_target(false)
        .init();
    
    info!("🧪 UAS Server starting...");
    info!("🔧 Configuration:");
    info!("  📡 Port: {}", args.port);
    
    // Create and run server
    let server = UasServer::new(args.port).await?;
    
    // Run with timeout if specified
    if let Some(timeout_secs) = args.auto_shutdown {
        info!("⏰ Auto-shutdown enabled: {} seconds", timeout_secs);
        tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            server.run()
        ).await??;
    } else {
        server.run().await?;
    }
    
    info!("👋 UAS Server shutdown complete");
    Ok(())
} 