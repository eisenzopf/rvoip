//! SIP Test Server for SIPp Integration
//! 
//! This example demonstrates a full SIP call lifecycle with audio exchange.
//! It acts as a SIP UAS (User Agent Server) that can receive calls from SIPp
//! and perform a complete call flow including media negotiation and RTP audio.

use tokio::time::{sleep, Duration};
use tracing::{info, warn, error};
use std::sync::Arc;
use std::net::SocketAddr;

use rvoip_client_core::{
    ClientManager, ClientConfig, ClientEventHandler, 
    call::{CallId, CallState},
    events::{
        CallAction, IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo,
        MediaEventInfo, MediaEventType
    },
    error::ClientError,
    MediaConfig,
};

/// Test server event handler that accepts incoming calls and handles media
struct TestServerEventHandler {
    auto_answer: bool,
}

impl TestServerEventHandler {
    fn new(auto_answer: bool) -> Self {
        Self { auto_answer }
    }
}

#[async_trait::async_trait]
impl ClientEventHandler for TestServerEventHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!(
            "📞 Incoming call from: {} (Call-ID: {})", 
            call_info.caller_uri,
            call_info.call_id
        );
        
        if let Some(subject) = &call_info.subject {
            info!("📝 Call subject: {}", subject);
        }
        
        if self.auto_answer {
            info!("🔔 Auto-answer enabled, deferring for SDP generation");
            // Defer so we can accept with SDP answer
            CallAction::Ignore
        } else {
            info!("🔔 Call ringing (manual answer required)");
            CallAction::Ignore
        }
    }

    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_emoji = match status_info.new_state {
            CallState::Initiating => "🚀",
            CallState::Proceeding => "⏳", 
            CallState::Ringing => "🔔",
            CallState::Connected => "📞",
            CallState::Terminating => "👋",
            CallState::Terminated => "🔚",
            CallState::Failed => "❌",
            CallState::Cancelled => "🚫",
            CallState::IncomingPending => "📨",
        };
        
        info!(
            "{} Call {} state changed: {:?} -> {:?}", 
            state_emoji,
            status_info.call_id,
            status_info.previous_state.as_ref().map(|s| format!("{:?}", s)).unwrap_or_else(|| "None".to_string()),
            status_info.new_state
        );
        
        if let Some(reason) = &status_info.reason {
            info!("💬 Reason: {}", reason);
        }
    }

    async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo) {
        info!(
            "📋 Registration status for {}: {:?}",
            status_info.user_uri, status_info.status
        );
    }

    async fn on_media_event(&self, event: MediaEventInfo) {
        let emoji = match &event.event_type {
            MediaEventType::AudioStarted => "▶️",
            MediaEventType::AudioStopped => "⏹️",
            MediaEventType::MediaSessionStarted { .. } => "🎵",
            MediaEventType::MediaSessionStopped => "⏹️",
            MediaEventType::SdpOfferGenerated { .. } => "📄",
            MediaEventType::SdpAnswerProcessed { .. } => "📥",
            _ => "🔊",
        };
        
        info!("{} Media Event for call {}: {:?}", emoji, event.call_id, event.event_type);
    }

    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        if let Some(call_id) = call_id {
            error!("💥 Error for call {}: {}", call_id, error);
        } else {
            error!("💥 General error: {}", error);
        }
    }

    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        let status = if connected { "🌐 Connected" } else { "🔌 Disconnected" };
        info!("{} Network status changed", status);
        
        if let Some(reason) = reason {
            info!("💬 Reason: {}", reason);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    info!("🚀 Starting SIP Test Server for SIPp Integration");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let sip_port: u16 = args.get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5060);
    let media_port: u16 = args.get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20000);
    let auto_answer = args.get(3)
        .map(|s| s == "auto")
        .unwrap_or(true);

    // Create client configuration
    let sip_addr = format!("0.0.0.0:{}", sip_port).parse::<SocketAddr>()?;
    let media_addr = format!("0.0.0.0:{}", media_port).parse::<SocketAddr>()?;
    
    let config = ClientConfig::new()
        .with_sip_addr(sip_addr)
        .with_media_addr(media_addr)
        .with_user_agent("rvoip-sipp-test-server/1.0.0".to_string())
        .with_media(MediaConfig {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            dtmf_enabled: true,
            echo_cancellation: false,  // Not needed for test server
            noise_suppression: false,  // Not needed for test server
            auto_gain_control: false,  // Not needed for test server
            rtp_port_start: media_port,
            rtp_port_end: media_port + 1000,
            preferred_ptime: Some(20),
            custom_sdp_attributes: {
                let mut attrs = std::collections::HashMap::new();
                attrs.insert("a=tool".to_string(), "rvoip-sipp-test".to_string());
                attrs
            },
            ..Default::default()
        })
        .with_max_calls(10);

    info!("⚙️  Server configuration:");
    info!("   📞 SIP Address: {}", config.local_sip_addr);
    info!("   🎵 Media Address: {}", config.local_media_addr);
    info!("   🤖 User Agent: {}", config.user_agent);
    info!("   🎧 Codecs: {:?}", config.media.preferred_codecs);
    info!("   🔄 Auto-answer: {}", auto_answer);

    // Create the client manager
    let client = ClientManager::new(config).await?;
    
    // Set up event handler
    let event_handler = Arc::new(TestServerEventHandler::new(auto_answer));
    client.set_event_handler(event_handler).await;

    // Start the client
    info!("▶️  Starting SIP server...");
    client.start().await?;
    
    let stats = client.get_client_stats().await;
    info!("✅ SIP Server ready!");
    info!("   📍 Listening on SIP: {}", stats.local_sip_addr);
    info!("   📍 Media port: {}", stats.local_media_addr);
    info!("   ⏳ Waiting for incoming calls from SIPp...");

    // Set up graceful shutdown
    let client_clone = client.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        info!("🛑 Received shutdown signal");
        client_clone.stop().await.expect("Failed to stop client");
        std::process::exit(0);
    });

    // Main loop - handle incoming calls
    let mut call_count = 0;
    let mut answered_calls = std::collections::HashSet::new();
    
    loop {
        sleep(Duration::from_secs(1)).await;
        
        // Get current stats
        let stats = client.get_client_stats().await;
        
        // Check for new calls
        let active_calls = client.get_active_calls().await;
        
        // Auto-answer pending incoming calls if enabled
        if auto_answer {
            for call_info in &active_calls {
                if call_info.state == CallState::IncomingPending && 
                   !answered_calls.contains(&call_info.call_id) {
                    info!("✅ Auto-answering call {}", call_info.call_id);
                    match client.answer_call(&call_info.call_id).await {
                        Ok(_) => {
                            info!("📞 Successfully answered call {} with SDP", call_info.call_id);
                            answered_calls.insert(call_info.call_id);
                        }
                        Err(e) => {
                            error!("❌ Failed to answer call {}: {}", call_info.call_id, e);
                        }
                    }
                }
            }
        }
        
        if active_calls.len() > call_count {
            call_count = active_calls.len();
            info!("📊 Active calls: {}", call_count);
            
            // For each connected call, simulate media activity
            for call_info in &active_calls {
                if call_info.state == CallState::Connected {
                    info!("🎵 Call {} is connected", call_info.call_id);
                    
                    // Get media info
                    match client.get_call_media_info(&call_info.call_id).await {
                        Ok(media_info) => {
                            info!("   📡 Local RTP: {:?}", media_info.local_rtp_port);
                            info!("   📡 Remote RTP: {:?}", media_info.remote_rtp_port);
                            info!("   🎵 Codec: {:?}", media_info.codec);
                        }
                        Err(e) => {
                            warn!("   ⚠️  No media info yet: {}", e);
                        }
                    }
                }
            }
        }
        
        // Clean up terminated calls
        let terminated_calls = client.get_calls_by_state(CallState::Terminated).await;
        if !terminated_calls.is_empty() {
            info!("🧹 Cleaning up {} terminated calls", terminated_calls.len());
            call_count = active_calls.len() - terminated_calls.len();
        }
        
        // Print periodic status
        if call_count > 0 || stats.total_calls > 0 {
            info!("📊 Server Stats: Total={}, Active={}, Connected={}", 
                stats.total_calls, call_count, stats.connected_calls);
        }
    }
} 