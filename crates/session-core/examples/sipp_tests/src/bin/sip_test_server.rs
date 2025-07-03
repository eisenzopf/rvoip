//! SIP Test Server - UAS that receives calls from SIPp
//! 
//! This application uses session-core to create a SIP User Agent Server (UAS)
//! that can receive and respond to calls from SIPp test scenarios.
//!
//! NOTE: This example uses some internal implementation details.
//! For the recommended approach using only the public API, see:
//! `examples/api_best_practices/uas_server_clean.rs`

use anyhow::Result;
use clap::{Parser, ValueEnum};
use rvoip_session_core::{SessionCoordinator, SessionManagerBuilder};
use rvoip_session_core::api::{
    CallHandler, CallSession, IncomingCall, CallDecision,
};
use sipp_tests::CallStats;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "sip_test_server")]
#[command(about = "SIP Test Server for SIPp integration testing")]
pub struct Args {
    /// SIP listening port
    #[arg(short, long, default_value = "5062")]
    pub port: u16,
    
    /// Response mode for incoming calls
    #[arg(short, long, default_value = "auto-answer")]
    pub mode: ResponseModeArg,
    
    /// Log level
    #[arg(short, long, default_value = "info")]
    pub log_level: String,
    
    /// Configuration file path
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    
    /// Enable metrics collection
    #[arg(long, default_value = "true")]
    pub metrics: bool,
    
    /// Auto-shutdown after N seconds of inactivity
    #[arg(long)]
    pub auto_shutdown: Option<u64>,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum ResponseModeArg {
    AutoAnswer,
    Busy,
    NotFound,
    Random,
}

/// Call handler for the SIP test server
#[derive(Debug)]
pub struct SipTestHandler {
    stats: Arc<Mutex<CallStats>>,
    response_mode: ResponseModeArg,
    name: String,
}

impl SipTestHandler {
    pub fn new(response_mode: ResponseModeArg, stats: Arc<Mutex<CallStats>>) -> Self {
        Self {
            stats,
            response_mode,
            name: "SIPp-Test-Server".to_string(),
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for SipTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("📞 [{}] Incoming call from {} to {}", self.name, call.from, call.to);
        info!("📞 [{}] Call ID: {}", self.name, call.id);
        
        // Update stats
        {
            let mut stats = self.stats.lock().await;
            stats.total_calls += 1;
            stats.active_calls += 1;
        }
        
        match self.response_mode {
            ResponseModeArg::AutoAnswer => {
                info!("✅ [{}] Auto-answering call", self.name);
                
                // Generate SDP answer if we have an offer
                let sdp_answer = if let Some(ref _sdp_offer) = call.sdp {
                    info!("📞 [{}] Received SDP offer, generating simple answer", self.name);
                    // Create a simple SDP answer that matches the offer
                    // In a real implementation, you'd negotiate codecs properly
                    let answer = format!(
                        "v=0\r\n\
                        o=rust 0 0 IN IP4 127.0.0.1\r\n\
                        s=Rust Session\r\n\
                        c=IN IP4 127.0.0.1\r\n\
                        t=0 0\r\n\
                        m=audio 10001 RTP/AVP 0 8\r\n\
                        a=rtpmap:0 PCMU/8000\r\n\
                        a=rtpmap:8 PCMA/8000\r\n\
                        a=sendrecv\r\n"
                    );
                    info!("📞 [{}] Generated SDP answer", self.name);
                    Some(answer)
                } else {
                    None
                };
                
                CallDecision::Accept(sdp_answer)
            }
            ResponseModeArg::Busy => {
                info!("📞 [{}] Rejecting call with 486 Busy Here", self.name);
                CallDecision::Reject("Busy Here".to_string())
            }
            ResponseModeArg::NotFound => {
                info!("📞 [{}] Rejecting call with 404 Not Found", self.name);
                CallDecision::Reject("Not Found".to_string())
            }
            ResponseModeArg::Random => {
                // Random choice
                let choice = fastrand::u8(0..3);
                match choice {
                    0 => {
                        info!("🎲 [{}] Random choice: Accept", self.name);
                        let sdp_answer = if let Some(ref _sdp_offer) = call.sdp {
                            // Create simple SDP answer
                            Some(format!(
                                "v=0\r\n\
                                o=rust 0 0 IN IP4 127.0.0.1\r\n\
                                s=Rust Session\r\n\
                                c=IN IP4 127.0.0.1\r\n\
                                t=0 0\r\n\
                                m=audio 10001 RTP/AVP 0 8\r\n\
                                a=rtpmap:0 PCMU/8000\r\n\
                                a=rtpmap:8 PCMA/8000\r\n\
                                a=sendrecv\r\n"
                            ))
                        } else {
                            None
                        };
                        CallDecision::Accept(sdp_answer)
                    }
                    1 => {
                        info!("🎲 [{}] Random choice: 486 Busy", self.name);
                        CallDecision::Reject("Busy Here".to_string())
                    }
                    _ => {
                        info!("🎲 [{}] Random choice: 404 Not Found", self.name);
                        CallDecision::Reject("Not Found".to_string())
                    }
                }
            }
        }
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

/// SIP Test Server implementation
pub struct SipTestServer {
    session_coordinator: Arc<SessionCoordinator>,
    stats: Arc<Mutex<CallStats>>,
    response_mode: ResponseModeArg,
    start_time: Instant,
    port: u16,
}

impl SipTestServer {
    /// Create a new SIP test server
    pub async fn new(port: u16, response_mode: ResponseModeArg) -> Result<Self> {
        info!("🚀 Starting SIP Test Server on port {}", port);
        
        let stats = Arc::new(Mutex::new(CallStats::default()));
        let handler = Arc::new(SipTestHandler::new(response_mode.clone(), Arc::clone(&stats)));
        
        // Create session coordinator with session-core
        let session_coordinator = SessionManagerBuilder::new()
            .with_sip_port(port)
            .with_local_address(format!("sip:test@127.0.0.1:{}", port))
            .with_media_ports(10000, 20000)
            .with_handler(handler)
            .build()
            .await?;
        
        let server = Self {
            session_coordinator,
            stats,
            response_mode,
            start_time: Instant::now(),
            port,
        };
        
        Ok(server)
    }
    
    /// Start the server and handle events
    pub async fn run(&self) -> Result<()> {
        info!("🚀 Starting SIP Test Server session coordinator...");
        
        // Start the session coordinator - this actually binds to the SIP port!
        self.session_coordinator.start().await?;
        
        info!("✅ SIP Test Server ready and listening on port {}", self.port);
        info!("📋 Response mode: {:?}", self.response_mode);
        info!("🔄 Waiting for incoming calls from SIPp...");
        info!("📡 Real SIP server now active - SIPp can connect!");
        
        // Wait for shutdown signal
        self.wait_for_shutdown().await?;
        
        info!("🛑 SIP Test Server shutting down");
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(&args.log_level)
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();
    
    info!("🧪 SIP Test Server starting...");
    info!("🔧 Configuration:");
    info!("  📡 Port: {}", args.port);
    info!("  🎯 Mode: {:?}", args.mode);
    info!("  📊 Metrics: {}", args.metrics);
    
    // Create and run server
    let server = SipTestServer::new(args.port, args.mode).await?;
    
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
    
    info!("👋 SIP Test Server shutdown complete");
    Ok(())
} 