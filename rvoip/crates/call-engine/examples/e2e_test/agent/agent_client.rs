//! Agent Client for E2E Testing
//!
//! This agent client:
//! 1. Registers with the call center server via SIP REGISTER
//! 2. Accepts incoming calls automatically
//! 3. Plays a test tone or silence for audio
//! 4. Hangs up after a configurable duration

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use tracing::{info, warn, error, debug};
use clap::Parser;
use uuid::Uuid;

use rvoip_client_core::{
    ClientConfig, ClientEventHandler, ClientError, ClientManager,
    IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
    CallAction, CallId, CallState, MediaEventType, MediaConfig,
    EventPriority, CallInfo,
};
use async_trait::async_trait;

#[derive(Parser, Debug)]
#[command(author, version, about = "SIP Agent Client for Call Center Testing", long_about = None)]
struct Args {
    /// Agent username (e.g., alice, bob)
    #[arg(short, long)]
    username: String,
    
    /// Call center server address
    #[arg(short, long, default_value = "127.0.0.1:5060")]
    server: String,
    
    /// Local SIP port to bind to
    #[arg(short, long, default_value = "0")]
    port: u16,
    
    /// Domain name
    #[arg(short, long, default_value = "127.0.0.1")]
    domain: String,
    
    /// Call duration in seconds (0 for manual hangup)
    #[arg(long, default_value = "10")]
    call_duration: u64,
    
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
    
    /// Enable RTP debug logging
    #[arg(long)]
    rtp_debug: bool,
}

const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(30);
const POLLING_INTERVAL: Duration = Duration::from_millis(100);

/// Event handler that defers incoming calls (like UAS example)
struct DeferCallHandler {
    rtp_debug: bool,
}

impl DeferCallHandler {
    fn new(rtp_debug: bool) -> Self {
        Self { rtp_debug }
    }
}

#[async_trait]
impl ClientEventHandler for DeferCallHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 Deferring incoming call from {} for later handling", call_info.caller_uri);
        CallAction::Ignore // Defer the call - we'll handle it in the polling loop
    }
    
    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_icon = match status_info.new_state {
            CallState::Initiating => "🔄",
            CallState::Proceeding => "📤",
            CallState::Ringing => "🔔",
            CallState::IncomingPending => "📥",
            CallState::Connected => "📞",
            CallState::Terminating => "📴",
            CallState::Terminated => "☎️",
            CallState::Failed => "❌",
            CallState::Cancelled => "🚫",
        };
        
        info!("{} Call {} state: {:?} → {:?} ({})",
            state_icon,
            status_info.call_id,
            status_info.previous_state.as_ref().unwrap_or(&CallState::Initiating),
            status_info.new_state,
            status_info.reason.as_deref().unwrap_or("no reason")
        );
    }
    
    async fn on_media_event(&self, media_info: MediaEventInfo) {
        match &media_info.event_type {
            MediaEventType::MediaSessionStarted { media_session_id } => {
                info!("🎵 Media session started for call {}: {}", media_info.call_id, media_session_id);
            }
            MediaEventType::MediaSessionStopped => {
                info!("🛑 Media session stopped for call {}", media_info.call_id);
            }
            MediaEventType::AudioStarted => {
                info!("🎵 Audio transmission started for call {}", media_info.call_id);
            }
            MediaEventType::AudioStopped => {
                info!("🛑 Audio transmission stopped for call {}", media_info.call_id);
            }
            MediaEventType::SdpOfferGenerated { sdp_size } => {
                if self.rtp_debug {
                    info!("📋 SDP offer generated for call {} ({} bytes)", media_info.call_id, sdp_size);
                }
            }
            MediaEventType::SdpAnswerProcessed { sdp_size } => {
                if self.rtp_debug {
                    info!("📋 SDP answer processed for call {} ({} bytes)", media_info.call_id, sdp_size);
                }
            }
            _ => {
                if self.rtp_debug {
                    debug!("🎵 Media event for call {}: {:?}", media_info.call_id, media_info.event_type);
                }
            }
        }
    }
    
    async fn on_registration_status_changed(&self, reg_info: RegistrationStatusInfo) {
        use rvoip_client_core::registration::RegistrationStatus;
        
        match reg_info.status {
            RegistrationStatus::Active => {
                info!("✅ Registration active: {}", reg_info.user_uri);
            }
            RegistrationStatus::Failed => {
                error!("❌ Registration failed: {}", reg_info.reason.as_deref().unwrap_or("unknown reason"));
            }
            RegistrationStatus::Expired => {
                warn!("⏰ Registration expired for {}", reg_info.user_uri);
            }
            _ => {
                debug!("Registration status: {:?}", reg_info.status);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Initialize logging
    let log_level = if args.verbose { tracing::Level::DEBUG } else { tracing::Level::INFO };
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .init();
    
    info!("🤖 Starting agent client for {}", args.username);
    
    // Build SIP URIs
    let agent_uri = format!("sip:{}@{}", args.username, args.domain);
    let server_addr: SocketAddr = args.server.parse()?;
    
    // Create client configuration
    let local_sip_addr = format!("0.0.0.0:{}", args.port).parse()?;
    let local_media_addr = format!("0.0.0.0:{}", args.port + 100).parse()?; // Media port offset
    
    info!("Creating SIP client...");
    let config = ClientConfig::new()
        .with_sip_addr(local_sip_addr)
        .with_media_addr(local_media_addr)
        .with_user_agent("RVoIP-Agent/1.0".to_string())
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
            ..Default::default()
        });
    
    let client = ClientManager::new(config).await?;
    
    // Set up the deferred call event handler (like UAS example)
    info!("Setting up deferred call handler...");
    let handler = Arc::new(DeferCallHandler::new(args.rtp_debug));
    client.set_event_handler(handler).await;
    
    // Start the client
    client.start().await?;
    
    // Register with the server using client-core registration API
    info!("📝 Registering as {} with server {}", agent_uri, server_addr);
    
    use rvoip_client_core::registration::RegistrationConfig;
    let reg_config = RegistrationConfig::new(
        format!("sip:{}", server_addr),  // registrar
        agent_uri.clone(),                // from_uri
        agent_uri.clone(),                // contact_uri
    )
    .with_expires(120); // 120 second expiry to prevent timeout during testing
    
    let registration_id = client.register(reg_config).await?;
    
    info!("✅ Successfully registered with ID: {}", registration_id);
    info!("👂 Agent {} is ready to receive calls...", args.username);
    
    // Start handling incoming calls (like UAS example)
    let call_client = client.clone();
    let call_duration = args.call_duration;
    let incoming_call_handler = tokio::spawn(async move {
        handle_incoming_calls(call_client, call_duration).await;
    });
    
    // Start general event handler for call state changes
    let event_client = client.clone();
    let event_handler = tokio::spawn(async move {
        handle_client_events(event_client, call_duration).await;
    });
    
    // Keep the client running
    info!("Press Ctrl+C to stop");
    
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    
    // Unregister before shutdown
    info!("🔚 Unregistering...");
    if let Err(e) = client.unregister(registration_id).await {
        warn!("Failed to unregister: {}", e);
    }
    
    // Stop handlers
    incoming_call_handler.abort();
    event_handler.abort();
    
    // Stop the client
    client.stop().await?;
    
    info!("👋 Agent client shutdown complete");
    Ok(())
}

/// Handle incoming calls by polling for IncomingPending state (like UAS example)
async fn handle_incoming_calls(client: Arc<ClientManager>, call_duration: u64) {
    loop {
        sleep(POLLING_INTERVAL).await;
        
        // Get all active calls
        let active_calls = client.get_active_calls().await;
        
        // Find calls in IncomingPending state
        for call_info in active_calls {
            if call_info.state == CallState::IncomingPending {
                info!("📥 Answering pending call {} from {}", 
                      call_info.call_id, call_info.remote_uri);
                
                match client.answer_call(&call_info.call_id).await {
                    Ok(_) => {
                        info!("✅ Successfully answered call {}", call_info.call_id);
                        
                        // If call_duration is set, automatically hang up after duration
                        if call_duration > 0 {
                            let client_clone = client.clone();
                            let call_id_clone = call_info.call_id.clone();
                            tokio::spawn(async move {
                                sleep(Duration::from_secs(call_duration)).await;
                                info!("⏰ Auto-hanging up call {} after {} seconds", 
                                      call_id_clone, call_duration);
                                if let Err(e) = client_clone.hangup_call(&call_id_clone).await {
                                    error!("Failed to hang up call: {}", e);
                                }
                            });
                        }
                    }
                    Err(e) => {
                        error!("❌ Failed to answer call {}: {}", call_info.call_id, e);
                    }
                }
            }
        }
    }
}

/// Handle client events (focusing on call state changes and media)
async fn handle_client_events(client: Arc<ClientManager>, call_duration: u64) {
    let mut event_rx = client.subscribe_events();
    
    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Ok(ev) => {
                        use rvoip_client_core::ClientEvent;
                        match ev {
                            ClientEvent::CallStateChanged { info, .. } => {
                                // Check if we should start transmitting audio
                                if info.new_state == CallState::Connected {
                                    info!("🔊 Call {} connected - starting audio transmission", info.call_id);
                                    if let Err(e) = client.start_audio_transmission(&info.call_id).await {
                                        error!("Failed to start audio: {}", e);
                                    }
                                }
                            }
                            
                            // Other events are handled by the event handler
                            _ => {}
                        }
                    }
                    Err(_) => {
                        // Channel closed, exit
                        break;
                    }
                }
            }
        }
    }
} 