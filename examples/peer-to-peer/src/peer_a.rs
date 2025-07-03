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
use tracing::{error, info};

/// Peer A - Initiates the call
#[derive(Clone)]
struct PeerAHandler {
    client_manager: Arc<RwLock<Option<Arc<ClientManager>>>>,
    call_completed: Arc<Mutex<bool>>,
    call_id: Arc<Mutex<Option<CallId>>>,
}

impl PeerAHandler {
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
}

#[async_trait::async_trait]
impl ClientEventHandler for PeerAHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 [PEER A] Incoming call: {} from {} to {}", 
            call_info.call_id, call_info.caller_uri, call_info.callee_uri);
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
        
        info!("{} [PEER A] Call {} state: {:?} → {:?}", 
            state_emoji, status_info.call_id, status_info.previous_state, status_info.new_state);
        
        if status_info.new_state == CallState::Connected {
            info!("🎉 [PEER A] Call connected! Starting media session...");
            
            // Start audio transmission
            if let Some(client) = self.client_manager.read().await.as_ref() {
                match client.start_audio_transmission(&status_info.call_id).await {
                    Ok(_) => info!("🎵 [PEER A] Audio transmission started"),
                    Err(e) => error!("❌ [PEER A] Failed to start audio: {}", e),
                }
                
                // Get media info
                if let Ok(media_info) = client.get_call_media_info(&status_info.call_id).await {
                    info!("📊 [PEER A] Media info - Local RTP: {:?}, Remote RTP: {:?}, Codec: {:?}",
                        media_info.local_rtp_port, media_info.remote_rtp_port, media_info.codec);
                }
            }
        } else if status_info.new_state == CallState::Terminated {
            info!("📴 [PEER A] Call terminated - marking as completed");
            *self.call_completed.lock().await = true;
        }
    }

    async fn on_media_event(&self, event: MediaEventInfo) {
        info!("🎵 [PEER A] Media event for {}: {:?}", event.call_id, event.event_type);
    }

    async fn on_registration_status_changed(&self, _status_info: RegistrationStatusInfo) {
        // Not needed for peer-to-peer
    }

    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        error!("❌ [PEER A] Error on call {:?}: {}", call_id, error);
    }

    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        let status = if connected { "🌐 Connected" } else { "🔌 Disconnected" };
        info!("{} [PEER A] Network status changed", status);
        if let Some(reason) = reason {
            info!("💬 [PEER A] Reason: {}", reason);
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
                .add_directive("peer_a=info".parse()?)
                .add_directive("rvoip=debug".parse()?)
        )
        .init();

    info!("🚀 [PEER A] Starting peer-to-peer demo");
    info!("📞 [PEER A] SIP Port: 5060, Media Port: 20000");

    // Create configuration for Peer A
    let config = ClientConfig::new()
        .with_sip_addr("127.0.0.1:5060".parse()?)
        .with_media_addr("127.0.0.1:20000".parse()?)
        .with_user_agent("RVOIP-Peer-A/1.0".to_string())
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
            rtp_port_start: 20000,
            rtp_port_end: 20100,
            ..Default::default()
        });

    // Create handler and client
    let handler = Arc::new(PeerAHandler::new());
    let client = ClientManager::new(config).await?;
    
    handler.set_client_manager(client.clone()).await;
    client.set_event_handler(handler.clone()).await;
    
    // Start the client
    client.start().await?;
    info!("✅ [PEER A] Client started and ready");

    // Wait a moment for Peer B to be ready
    info!("⏳ [PEER A] Waiting 3 seconds for Peer B to be ready...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Make a call to Peer B
    info!("📞 [PEER A] Initiating call to Peer B...");
    let from_uri = "sip:alice@127.0.0.1:5060".to_string();
    let to_uri = "sip:bob@127.0.0.1:5061".to_string();
    
    let call_id = client.make_call(from_uri, to_uri, None).await?;
    info!("📞 [PEER A] Call initiated with ID: {}", call_id);
    
    // Store the call ID
    *handler.call_id.lock().await = Some(call_id.clone());

    // Let the call run for 15 seconds to exchange media
    info!("⏰ [PEER A] Letting call run for 15 seconds...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    // Get final statistics
    if let Ok(Some(rtp_stats)) = client.get_rtp_statistics(&call_id).await {
        info!("📊 [PEER A] Final RTP Stats - Sent: {} packets ({} bytes), Received: {} packets ({} bytes)",
            rtp_stats.packets_sent, rtp_stats.bytes_sent, 
            rtp_stats.packets_received, rtp_stats.bytes_received);
    }

    // Terminate the call
    info!("📴 [PEER A] Terminating call...");
    client.hangup_call(&call_id).await?;

    // Wait for call termination
    let mut attempts = 0;
    while !handler.is_call_completed().await && attempts < 10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        attempts += 1;
    }

    // Stop the client
    client.stop().await?;
    info!("✅ [PEER A] Demo completed successfully!");

    Ok(())
} 