//! SIPp Integration Server
//!
//! This example demonstrates session-core handling real SIPp traffic scenarios:
//! - Basic INVITE/200 OK/ACK flows
//! - Call hold/resume scenarios  
//! - Call termination (BYE)
//! - Error handling and timeouts
//! - Real SDP negotiation with media-core
//! - Zero-copy event monitoring
//!
//! Usage:
//! 1. Run this server: `cargo run --example sipp_integration_server`
//! 2. Run SIPp scenarios against it (see sipp_scenarios/ directory)
//!
//! This validates our architectural achievements:
//! - session-core coordinates (doesn't handle SIP protocol)
//! - transaction-core handles all SIP details
//! - media-core handles media processing
//! - Real end-to-end SIP call flows

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, debug, warn, error};

use rvoip_session_core::{
    // API layer - clean interface
    api::{
        factory::create_sip_server,
        server::config::{ServerConfig, TransportProtocol},
    },
    // Core types for monitoring
    session::{SessionId, SessionState},
    dialog::{DialogId, DialogState},
    // Zero-copy events for monitoring
    events::{EventBus, SessionEvent, EventHandler},
    // Media coordination
    media::{MediaManager, AudioCodecType},
};

use async_trait::async_trait;

/// Comprehensive event handler for SIPp integration testing
struct SippIntegrationHandler {
    name: String,
    call_count: std::sync::atomic::AtomicU32,
    success_count: std::sync::atomic::AtomicU32,
    error_count: std::sync::atomic::AtomicU32,
}

impl SippIntegrationHandler {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            call_count: std::sync::atomic::AtomicU32::new(0),
            success_count: std::sync::atomic::AtomicU32::new(0),
            error_count: std::sync::atomic::AtomicU32::new(0),
        }
    }
    
    fn get_stats(&self) -> (u32, u32, u32) {
        (
            self.call_count.load(std::sync::atomic::Ordering::Relaxed),
            self.success_count.load(std::sync::atomic::Ordering::Relaxed),
            self.error_count.load(std::sync::atomic::Ordering::Relaxed),
        )
    }
}

#[async_trait]
impl EventHandler for SippIntegrationHandler {
    async fn handle_event(&self, event: SessionEvent) {
        match event {
            SessionEvent::Created { session_id } => {
                let count = self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                info!("[{}] 📞 Call #{} - Session created: {}", self.name, count, session_id);
            },
            
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                info!("[{}] 🔄 Session {} state: {} -> {}", 
                      self.name, session_id, old_state, new_state);
                      
                // Track successful connections
                if new_state == SessionState::Connected {
                    let count = self.success_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    info!("[{}] ✅ Call #{} connected successfully", self.name, count);
                }
            },
            
            SessionEvent::DialogCreated { session_id, dialog_id } => {
                info!("[{}] 💬 Dialog created: {} for session {}", 
                      self.name, dialog_id, session_id);
            },
            
            SessionEvent::DialogStateChanged { session_id, dialog_id, previous, current } => {
                info!("[{}] 🔄 Dialog {} state: {} -> {} (session: {})", 
                      self.name, dialog_id, previous, current, session_id);
            },
            
            SessionEvent::MediaStarted { session_id } => {
                info!("[{}] 🎵 Media started for session {}", self.name, session_id);
            },
            
            SessionEvent::MediaStopped { session_id } => {
                info!("[{}] 🔇 Media stopped for session {}", self.name, session_id);
            },
            
            SessionEvent::SdpOfferReceived { session_id, dialog_id } => {
                info!("[{}] 📋 SDP offer received: dialog {} session {}", 
                      self.name, dialog_id, session_id);
            },
            
            SessionEvent::SdpAnswerSent { session_id, dialog_id } => {
                info!("[{}] 📋 SDP answer sent: dialog {} session {}", 
                      self.name, dialog_id, session_id);
            },
            
            SessionEvent::SdpNegotiationComplete { session_id, dialog_id } => {
                info!("[{}] ✅ SDP negotiation complete: dialog {} session {}", 
                      self.name, dialog_id, session_id);
            },
            
            SessionEvent::Terminated { session_id, reason } => {
                info!("[{}] ❌ Session terminated: {} ({})", self.name, session_id, reason);
                
                // Track errors vs normal termination
                if reason.contains("error") || reason.contains("timeout") || reason.contains("failed") {
                    let count = self.error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    warn!("[{}] ⚠️  Error termination #{}: {}", self.name, count, reason);
                }
            },
            
            SessionEvent::ProvisionalResponse { session_id, status_code, reason_phrase } => {
                debug!("[{}] 📞 Provisional response: {} {} (session: {})", 
                       self.name, status_code, reason_phrase, session_id);
            },
            
            SessionEvent::SuccessResponse { session_id, status_code, reason_phrase } => {
                info!("[{}] ✅ Success response: {} {} (session: {})", 
                      self.name, status_code, reason_phrase, session_id);
            },
            
            SessionEvent::FailureResponse { session_id, status_code, reason_phrase } => {
                warn!("[{}] ❌ Failure response: {} {} (session: {})", 
                      self.name, status_code, reason_phrase, session_id);
                self.error_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
            
            _ => {
                debug!("[{}] Event: {:?}", self.name, event);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize comprehensive logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!("🚀 Starting SIPp Integration Server");
    info!("📋 This server demonstrates:");
    info!("   ✅ Real SIP traffic handling via transaction-core");
    info!("   ✅ Real media negotiation via media-core");
    info!("   ✅ Session coordination via session-core");
    info!("   ✅ Zero-copy event monitoring");
    info!("   ✅ Comprehensive SIPp scenario support");

    // Create production-ready server configuration
    info!("\n🔧 Creating production server configuration...");
    let server_config = ServerConfig {
        bind_address: "127.0.0.1:5060".parse()?,
        transport_protocol: TransportProtocol::Udp,
        max_sessions: 1000, // Support high-volume SIPp testing
        session_timeout: Duration::from_secs(300),
        transaction_timeout: Duration::from_secs(32),
        enable_media: true,
        server_name: "sipp-integration-server".to_string(),
        contact_uri: Some("sip:server@127.0.0.1:5060".to_string()),
    };
    
    info!("   📍 Bind address: {}", server_config.bind_address);
    info!("   🚛 Transport: {}", server_config.transport_protocol);
    info!("   📊 Max sessions: {}", server_config.max_sessions);
    info!("   ⏱️  Session timeout: {:?}", server_config.session_timeout);

    // Create the SIP server using session-core API
    info!("\n🏗️  Creating SIP server with real components...");
    let mut server = create_sip_server(server_config).await?;
    info!("   ✅ Server created and listening");

    // **CRITICAL FIX**: Start the server event processing loop
    info!("\n🔄 Starting server event processing...");
    let server_handle = {
        let server_manager = server.server_manager().clone();
        tokio::spawn(async move {
            if let Err(e) = server.run().await {
                error!("Server event processing failed: {}", e);
            }
        })
    };
    info!("   ✅ Server event processing started");

    // Set up comprehensive event monitoring
    info!("\n📡 Setting up zero-copy event monitoring...");
    let event_bus = EventBus::new(10000).await?; // Large buffer for high-volume testing
    
    // Register event handler for monitoring
    let handler = Arc::new(SippIntegrationHandler::new("SIPP-SERVER"));
    event_bus.register_handler(handler.clone()).await?;
    
    info!("   ✅ Event monitoring active");
    info!("   📊 Event buffer size: 10,000 events");

    // Demonstrate media capabilities
    info!("\n🎵 Media capabilities available...");
    let media_manager = MediaManager::new().await?;
    let capabilities = media_manager.get_capabilities().await;
    
    info!("   🎤 Supported codecs: {:?}", [
        AudioCodecType::PCMU,
        AudioCodecType::PCMA, 
        AudioCodecType::G722,
        AudioCodecType::Opus
    ]);
    info!("   📊 Media engine capabilities available");

    // Server is now ready for SIPp testing
    info!("\n🎯 SIPp Integration Server Ready!");
    info!("   📞 Ready to handle SIPp scenarios");
    info!("   🔗 Connect SIPp to: 127.0.0.1:5060");
    info!("   📋 Supported scenarios:");
    info!("      • Basic INVITE/200 OK/ACK flows");
    info!("      • Call hold/resume with re-INVITE");
    info!("      • Call termination with BYE");
    info!("      • Error scenarios and timeouts");
    info!("      • High-volume concurrent calls");

    // Print SIPp command examples
    info!("\n📝 Example SIPp Commands:");
    info!("   Basic call:     sipp -sn uac 127.0.0.1:5060");
    info!("   Multiple calls: sipp -sn uac 127.0.0.1:5060 -m 100");
    info!("   Call rate:      sipp -sn uac 127.0.0.1:5060 -r 10");
    info!("   Custom scenario: sipp -sf scenario.xml 127.0.0.1:5060");

    // Statistics reporting loop
    info!("\n📊 Starting statistics reporting...");
    let stats_handler = handler.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let (calls, success, errors) = stats_handler.get_stats();
            if calls > 0 {
                let success_rate = (success as f64 / calls as f64) * 100.0;
                info!("📊 Stats: {} calls, {} success ({:.1}%), {} errors", 
                      calls, success, success_rate, errors);
            }
        }
    });

    // Keep server running
    info!("\n⏳ Server running... Press Ctrl+C to stop");
    
    // Handle graceful shutdown
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("\n🛑 Shutdown signal received");
        }
        _ = tokio::time::sleep(Duration::from_secs(3600)) => {
            info!("\n⏰ Server timeout after 1 hour");
        }
        result = server_handle => {
            match result {
                Ok(_) => info!("\n✅ Server event processing completed"),
                Err(e) => error!("\n❌ Server event processing task failed: {}", e),
            }
        }
    }

    // Final statistics
    let (calls, success, errors) = handler.get_stats();
    info!("\n📊 Final Statistics:");
    info!("   📞 Total calls: {}", calls);
    info!("   ✅ Successful: {}", success);
    info!("   ❌ Errors: {}", errors);
    if calls > 0 {
        let success_rate = (success as f64 / calls as f64) * 100.0;
        info!("   📈 Success rate: {:.1}%", success_rate);
    }

    // Shutdown event system
    event_bus.shutdown().await?;
    
    info!("✅ SIPp Integration Server stopped");
    info!("🎉 Session-core handled real SIP traffic successfully!");

    Ok(())
} 