//! Call Center Customer
//!
//! This customer:
//! 1. Makes a call to the call center support line (sip:support@127.0.0.1)
//! 2. Stays on the call for a configurable duration
//! 3. Hangs up and provides statistics

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error, warn};
use clap::Parser;

use rvoip::{
    client_core::{
        ClientConfig, ClientEventHandler, ClientError, ClientManager,
        IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
        CallAction, CallId, CallState, MediaConfig,
    },
};
use async_trait::async_trait;

#[derive(Parser, Debug)]
#[command(author, version, about = "Call Center Customer", long_about = None)]
struct Args {
    /// Customer name
    #[arg(short, long, default_value = "customer")]
    name: String,
    
    /// Call center server address
    #[arg(short, long, default_value = "127.0.0.1:5060")]
    server: String,
    
    /// Local SIP port to bind to
    #[arg(short, long, default_value = "5080")]
    port: u16,
    
    /// Call duration in seconds
    #[arg(long, default_value = "15")]
    call_duration: u64,
    
    /// Wait time before making call
    #[arg(long, default_value = "3")]
    wait_time: u64,
}

/// Event handler for the customer
#[derive(Clone)]
struct CustomerHandler {
    name: String,
    call_completed: Arc<tokio::sync::Mutex<bool>>,
    call_id: Arc<tokio::sync::Mutex<Option<CallId>>>,
    client: Arc<tokio::sync::RwLock<Option<Arc<ClientManager>>>>,
}

impl CustomerHandler {
    fn new(name: String) -> Self {
        Self {
            name,
            call_completed: Arc::new(tokio::sync::Mutex::new(false)),
            call_id: Arc::new(tokio::sync::Mutex::new(None)),
            client: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    async fn set_client(&self, client: Arc<ClientManager>) {
        *self.client.write().await = Some(client);
    }
    
    async fn is_call_completed(&self) -> bool {
        *self.call_completed.lock().await
    }
    
    async fn set_call_id(&self, call_id: CallId) {
        *self.call_id.lock().await = Some(call_id);
    }
}

#[async_trait]
impl ClientEventHandler for CustomerHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 [{}] Unexpected incoming call from {}", 
            self.name, call_info.caller_uri);
        CallAction::Accept
    }
    
    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_emoji = match status_info.new_state {
            CallState::Initiating => "🔄",
            CallState::Ringing => "🔔",
            CallState::Connected => "✅",
            CallState::Failed => "❌",
            CallState::Cancelled => "🚫",
            CallState::Terminated => "📴",
            _ => "❓",
        };
        
        info!("{} [{}] Call {} state: {:?} → {:?}", 
            state_emoji, self.name, status_info.call_id, 
            status_info.previous_state, status_info.new_state);
        
        match status_info.new_state {
            CallState::Ringing => {
                info!("🔔 [{}] Call is ringing... waiting for agent to answer", self.name);
            }
            CallState::Connected => {
                info!("🎉 [{}] Connected to agent! Starting media session...", self.name);
                
                // Start audio transmission
                if let Some(client) = self.client.read().await.as_ref() {
                    if let Err(e) = client.start_audio_transmission(&status_info.call_id).await {
                        error!("❌ [{}] Failed to start audio: {}", self.name, e);
                    } else {
                        info!("🎵 [{}] Audio transmission started", self.name);
                    }
                    
                    // Get media info
                    if let Ok(media_info) = client.get_call_media_info(&status_info.call_id).await {
                        info!("📊 [{}] Media info - Codec: {:?}, Local RTP: {:?}, Remote RTP: {:?}",
                            self.name, media_info.codec, media_info.local_rtp_port, media_info.remote_rtp_port);
                    }
                }
            }
            CallState::Failed => {
                error!("❌ [{}] Call failed: {:?}", self.name, status_info.reason);
                *self.call_completed.lock().await = true;
            }
            CallState::Terminated => {
                info!("📴 [{}] Call terminated", self.name);
                *self.call_completed.lock().await = true;
            }
            _ => {}
        }
    }
    
    async fn on_media_event(&self, event: MediaEventInfo) {
        info!("🎵 [{}] Media event for {}: {:?}", 
            self.name, event.call_id, event.event_type);
    }
    
    async fn on_registration_status_changed(&self, _reg_info: RegistrationStatusInfo) {
        // Not needed for customer calls
    }
    
    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        error!("❌ [{}] Client error on call {:?}: {}", self.name, call_id, error);
    }
    
    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        if connected {
            info!("🌐 [{}] Network connected", self.name);
        } else {
            warn!("🔌 [{}] Network disconnected: {}", 
                self.name, reason.unwrap_or("unknown".to_string()));
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Create logs directory
    std::fs::create_dir_all("logs")?;
    
    // Initialize logging with file output
    let file_appender = tracing_appender::rolling::never("logs", "customer.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("call_center_demo=info".parse()?)
                .add_directive("rvoip=info".parse()?)
        )
        .init();
    
    info!("👤 Starting customer: {}", args.name);
    info!("🏢 Call center server: {}", args.server);
    info!("📱 Local SIP port: {}", args.port);
    info!("⏰ Call duration: {}s", args.call_duration);
    info!("⌛ Wait time: {}s", args.wait_time);
    
    // Create client configuration
    let local_sip_addr = format!("0.0.0.0:{}", args.port).parse()?;
    let local_media_addr = format!("0.0.0.0:{}", args.port + 1000).parse()?;
    
    let config = ClientConfig::new()
        .with_sip_addr(local_sip_addr)
        .with_media_addr(local_media_addr)
        .with_user_agent(format!("CallCenter-Customer-{}/1.0", args.name))
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
            rtp_port_start: args.port + 2000,
            rtp_port_end: args.port + 2100,
            ..Default::default()
        });
    
    // Create client and handler
    let client = ClientManager::new(config).await?;
    let handler = Arc::new(CustomerHandler::new(args.name.clone()));
    
    handler.set_client(client.clone()).await;
    client.set_event_handler(handler.clone()).await;
    
    // Start the client
    client.start().await?;
    info!("✅ [{}] Client started", args.name);
    
    // Wait for the call center to be ready
    info!("⏳ [{}] Waiting {} seconds for call center to be ready...", args.name, args.wait_time);
    sleep(Duration::from_secs(args.wait_time)).await;
    
    // Make a call to the support line
    info!("📞 [{}] Calling call center support line...", args.name);
    let from_uri = format!("sip:{}@127.0.0.1:{}", args.name, args.port);
    let to_uri = "sip:support@127.0.0.1".to_string();
    
    let call_id = client.make_call(from_uri, to_uri.clone(), None).await?;
    info!("📞 [{}] Call initiated to {} with ID: {}", args.name, to_uri, call_id);
    
    handler.set_call_id(call_id.clone()).await;
    
    // Let the call run for the specified duration
    info!("⏰ [{}] Staying on call for {} seconds...", args.name, args.call_duration);
    sleep(Duration::from_secs(args.call_duration)).await;
    
    // Get final statistics before hanging up
    if let Ok(Some(rtp_stats)) = client.get_rtp_statistics(&call_id).await {
        info!("📊 [{}] Final RTP Stats - Sent: {} packets ({} bytes), Received: {} packets ({} bytes)",
            args.name, rtp_stats.packets_sent, rtp_stats.bytes_sent,
            rtp_stats.packets_received, rtp_stats.bytes_received);
    }
    
    // Hang up the call
    info!("📴 [{}] Hanging up call...", args.name);
    client.hangup_call(&call_id).await?;
    
    // Wait for call termination
    let mut attempts = 0;
    while !handler.is_call_completed().await && attempts < 20 {
        sleep(Duration::from_millis(500)).await;
        attempts += 1;
    }
    
    if handler.is_call_completed().await {
        info!("✅ [{}] Call completed successfully", args.name);
    } else {
        warn!("⚠️  [{}] Call may not have terminated cleanly", args.name);
    }
    
    // Stop the client
    client.stop().await?;
    info!("👋 [{}] Customer session completed", args.name);
    
    Ok(())
} 