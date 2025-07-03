use anyhow::Result;
use rvoip::{
    client_core::{
        ClientConfig, ClientEventHandler, ClientError, 
        IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo, MediaEventInfo,
        CallAction, CallId, CallState, MediaConfig,
        client::ClientManager,
    },
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

/// Peer B - Receives and answers calls
#[derive(Clone)]
struct PeerBHandler {
    client_manager: Arc<RwLock<Option<Arc<ClientManager>>>>,
    call_completed: Arc<Mutex<bool>>,
    call_id: Arc<Mutex<Option<CallId>>>,
}

impl PeerBHandler {
    pub fn new() -> Self {
        Self {
            client_manager: Arc::new(RwLock::new(None)),
            call_completed: Arc::new(Mutex::new(false)),
            call_id: Arc::new(Mutex::new(None)),
        }
    }
    
    async fn set_client_manager(&self, client: Arc<ClientManager>) {
        *self.client_manager.write().await = Some(client);
    }
    
    pub async fn is_call_completed(&self) -> bool {
        *self.call_completed.lock().await
    }
    
    pub async fn get_call_id(&self) -> Option<CallId> {
        self.call_id.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl ClientEventHandler for PeerBHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 [PEER B] Incoming call: {} from {} to {}", 
            call_info.call_id, call_info.caller_uri, call_info.callee_uri);
        
        // Store the call ID
        *self.call_id.lock().await = Some(call_info.call_id.clone());
        
        // Auto-answer after a short delay
        let client_ref = Arc::clone(&self.client_manager);
        let call_id = call_info.call_id.clone();
        
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Some(client) = client_ref.read().await.as_ref() {
                info!("📞 [PEER B] Auto-answering call: {}", call_id);
                match client.answer_call(&call_id).await {
                    Ok(_) => info!("✅ [PEER B] Call answered successfully"),
                    Err(e) => error!("❌ [PEER B] Failed to answer call: {}", e),
                }
            }
        });
        
        CallAction::Ignore // We'll handle it asynchronously
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
        
        info!("{} [PEER B] Call {} state: {:?} → {:?}", 
            state_emoji, status_info.call_id, status_info.previous_state, status_info.new_state);
        
        if status_info.new_state == CallState::Connected {
            info!("🎉 [PEER B] Call connected! Starting media session...");
            
            // Start audio transmission
            if let Some(client) = self.client_manager.read().await.as_ref() {
                match client.start_audio_transmission(&status_info.call_id).await {
                    Ok(_) => info!("🎵 [PEER B] Audio transmission started"),
                    Err(e) => error!("❌ [PEER B] Failed to start audio: {}", e),
                }
                
                // Get media info
                if let Ok(media_info) = client.get_call_media_info(&status_info.call_id).await {
                    info!("📊 [PEER B] Media info - Local RTP: {:?}, Remote RTP: {:?}, Codec: {:?}",
                        media_info.local_rtp_port, media_info.remote_rtp_port, media_info.codec);
                }
            }
        } else if status_info.new_state == CallState::Terminated {
            info!("📴 [PEER B] Call terminated - marking as completed");
            *self.call_completed.lock().await = true;
        }
    }

    async fn on_media_event(&self, event: MediaEventInfo) {
        info!("🎵 [PEER B] Media event for {}: {:?}", event.call_id, event.event_type);
    }

    async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {
        // Not needed for peer-to-peer
    }

    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        error!("❌ [PEER B] Error on call {:?}: {}", call_id, error);
    }

    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        let status = if connected { "🌐 Connected" } else { "🔌 Disconnected" };
        info!("{} [PEER B] Network status changed", status);
        if let Some(reason) = reason {
            info!("💬 [PEER B] Reason: {}", reason);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Create logs directory
    std::fs::create_dir_all("logs")?;
    
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("peer_b=info".parse()?)
                .add_directive("rvoip=debug".parse()?)
        )
        .init();

    info!("🚀 [PEER B] Starting peer-to-peer demo");
    info!("📞 [PEER B] SIP Port: 5061, Media Port: 21000");

    // Create configuration for Peer B
    let config = ClientConfig::new()
        .with_sip_addr("127.0.0.1:5061".parse()?)
        .with_media_addr("127.0.0.1:21000".parse()?)
        .with_user_agent("RVOIP-Peer-B/1.0".to_string())
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
            rtp_port_start: 21000,
            rtp_port_end: 21100,
            ..Default::default()
        });

    // Create handler and client
    let handler = Arc::new(PeerBHandler::new());
    let client = ClientManager::new(config).await?;
    
    handler.set_client_manager(client.clone()).await;
    client.set_event_handler(handler.clone()).await;
    
    // Start the client
    client.start().await?;
    info!("✅ [PEER B] Client started and ready to receive calls");

    // Wait for call to complete or timeout (30 seconds)
    info!("⏳ [PEER B] Waiting for incoming call...");
    let mut timeout_counter = 0;
    
    while !handler.is_call_completed().await && timeout_counter < 60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        timeout_counter += 1;
        
        // Log periodic status updates
        if timeout_counter % 10 == 0 {
            info!("⏰ [PEER B] Still waiting... ({} seconds elapsed)", timeout_counter);
        }
    }

    // Get final statistics if we have a call
    if let Some(call_id) = handler.get_call_id().await {
        if let Ok(Some(rtp_stats)) = client.get_rtp_statistics(&call_id).await {
            info!("📊 [PEER B] Final RTP Stats - Sent: {} packets ({} bytes), Received: {} packets ({} bytes)",
                rtp_stats.packets_sent, rtp_stats.bytes_sent, 
                rtp_stats.packets_received, rtp_stats.bytes_received);
        }
    }

    // Stop the client
    client.stop().await?;
    
    if handler.is_call_completed().await {
        info!("✅ [PEER B] Demo completed successfully!");
    } else {
        warn!("⚠️ [PEER B] Demo timed out - no call received");
    }

    Ok(())
} 