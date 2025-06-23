use anyhow::Result;
use clap::Parser;
use rvoip_client_core::{
    ClientConfig, ClientEventHandler, ClientError, 
    IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
    CallAction, CallId, CallState, MediaConfig, MediaEventType,
    client::ClientManager,
};
use std::sync::Arc;
use std::collections::HashSet;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// SIP listening port
    #[arg(short, long, default_value = "5070")]
    port: u16,

    /// Media port range start
    #[arg(short, long, default_value = "30000")]
    media_port: u16,

    /// Enable RTP packet logging
    #[arg(short, long)]
    rtp_debug: bool,
}

/// Simple UAS server that auto-answers calls and receives RTP
#[derive(Clone)]
struct SimpleUasHandler {
    rtp_debug: bool,
    auto_answer_delay_ms: u64,
    active_calls: Arc<Mutex<HashSet<CallId>>>,
    client_manager: Arc<RwLock<Option<Arc<ClientManager>>>>,
}

impl SimpleUasHandler {
    pub fn new(rtp_debug: bool) -> Self {
        Self {
            config: Arc::new(RwLock::new(ClientConfig::default())),
            client_manager: Arc::new(RwLock::new(None)),
            active_calls: Arc::new(Mutex::new(HashSet::new())),
            rtp_debug,
        }
    }
    
    async fn set_client_manager(&self, client: Arc<ClientManager>) {
        *self.client_manager.write().await = Some(client);
    }
}

#[async_trait::async_trait]
impl ClientEventHandler for SimpleUasHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!(
            "📞 Incoming call: {} from {} to {}",
            call_info.call_id, call_info.caller_uri, call_info.callee_uri
        );

        // For this demo, we'll use deferred handling (return Ignore)
        // and answer the call in the polling loop after a delay
        info!("⏳ Will auto-answer call {} after {} ms", 
            call_info.call_id, self.auto_answer_delay_ms);
        
        CallAction::Ignore
    }

    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_emoji = match status_info.new_state {
            CallState::Initiating => "🔄",
            CallState::Ringing => "🔔",
            CallState::Connected => "✅",
            CallState::OnHold => "⏸️",
            CallState::Terminating => "🔚",
            CallState::Terminated => "❌",
            CallState::Failed => "⚠️",
        };
        
        info!(
            "{} Call {} state changed: {:?} → {:?} {}",
            state_emoji,
            status_info.call_id,
            status_info.previous_state,
            status_info.new_state,
            status_info.reason.as_deref().unwrap_or("")
        );
        
        if status_info.new_state == CallState::Connected {
            let mut calls = self.active_calls.lock().await;
            calls.insert(status_info.call_id);
            info!("✅ Call {} connected - ready to receive RTP", status_info.call_id);
            
            // Start monitoring RTP statistics
            self.start_stats_monitoring(status_info.call_id).await;
            
            // The server doesn't need to explicitly start audio transmission
            // as it will automatically receive RTP packets sent by the client
            // However, we can get media info to see the negotiated parameters
            if let Some(client) = self.client_manager.read().await.as_ref() {
                if let Ok(media_info) = client.get_call_media_info(&status_info.call_id).await {
                    info!(
                        "📊 Media info for call {}: codec={:?}, local_port={}, remote_port={:?}, remote_sdp={}",
                        status_info.call_id,
                        media_info.codec,
                        media_info.local_rtp_port,
                        media_info.remote_rtp_port,
                        media_info.remote_sdp.is_some()
                    );
                }
            }
        } else if status_info.new_state == CallState::Terminated {
            let mut calls = self.active_calls.lock().await;
            calls.remove(&status_info.call_id);
            info!("Call {} terminated", status_info.call_id);
        }
    }

    async fn on_media_event(&self, event: MediaEventInfo) {
        // Always log important media events
        match &event.event_type {
            MediaEventType::MediaSessionStarted { media_session_id } => {
                info!("🎬 Media session started for call {}: {}", event.call_id, media_session_id);
            }
            MediaEventType::MediaSessionStopped => {
                info!("🛑 Media session stopped for call {}", event.call_id);
            }
            MediaEventType::AudioStarted => {
                info!("🎵 Audio transmission started for call {}", event.call_id);
            }
            MediaEventType::AudioStopped => {
                info!("🛑 Audio transmission stopped for call {}", event.call_id);
            }
            MediaEventType::SdpOfferGenerated { sdp_size } => {
                info!("📋 SDP offer generated for call {} ({} bytes)", event.call_id, sdp_size);
            }
            MediaEventType::SdpAnswerProcessed { sdp_size } => {
                info!("📋 SDP answer processed for call {} ({} bytes)", event.call_id, sdp_size);
            }
            _ => {
                if self.rtp_debug {
                    info!("🎵 Media event for call {}: {:?}", event.call_id, event.event_type);
                }
            }
        }

        // Log RTP packet reception if available
        if let Some(rtp_received) = event.metadata.get("rtp_packets_received") {
            info!("📥 RTP packets received for call {}: {}", event.call_id, rtp_received);
        }
        
        if let Some(rtp_stats) = event.metadata.get("rtp_stats") {
            info!("📊 RTP stats for call {}: {}", event.call_id, rtp_stats);
        }
    }

    async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {
        // UAS doesn't need registration for this demo
    }

    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        error!("❌ Error on call {:?}: {}", call_id, error);
    }

    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        let status = if connected { "🌐 Connected" } else { "🔌 Disconnected" };
        info!("{} Network status changed", status);
        if let Some(reason) = reason {
            info!("💬 Reason: {}", reason);
        }
    }
}

impl SimpleUasHandler {
    async fn start_stats_monitoring(&self, call_id: CallId) {
        let client_ref = Arc::clone(&self.client_manager);
        let call_id = call_id.clone();
        let rtp_debug = self.rtp_debug;
        
        tokio::spawn(async move {
            // Wait a bit for RTP to start flowing
            tokio::time::sleep(Duration::from_secs(1)).await;
            
            // Monitor statistics every second
            loop {
                if let Some(client) = client_ref.read().await.as_ref() {
                    // Get RTP statistics
                    if let Ok(Some(rtp_stats)) = client.get_rtp_statistics(&call_id).await {
                        if rtp_debug || rtp_stats.packets_received > 0 {
                            info!("📊 Server RTP Stats for {}: Sent: {} packets ({} bytes), Received: {} packets ({} bytes), Lost: {}", 
                                call_id,
                                rtp_stats.packets_sent,
                                rtp_stats.bytes_sent,
                                rtp_stats.packets_received,
                                rtp_stats.bytes_received,
                                rtp_stats.packets_lost
                            );
                        }
                    }
                    
                    // Get call statistics for quality metrics
                    if let Ok(Some(call_stats)) = client.get_call_statistics(&call_id).await {
                        if let Some(quality) = &call_stats.quality_metrics {
                            if rtp_debug {
                                info!("📈 Server Quality for {}: MOS: {:.2}, Jitter: {}ms, Packet Loss: {:.1}%",
                                    call_id,
                                    quality.mos_score,
                                    quality.jitter_ms,
                                    quality.packet_loss_percent
                                );
                            }
                        }
                    }
                    
                    // Check if call is still active
                    if let Ok(call_info) = client.get_call(&call_id).await {
                        if call_info.state == CallState::Terminated {
                            info!("Call {} terminated, stopping stats monitoring", call_id);
                            break;
                        }
                    } else {
                        // Call not found, stop monitoring
                        break;
                    }
                }
                
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_client_core=debug".parse()?)
                .add_directive("rvoip_media_core=info".parse()?)
                .add_directive("rvoip_rtp_core=info".parse()?)
                .add_directive("uas_server=info".parse()?),
        )
        .init();

    let args = Args::parse();

    info!("🚀 Starting UAS Server");
    info!("📞 SIP Port: {}", args.port);
    info!("🎵 Media Port Range: {}-{}", args.media_port, args.media_port + 1000);
    info!("🐛 RTP Debug: {}", args.rtp_debug);

    // Create server configuration
    let config = ClientConfig::new()
        .with_sip_addr(format!("0.0.0.0:{}", args.port).parse()?)
        .with_media_addr(format!("0.0.0.0:{}", args.media_port).parse()?)
        .with_user_agent("RVOIP-UAS-Server/1.0".to_string())
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string(), "G722".to_string(), "OPUS".to_string()],
            dtmf_enabled: true,
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
            rtp_port_start: args.media_port,
            rtp_port_end: args.media_port + 1000,
            ..Default::default()
        });

    // Create handler with auto-answer delay
    let handler = Arc::new(SimpleUasHandler::new(args.rtp_debug));

    // Build and start server
    let client = ClientManager::new(config).await?;
    
    // Set the client manager reference in the handler
    handler.set_client_manager(client.clone()).await;
    
    client.set_event_handler(handler.clone()).await;
    client.start().await?;

    info!("✅ UAS Server ready and listening on port {}", args.port);
    info!("⏳ Will auto-answer incoming calls after 100ms");
    info!("📥 Ready to receive RTP packets");

    // Main loop to handle incoming calls
    let mut check_interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
    
    loop {
        check_interval.tick().await;
        
        // Get active calls and answer pending ones
        let active_calls = client.get_active_calls().await;
        for call_info in active_calls {
            if call_info.state == CallState::IncomingPending {
                info!("📞 Answering pending call: {}", call_info.call_id);
                match client.answer_call(&call_info.call_id).await {
                    Ok(_) => info!("✅ Call {} answered", call_info.call_id),
                    Err(e) => error!("❌ Failed to answer call {}: {}", call_info.call_id, e),
                }
            }
        }
        
        // Optional: Add graceful shutdown mechanism here
        // For now, the server runs indefinitely
    }
} 