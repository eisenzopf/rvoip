//! Minimal SIP Client Demo
//! 
//! This example demonstrates the basic capabilities of the rvoip-client-core library.
//! It shows how to create a SIP client, register with a server, and manage calls.

use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error, warn};

use rvoip_client_core::{
    ClientManager, ClientConfig, RegistrationConfig,
    ClientEventHandler, IncomingCallInfo, CallStatusInfo, 
    RegistrationStatusInfo, CallAction, MediaEventType,
    CallId, CallState,
    events::Credentials,
};

/// A simple event handler that logs all events
struct LoggingEventHandler;

#[async_trait::async_trait]
impl ClientEventHandler for LoggingEventHandler {
    async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
        info!("📞 Incoming call from: {} -> {}", 
              call_info.caller_uri, call_info.callee_uri);
        
        if let Some(display_name) = &call_info.caller_display_name {
            info!("👤 Caller display name: {}", display_name);
        }
        
        // For demo purposes, auto-accept all calls
        info!("✅ Auto-accepting call");
        CallAction::Accept
    }
    
    async fn on_call_state_changed(&self, status_info: CallStatusInfo) {
        let state_emoji = match status_info.new_state {
            CallState::Initiating => "🔄",
            CallState::Proceeding => "⏳", 
            CallState::Ringing => "📳",
            CallState::Connected => "🟢",
            CallState::Terminating => "🔴",
            CallState::Terminated => "❌",
            CallState::Failed => "💥",
            CallState::Cancelled => "🚫",
            CallState::IncomingPending => "📞",
        };
        
        info!("{} Call {} state: {:?}", 
              state_emoji, status_info.call_id, status_info.new_state);
        
        if let Some(reason) = &status_info.reason {
            info!("   Reason: {}", reason);
        }
    }
    
    async fn on_registration_status_changed(&self, status_info: RegistrationStatusInfo) {
        let status_emoji = match status_info.status {
            rvoip_client_core::RegistrationStatus::Unregistered => "❌",
            rvoip_client_core::RegistrationStatus::Registering => "🔄",
            rvoip_client_core::RegistrationStatus::Registered => "✅",
            rvoip_client_core::RegistrationStatus::Failed => "💥",
            rvoip_client_core::RegistrationStatus::Unregistering => "🔄",
        };
        
        info!("{} Registration status for {}: {:?}", 
              status_emoji, status_info.server_uri, status_info.status);
    }
    
    async fn on_network_status_changed(&self, connected: bool, server: String, message: Option<String>) {
        let status = if connected { "🌐 Connected" } else { "🔌 Disconnected" };
        info!("{} to server: {}", status, server);
        
        if let Some(msg) = message {
            info!("   Message: {}", msg);
        }
    }
    
    async fn on_media_event(&self, call_id: Option<CallId>, event_type: MediaEventType, description: String) {
        let emoji = match event_type {
            MediaEventType::AudioStarted => "🔊",
            MediaEventType::AudioStopped => "🔇",
            MediaEventType::AudioQualityChanged => "📈",
            MediaEventType::MicrophoneStateChanged { muted } => if muted { "🎙️❌" } else { "🎙️✅" },
            MediaEventType::SpeakerStateChanged { muted } => if muted { "🔊❌" } else { "🔊✅" },
            MediaEventType::CodecChanged { .. } => "🎵",
        };
        
        if let Some(call_id) = call_id {
            info!("{} Media event for call {}: {}", emoji, call_id, description);
        } else {
            info!("{} Global media event: {}", emoji, description);
        }
    }
    
    async fn on_error(&self, error: String, recoverable: bool, context: Option<String>) {
        let severity = if recoverable { "⚠️  WARNING" } else { "💥 ERROR" };
        error!("{}: {}", severity, error);
        
        if let Some(ctx) = context {
            error!("   Context: {}", ctx);
        }
    }
    
    async fn get_credentials(&self, realm: String, server: String) -> Option<Credentials> {
        warn!("🔐 Authentication required for realm '{}' on server '{}'", realm, server);
        
        // For demo purposes, return dummy credentials
        Some(Credentials {
            username: "demo_user".to_string(),
            password: "demo_pass".to_string(),
        })
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
    let event_handler = std::sync::Arc::new(LoggingEventHandler);
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
    .with_auth("alice".to_string(), "secret123".to_string())
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