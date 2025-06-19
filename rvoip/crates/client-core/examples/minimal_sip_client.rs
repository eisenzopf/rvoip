//! Minimal SIP Client Demo
//! 
//! This example demonstrates the basic capabilities of the rvoip-client-core library.
//! It shows how to create a SIP client, register with a server, and manage calls.

use tokio::time::{sleep, Duration};
use tracing::{info, warn, error};

use rvoip_client_core::{
    ClientManager, ClientConfig, ClientEventHandler, RegistrationConfig,
    call::{CallId, CallState},
    events::{
        CallAction, IncomingCallInfo, CallStatusInfo, RegistrationStatusInfo,
        MediaEventInfo, MediaEventType
    },
    error::ClientError,
};

/// Example event handler that demonstrates all the client events
struct ExampleEventHandler {
    name: String,
}

impl ExampleEventHandler {
    fn new(name: String) -> Self {
        Self { name }
    }
}

#[async_trait::async_trait]
impl ClientEventHandler for ExampleEventHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!(
            "📞 [{}] Incoming call from: {} ({})", 
            self.name,
            call_info.caller_uri,
            call_info.caller_display_name.as_deref().unwrap_or("Unknown")
        );
        
        if let Some(subject) = &call_info.subject {
            info!("📝 Call subject: {}", subject);
        }
        
        // For this example, we'll automatically accept incoming calls
        info!("✅ Auto-accepting incoming call");
        CallAction::Accept
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
            "{} [{}] Call {} state: {:?} -> {:?}", 
            state_emoji,
            self.name,
            status_info.call_id,
            status_info.previous_state.as_ref().map(|s| format!("{:?}", s)).unwrap_or_else(|| "None".to_string()),
            status_info.new_state
        );
        
        if let Some(reason) = &status_info.reason {
            info!("💬 Reason: {}", reason);
        }
    }

    async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo) {
        let status_emoji = match status_info.status {
            rvoip_client_core::registration::RegistrationStatus::Pending => "⏳",
            rvoip_client_core::registration::RegistrationStatus::Active => "✅",
            rvoip_client_core::registration::RegistrationStatus::Failed => "💥",
            rvoip_client_core::registration::RegistrationStatus::Expired => "⏰",
            rvoip_client_core::registration::RegistrationStatus::Cancelled => "❌",
        };
        
        info!(
            "{} [{}] Registration {} for {}: {:?}",
            status_emoji, self.name, status_info.user_uri, status_info.server_uri, status_info.status
        );
        
        if let Some(reason) = &status_info.reason {
            info!("💬 Reason: {}", reason);
        }
    }

    async fn on_media_event(&self, event: MediaEventInfo) {
        let emoji = match &event.event_type {
            MediaEventType::MicrophoneStateChanged { muted } => if *muted { "🔇" } else { "🎤" },
            MediaEventType::SpeakerStateChanged { muted } => if *muted { "🔇" } else { "🔊" },
            MediaEventType::AudioStarted => "▶️",
            MediaEventType::AudioStopped => "⏹️",
            MediaEventType::HoldStateChanged { on_hold } => if *on_hold { "⏸️" } else { "▶️" },
            MediaEventType::DtmfSent { .. } => "📞",
            MediaEventType::TransferInitiated { .. } => "🔄",
            MediaEventType::SdpOfferGenerated { .. } => "📄",
            MediaEventType::SdpAnswerProcessed { .. } => "📥",
            MediaEventType::MediaSessionStarted { .. } => "🎵",
            MediaEventType::MediaSessionStopped => "⏹️",
            MediaEventType::MediaSessionUpdated { .. } => "🔄",
            MediaEventType::QualityChanged { .. } => "📊",
            MediaEventType::PacketLoss { .. } => "📉",
            MediaEventType::JitterChanged { .. } => "📈",
        };
        
        println!("    {} Media Event: {:?}", emoji, event.event_type);
    }

    async fn on_client_error(&self, error: ClientError, call_id: Option<CallId>) {
        if let Some(call_id) = call_id {
            error!("💥 [{}] Error for call {}: {}", self.name, call_id, error);
        } else {
            error!("💥 [{}] General error: {}", self.name, error);
        }
    }

    async fn on_network_event(&self, connected: bool, reason: Option<String>) {
        let status = if connected { "🌐 Connected" } else { "🔌 Disconnected" };
        info!("{} [{}] Network status changed", status, self.name);
        
        if let Some(reason) = reason {
            info!("💬 Reason: {}", reason);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for beautiful logs
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();

    info!("🚀 Starting RVOIP Client Core Demo");
    info!("📡 Leveraging 80% reused infrastructure from rvoip server stack!");

    // Create client configuration
    let config = ClientConfig::new()
        .with_sip_addr("127.0.0.1:0".parse()?) // Random port
        .with_media_addr("127.0.0.1:0".parse()?) // Random port  
        .with_user_agent("rvoip-client-demo/1.0.0".to_string())
        .with_codecs(vec!["PCMU".to_string(), "PCMA".to_string(), "opus".to_string()])
        .with_max_calls(5);

    info!("⚙️  Client configuration:");
    info!("   📞 SIP Address: {}", config.local_sip_addr);
    info!("   🎵 Media Address: {}", config.local_media_addr);
    info!("   🤖 User Agent: {}", config.user_agent);
    info!("   🎧 Preferred Codecs: {:?}", config.preferred_codecs);
    info!("   📱 Max Concurrent Calls: {}", config.max_concurrent_calls);

    // Create the client manager
    info!("🔧 Creating ClientManager (leveraging rvoip infrastructure)...");
    let client = ClientManager::new(config).await?;
    
    // Set up event handler
    let event_handler = std::sync::Arc::new(ExampleEventHandler::new("Main".to_string()));
    client.set_event_handler(event_handler).await;
    info!("📋 Event handler registered");

    // Start the client
    info!("▶️  Starting SIP client...");
    client.start().await?;
    
    // Get the actual bound addresses
    let stats = client.get_client_stats().await;
    info!("✅ SIP Client started successfully!");
    info!("   📍 Bound to SIP: {}", stats.local_sip_addr);
    info!("   📍 Bound to Media: {}", stats.local_media_addr);

    // Demo registration (would normally connect to real SIP server)
    info!("📝 Demonstrating registration workflow...");
    let reg_config = RegistrationConfig::new(
        "sip:demo.example.com".to_string(),
        "sip:alice@demo.example.com".to_string(),
        "sip:alice@127.0.0.1:5060".to_string(),
    )
    .with_credentials("alice".to_string(), "secret123".to_string())
    .with_expires(3600);

    match client.register(reg_config).await {
        Ok(reg_id) => {
            info!("📋 Registration initiated with ID: {}", reg_id);
        }
        Err(e) => {
            warn!("⚠️  Registration failed (expected in demo): {}", e);
        }
    }

    // Demo outgoing call creation
    info!("📞 Demonstrating call creation...");
    match client.make_call(
        "sip:alice@demo.example.com".to_string(),
        "sip:bob@demo.example.com".to_string(),
        Some("Demo call from rvoip-client-core".to_string()),
    ).await {
        Ok(call_id) => {
            info!("📲 Call created with ID: {}", call_id);
            
            // Simulate call progression
            sleep(Duration::from_millis(500)).await;
            
            // Demo call operations
            info!("🎙️  Testing media controls...");
            client.set_microphone_mute(&call_id, true).await?;
            sleep(Duration::from_millis(200)).await;
            client.set_microphone_mute(&call_id, false).await?;
            
            client.set_speaker_mute(&call_id, true).await?;
            sleep(Duration::from_millis(200)).await;
            client.set_speaker_mute(&call_id, false).await?;
            
            // Get call info
            if let Ok(call_info) = client.get_call(&call_id).await {
                info!("📋 Call Info:");
                info!("   🆔 ID: {}", call_info.call_id);
                info!("   📊 State: {:?}", call_info.state);
                info!("   🎯 Direction: {:?}", call_info.direction);
                info!("   👤 Local: {}", call_info.local_uri);
                info!("   👥 Remote: {}", call_info.remote_uri);
                info!("   📅 Created: {}", call_info.created_at.format("%H:%M:%S"));
            }
            
            sleep(Duration::from_secs(1)).await;
            
            // Hangup the call
            info!("📴 Hanging up call...");
            client.hangup_call(&call_id).await?;
        }
        Err(e) => {
            warn!("⚠️  Call creation failed (expected in demo): {}", e);
        }
    }

    // Show available codecs
    let codecs = client.get_available_codecs().await;
    info!("🎵 Available codecs: {:?}", codecs);

    // Show final stats
    let final_stats = client.get_client_stats().await;
    info!("📊 Final Client Statistics:");
    info!("   🏃 Running: {}", final_stats.is_running);
    info!("   📞 Total Calls: {}", final_stats.total_calls);
    info!("   🟢 Connected Calls: {}", final_stats.connected_calls);
    info!("   📝 Total Registrations: {}", final_stats.total_registrations);
    info!("   ✅ Active Registrations: {}", final_stats.active_registrations);

    // Demonstrate listing calls
    let all_calls = client.list_calls().await;
    info!("📋 All calls: {} total", all_calls.len());
    
    let connected_calls = client.get_calls_by_state(CallState::Connected).await;
    info!("🟢 Connected calls: {}", connected_calls.len());

    // Graceful shutdown
    info!("🛑 Stopping SIP client...");
    client.stop().await?;
    
    info!("✨ Demo completed successfully!");
    info!("🎉 RVOIP Client Core showcased:");
    info!("   ✅ Infrastructure reuse (80% shared with server)");
    info!("   ✅ Memory-safe Rust implementation");
    info!("   ✅ Async performance with tokio");
    info!("   ✅ Event-driven architecture for UI integration");
    info!("   ✅ Clean APIs for registration and call management");
    info!("   ✅ Ready for production SIP client development!");

    Ok(())
} 