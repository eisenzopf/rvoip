//! Call Center Agent
//!
//! This agent:
//! 1. Registers with the call center server via SIP REGISTER
//! 2. Accepts incoming calls automatically
//! 3. Handles calls for a configurable duration
//! 4. Provides detailed logging

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, error};
use clap::Parser;

use rvoip_client_core::{
    ClientConfig, ClientEventHandler, ClientError, ClientManager,
    IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
    CallAction, CallId, CallState, MediaConfig,
    registration::{RegistrationConfig, RegistrationStatus},
};
use async_trait::async_trait;

#[derive(Parser, Debug)]
#[command(author, version, about = "Call Center Agent", long_about = None)]
struct Args {
    /// Agent name (e.g., alice, bob)
    #[arg(short, long, default_value = "alice")]
    name: String,
    
    /// Call center server address
    #[arg(short, long, default_value = "127.0.0.1:5060")]
    server: String,
    
    /// Local SIP port to bind to
    #[arg(short, long, default_value = "5071")]
    port: u16,
    
    /// Call duration in seconds
    #[arg(long, default_value = "10")]
    call_duration: u64,
}

/// Event handler for the agent
#[derive(Clone)]
struct AgentHandler {
    name: String,
    call_duration: u64,
    client: Arc<tokio::sync::RwLock<Option<Arc<ClientManager>>>>,
}

impl AgentHandler {
    fn new(name: String, call_duration: u64) -> Self {
        Self {
            name,
            call_duration,
            client: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    async fn set_client(&self, client: Arc<ClientManager>) {
        *self.client.write().await = Some(client);
    }
}

#[async_trait]
impl ClientEventHandler for AgentHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 [{}] Incoming call from {} (call_id: {})", 
            self.name, call_info.caller_uri, call_info.call_id);
        
        // Accept the call immediately
        info!("✅ [{}] Accepting call {}", self.name, call_info.call_id);
        CallAction::Accept
    }
    
    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_emoji = match status_info.new_state {
            CallState::IncomingPending => "🔔",
            CallState::Connected => "✅",
            CallState::Failed => "❌",
            CallState::Terminated => "📴",
            _ => "🔄",
        };
        
        info!("{} [{}] Call {} state: {:?} → {:?}", 
            state_emoji, self.name, status_info.call_id, 
            status_info.previous_state, status_info.new_state);
        
        match status_info.new_state {
            CallState::Connected => {
                info!("🎉 [{}] Call {} connected! Starting media...", self.name, status_info.call_id);
                
                // Start audio transmission
                if let Some(client) = self.client.read().await.as_ref() {
                    if let Err(e) = client.start_audio_transmission(&status_info.call_id).await {
                        error!("❌ [{}] Failed to start audio: {}", self.name, e);
                    } else {
                        info!("🎵 [{}] Audio transmission started", self.name);
                    }
                    
                    // Auto-hangup after call duration
                    let client_clone = client.clone();
                    let call_id = status_info.call_id.clone();
                    let name = self.name.clone();
                    let duration = self.call_duration;
                    
                    tokio::spawn(async move {
                        sleep(Duration::from_secs(duration)).await;
                        info!("⏰ [{}] Auto-hanging up call {} after {} seconds", 
                              name, call_id, duration);
                        
                        if let Err(e) = client_clone.hangup_call(&call_id).await {
                            error!("❌ [{}] Failed to hang up call: {}", name, e);
                        }
                    });
                }
            }
            CallState::Terminated => {
                info!("📴 [{}] Call {} completed", self.name, status_info.call_id);
            }
            CallState::Failed => {
                error!("❌ [{}] Call {} failed", self.name, status_info.call_id);
            }
            _ => {}
        }
    }
    
    async fn on_media_event(&self, event: MediaEventInfo) {
        info!("🎵 [{}] Media event for {}: {:?}", 
            self.name, event.call_id, event.event_type);
    }
    
    async fn on_registration_status_changed(&self, reg_info: RegistrationStatusInfo) {
        match reg_info.status {
            RegistrationStatus::Active => {
                info!("✅ [{}] Registration active: {}", self.name, reg_info.user_uri);
            }
            RegistrationStatus::Failed => {
                error!("❌ [{}] Registration failed: {}", 
                    self.name, reg_info.reason.as_deref().unwrap_or("unknown"));
            }
            RegistrationStatus::Expired => {
                warn!("⏰ [{}] Registration expired", self.name);
            }
            _ => {
                info!("🔄 [{}] Registration status: {:?}", self.name, reg_info.status);
            }
        }
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
    let file_appender = tracing_appender::rolling::never("logs", format!("{}.log", args.name));
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("call_center_demo=info".parse()?)
                .add_directive("rvoip_client_core=info".parse()?)
        )
        .init();
    
    info!("🤖 Starting agent: {}", args.name);
    info!("🏢 Call center server: {}", args.server);
    info!("📱 Local SIP port: {}", args.port);
    info!("⏰ Call duration: {}s", args.call_duration);
    
    // Build URIs
    let agent_uri = format!("sip:{}@127.0.0.1", args.name);
    let server_uri = format!("sip:{}", args.server);
    
    // Create client configuration
    let local_sip_addr = format!("0.0.0.0:{}", args.port).parse()?;
    let local_media_addr = format!("0.0.0.0:{}", args.port + 1000).parse()?;
    
    let config = ClientConfig::new()
        .with_sip_addr(local_sip_addr)
        .with_media_addr(local_media_addr)
        .with_user_agent(format!("CallCenter-Agent-{}/1.0", args.name))
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
            ..Default::default()
        });
    
    // Create client and handler
    let client = ClientManager::new(config).await?;
    let handler = Arc::new(AgentHandler::new(args.name.clone(), args.call_duration));
    
    handler.set_client(client.clone()).await;
    client.set_event_handler(handler).await;
    
    // Start the client
    client.start().await?;
    info!("✅ [{}] Client started", args.name);
    
    // Register with the call center server
    info!("📝 [{}] Registering with call center server...", args.name);
    
    let reg_config = RegistrationConfig::new(
        server_uri,
        agent_uri.clone(),
        agent_uri.clone(),
    ).with_expires(300); // 5 minute expiry
    
    let registration_id = client.register(reg_config).await?;
    info!("✅ [{}] Successfully registered with ID: {}", args.name, registration_id);
    
    info!("👂 [{}] Agent ready to receive calls!", args.name);
    
    // Keep the agent running
    tokio::signal::ctrl_c().await?;
    
    // Cleanup
    info!("🔚 [{}] Shutting down...", args.name);
    if let Err(e) = client.unregister(registration_id).await {
        warn!("⚠️  [{}] Failed to unregister: {}", args.name, e);
    }
    
    client.stop().await?;
    info!("👋 [{}] Agent shutdown complete", args.name);
    
    Ok(())
} 