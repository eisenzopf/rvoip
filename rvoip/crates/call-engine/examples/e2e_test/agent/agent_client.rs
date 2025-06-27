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
use rvoip_sip_core::sdp::parser::parse_sdp;
use bytes::Bytes;

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

/// Event handler that immediately accepts incoming calls
struct AutoAcceptCallHandler {
    rtp_debug: bool,
    client: Arc<tokio::sync::RwLock<Option<Arc<ClientManager>>>>,
}

impl AutoAcceptCallHandler {
    fn new(rtp_debug: bool) -> Self {
        Self { 
            rtp_debug,
            client: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    
    async fn set_client(&self, client: Arc<ClientManager>) {
        *self.client.write().await = Some(client);
    }
}

#[async_trait]
impl ClientEventHandler for AutoAcceptCallHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 Incoming call from {} to {} (call_id: {})", 
            call_info.caller_uri, call_info.callee_uri, call_info.call_id);
        
        // Phase 0.9 - Accept immediately
        // The session coordinator will handle SDP negotiation automatically
        info!("✅ Accepting call {} immediately", call_info.call_id);
        CallAction::Accept
    }
    
    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        info!("📱 Call {} state changed from {:?} to {:?} (reason: {:?})",
            status_info.call_id,
            status_info.previous_state,
            status_info.new_state,
            status_info.reason
        );
        
        match status_info.new_state {
            CallState::Connected => {
                info!("✅ Call {} is now connected", status_info.call_id);
                
                // Get the client from the handler to establish media
                let client_guard = self.client.read().await;
                if let Some(client) = client_guard.as_ref() {
                    // Get call info to find remote SDP
                    match client.get_call(&status_info.call_id).await {
                        Ok(call_info) => {
                            // Check if we have remote SDP in metadata
                            if let Some(remote_sdp) = call_info.metadata.get("remote_sdp") {
                                info!("Found remote SDP for call {}", status_info.call_id);
                                
                                // Clone the SDP to avoid lifetime issues
                                let remote_sdp = remote_sdp.clone();
                                
                                // Parse SDP to extract media endpoint
                                match parse_sdp(&Bytes::from(remote_sdp)) {
                                    Ok(sdp_session) => {
                                        // Extract connection info from SDP
                                        let mut media_addr = None;
                                        
                                        // First check media-level connection
                                        if let Some(media) = sdp_session.media_descriptions.first() {
                                            if let Some(conn) = &media.connection_info {
                                                let addr = format!("{}:{}", conn.connection_address, media.port);
                                                info!("Found media connection: {}", addr);
                                                media_addr = Some(addr);
                                            }
                                        }
                                        
                                        // Fall back to session-level connection
                                        if media_addr.is_none() {
                                            if let Some(conn) = &sdp_session.connection_info {
                                                if let Some(media) = sdp_session.media_descriptions.first() {
                                                    let addr = format!("{}:{}", conn.connection_address, media.port);
                                                    info!("Found session connection: {}", addr);
                                                    media_addr = Some(addr);
                                                }
                                            }
                                        }
                                        
                                        // Establish media if we found an address
                                        if let Some(addr) = media_addr {
                                            info!("📡 Establishing media flow to {}", addr);
                                            match client.establish_media(&status_info.call_id, &addr).await {
                                                Ok(_) => {
                                                    info!("✅ Media flow established for call {}", status_info.call_id);
                                                    
                                                    // Start audio transmission after media is established
                                                    info!("🔊 Starting audio transmission for call {}", status_info.call_id);
                                                    if let Err(e) = client.start_audio_transmission(&status_info.call_id).await {
                                                        error!("Failed to start audio: {}", e);
                                                    }
                                                }
                                                Err(e) => error!("❌ Failed to establish media for call {}: {}", status_info.call_id, e),
                                            }
                                        } else {
                                            error!("❌ No media connection info found in SDP for call {}", status_info.call_id);
                                        }
                                    }
                                    Err(e) => {
                                        error!("❌ Failed to parse remote SDP for call {}: {}", status_info.call_id, e);
                                    }
                                }
                            } else {
                                warn!("⚠️ No remote SDP found for connected call {}", status_info.call_id);
                            }
                        }
                        Err(e) => {
                            error!("❌ Failed to get call info for {}: {}", status_info.call_id, e);
                        }
                    }
                }
            }
            CallState::Terminated => {
                info!("📴 Call {} terminated", status_info.call_id);
            }
            CallState::Failed => {
                error!("❌ Call {} failed", status_info.call_id);
            }
            _ => {}
        }
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
    
    // Set up the auto-accept event handler
    info!("Setting up auto-accept call handler...");
    let handler = Arc::new(AutoAcceptCallHandler::new(args.rtp_debug));
    
    // Set the client reference in the handler first
    handler.set_client(client.clone()).await;
    
    // Then set it as the event handler
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
    
    // Start event handler for call state changes and auto-hangup
    let event_client = client.clone();
    let call_duration = args.call_duration;
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
    event_handler.abort();
    
    // Stop the client
    client.stop().await?;
    
    info!("👋 Agent client shutdown complete");
    Ok(())
}

/// Handle client events for auto-hangup
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
                                // If call_duration is set, automatically hang up after duration
                                if info.new_state == CallState::Connected && call_duration > 0 {
                                    let client_clone = client.clone();
                                    let call_id_clone = info.call_id.clone();
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