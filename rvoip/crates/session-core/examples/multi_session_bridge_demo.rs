//! Multi-Session Bridge Conference Demo
//!
//! This example demonstrates how the bridge infrastructure supports N-way conferencing
//! with 3+ sessions, not just 2-way bridging.
//!
//! **Conference Topology:**
//! ```
//! Client A ──┐
//!            ├── Bridge Server (N-way Conference)
//! Client B ──┤
//!            │
//! Client C ──┘
//! ```
//!
//! **RTP Forwarding Pattern:**
//! - Client A ↔ Client B (via bridge)
//! - Client A ↔ Client C (via bridge) 
//! - Client B ↔ Client C (via bridge)
//! 
//! All audio flows through the bridge server with full-mesh RTP forwarding.

use std::time::Duration;
use std::collections::HashMap;
use std::sync::Arc;
use anyhow::Result;
use tracing::{info, debug, warn, error};
use tokio::signal;
use tokio::time::sleep;
use tokio::sync::{RwLock, mpsc};
use async_trait::async_trait;

use rvoip_session_core::{
    SessionManager, SessionConfig,
    session::bridge::{BridgeConfig, BridgeState, BridgeId},
    events::EventBus,
    media::AudioCodecType,
    SessionId,
    api::server::{IncomingCallNotification, IncomingCallEvent, CallDecision},
};
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_transport::UdpTransport;

/// Conference coordinator that demonstrates N-way bridging
struct ConferenceCoordinator {
    session_manager: Arc<SessionManager>,
    active_conference_bridge: Arc<RwLock<Option<BridgeId>>>,
    conference_sessions: Arc<RwLock<HashMap<SessionId, String>>>,
}

impl ConferenceCoordinator {
    fn new(session_manager: Arc<SessionManager>) -> Self {
        Self {
            session_manager,
            active_conference_bridge: Arc::new(RwLock::new(None)),
            conference_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Handle incoming call - add to conference bridge
    async fn handle_incoming_call(&self, session_id: SessionId, caller_info: String) -> Result<()> {
        info!("📞 Conference call from: {} ({})", caller_info, session_id);
        
        // Add to conference sessions
        {
            let mut sessions = self.conference_sessions.write().await;
            sessions.insert(session_id.clone(), caller_info.clone());
        }
        
        // Get or create conference bridge
        let bridge_id = self.get_or_create_conference_bridge().await?;
        
        // Add session to conference bridge
        self.session_manager.add_session_to_bridge(&bridge_id, &session_id).await
            .map_err(|e| anyhow::anyhow!("Failed to add session to conference: {}", e))?;
        
        // Report conference status
        self.report_conference_status().await?;
        
        Ok(())
    }
    
    /// Get or create the main conference bridge
    async fn get_or_create_conference_bridge(&self) -> Result<BridgeId> {
        let mut bridge_option = self.active_conference_bridge.write().await;
        
        if let Some(bridge_id) = bridge_option.as_ref() {
            // Check if bridge still exists
            if let Ok(_) = self.session_manager.get_bridge_info(bridge_id).await {
                return Ok(bridge_id.clone());
            } else {
                // Bridge was destroyed, clear it
                *bridge_option = None;
            }
        }
        
        // Create new conference bridge
        let config = BridgeConfig {
            max_sessions: 10,  // Support up to 10 participants
            name: Some("Multi-Session Conference".to_string()),
            timeout_secs: Some(600), // 10 minutes
            enable_mixing: true,
        };
        
        let bridge_id = self.session_manager.create_bridge(config).await
            .map_err(|e| anyhow::anyhow!("Failed to create conference bridge: {}", e))?;
        
        *bridge_option = Some(bridge_id.clone());
        
        info!("🎉 Created new conference bridge: {}", bridge_id);
        Ok(bridge_id)
    }
    
    /// Handle session termination - remove from conference
    async fn handle_session_terminated(&self, session_id: &SessionId) -> Result<()> {
        info!("🛑 Session leaving conference: {}", session_id);
        
        // Remove from conference sessions
        let caller_info = {
            let mut sessions = self.conference_sessions.write().await;
            sessions.remove(session_id)
        };
        
        if let Some(caller) = caller_info {
            info!("👋 {} left the conference", caller);
        }
        
        // Remove from bridge if there's an active conference
        if let Some(bridge_id) = self.active_conference_bridge.read().await.as_ref() {
            if let Err(e) = self.session_manager.remove_session_from_bridge(bridge_id, session_id).await {
                warn!("Failed to remove session from conference bridge: {}", e);
            }
        }
        
        // Check if conference is empty
        {
            let sessions = self.conference_sessions.read().await;
            if sessions.is_empty() {
                info!("📭 Conference is now empty");
                
                // Destroy empty conference bridge
                if let Some(bridge_id) = self.active_conference_bridge.write().await.take() {
                    if let Err(e) = self.session_manager.destroy_bridge(&bridge_id).await {
                        warn!("Failed to destroy empty conference bridge: {}", e);
                    } else {
                        info!("🗑️ Destroyed empty conference bridge");
                    }
                }
            }
        }
        
        // Report updated conference status
        self.report_conference_status().await?;
        
        Ok(())
    }
    
    /// Report current conference status
    async fn report_conference_status(&self) -> Result<()> {
        let sessions = self.conference_sessions.read().await;
        let participant_count = sessions.len();
        
        info!("📊 Conference Status: {} participants", participant_count);
        
        if participant_count > 0 {
            info!("👥 Participants:");
            for (session_id, caller_info) in sessions.iter() {
                info!("   • {} ({})", caller_info, session_id);
            }
            
            // Show RTP forwarding topology
            if participant_count >= 2 {
                info!("🎵 RTP Forwarding Topology:");
                let participants: Vec<_> = sessions.values().collect();
                for (i, participant_a) in participants.iter().enumerate() {
                    for participant_b in participants.iter().skip(i + 1) {
                        info!("   {} ↔ {} (bidirectional audio)", participant_a, participant_b);
                    }
                }
                
                let total_pairs = (participant_count * (participant_count - 1)) / 2;
                info!("   📈 Total RTP relay pairs: {}", total_pairs);
            }
        }
        
        Ok(())
    }
    
    /// Get conference statistics
    async fn get_conference_stats(&self) -> (usize, Option<BridgeId>) {
        let sessions = self.conference_sessions.read().await;
        let bridge_id = self.active_conference_bridge.read().await.clone();
        (sessions.len(), bridge_id)
    }
    
    /// Start monitoring conference
    async fn start_monitoring(&self) {
        info!("🎧 Starting conference monitoring");
        
        loop {
            let (participant_count, bridge_id) = self.get_conference_stats().await;
            
            if participant_count > 0 {
                debug!("📊 Conference active: {} participants", participant_count);
                
                if let Some(bridge_id) = bridge_id {
                    if let Ok(bridge_info) = self.session_manager.get_bridge_info(&bridge_id).await {
                        debug!("🌉 Bridge {} state: {:?}, sessions: {}", 
                               bridge_id, bridge_info.state, bridge_info.sessions.len());
                    }
                }
            }
            
            sleep(Duration::from_secs(10)).await;
        }
    }
}

/// Conference call handler for N-way bridging
struct ConferenceCallHandler {
    coordinator: Arc<ConferenceCoordinator>,
}

impl ConferenceCallHandler {
    fn new(coordinator: Arc<ConferenceCoordinator>) -> Self {
        Self { coordinator }
    }
}

#[async_trait]
impl IncomingCallNotification for ConferenceCallHandler {
    async fn on_incoming_call(
        &self,
        event: IncomingCallEvent,
    ) -> CallDecision {
        let caller_info = format!("{}@{}", 
            event.caller_info.from,
            event.source.ip()
        );
        
        info!("📞 Conference server receiving call from {}", caller_info);
        
        // Always accept calls for conference
        let session_id = event.session_id.clone();
        let coordinator = self.coordinator.clone();
        let caller_info_clone = caller_info.clone();
        
        // Handle the conference logic asynchronously
        tokio::spawn(async move {
            if let Err(e) = coordinator.handle_incoming_call(session_id, caller_info_clone).await {
                error!("Failed to handle incoming conference call: {}", e);
            }
        });
        
        CallDecision::Accept
    }
    
    async fn on_call_terminated_by_remote(
        &self,
        session_id: SessionId,
        _call_id: String,
    ) {
        if let Err(e) = self.coordinator.handle_session_terminated(&session_id).await {
            error!("Failed to handle session termination: {}", e);
        }
    }
    
    async fn on_call_ended_by_server(
        &self,
        session_id: SessionId,
        _call_id: String,
    ) {
        if let Err(e) = self.coordinator.handle_session_terminated(&session_id).await {
            error!("Failed to handle session termination: {}", e);
        }
    }
}

async fn create_conference_server() -> Result<(Arc<SessionManager>, Arc<ConferenceCoordinator>)> {
    // Create TransactionManager with real transport
    let (transport, transport_rx) = UdpTransport::bind("127.0.0.1:5060".parse().unwrap(), None).await?;
    let (transaction_manager, event_rx) = TransactionManager::new(
        Arc::new(transport),
        transport_rx,
        Some(1000)
    ).await?;
    let transaction_manager = Arc::new(transaction_manager);
    
    // Create EventBus
    let event_bus = EventBus::new(1000).await
        .expect("Failed to create EventBus");
    
    // Create SessionConfig
    let config = SessionConfig {
        local_signaling_addr: "127.0.0.1:5060".parse().unwrap(),
        local_media_addr: "127.0.0.1:10000".parse().unwrap(),
        supported_codecs: vec![AudioCodecType::PCMU, AudioCodecType::PCMA],
        display_name: Some("Conference Server".to_string()),
        user_agent: "RVOIP-Conference-Server/1.0".to_string(),
        max_duration: 0, // No call duration limit
        max_sessions: Some(100),
    };
    
    // Create SessionManager directly
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager,
        config,
        event_bus,
    ).await?);
    
    // **CRITICAL FIX**: Forward transaction events to session manager
    let session_manager_clone = session_manager.clone();
    tokio::spawn(async move {
        let mut event_rx = event_rx;
        while let Some(transaction_event) = event_rx.recv().await {
            if let Err(e) = session_manager_clone.handle_transaction_event(transaction_event).await {
                error!("Failed to handle transaction event: {}", e);
            }
        }
        warn!("Transaction event forwarding loop terminated");
    });
    
    // Create conference coordinator
    let coordinator = Arc::new(ConferenceCoordinator::new(session_manager.clone()));
    
    Ok((session_manager, coordinator))
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!("🎉 Starting Multi-Session Conference Demo");
    info!("🏛️ This server supports N-way conferencing with 3+ participants");
    info!("📊 Default configuration: up to 10 concurrent participants");
    info!("");
    info!("🔗 Conference Architecture:");
    info!("   • Each participant connects to the conference bridge");
    info!("   • Full-mesh RTP forwarding between all participants");  
    info!("   • N*(N-1)/2 total RTP relay pairs for N participants");
    info!("   • Example: 4 participants = 6 RTP relay pairs");

    // Create the conference server
    let (session_manager, coordinator) = create_conference_server().await?;
    
    // Create the call handler
    let call_handler = Arc::new(ConferenceCallHandler::new(coordinator.clone()));
    
    // Set up the incoming call notification
    session_manager.set_incoming_call_notifier(call_handler).await;
    
    info!("✅ Conference server created successfully");
    info!("📍 Listening on 127.0.0.1:5060");
    info!("");
    info!("🧪 Test with multiple SIPp clients:");
    info!("  📞 Terminal 1: sipp -sn uac 127.0.0.1:5060 -p 5061 -m 1 -d 60000");
    info!("  📞 Terminal 2: sipp -sn uac 127.0.0.1:5060 -p 5062 -m 1 -d 60000");
    info!("  📞 Terminal 3: sipp -sn uac 127.0.0.1:5060 -p 5063 -m 1 -d 60000");
    info!("  🎵 Expected: All participants hear each other in the conference");
    info!("  🛑 Press Ctrl+C to shutdown");
    info!("");

    // Start the coordinator monitoring
    let coordinator_monitor = coordinator.clone();
    let monitoring_task = tokio::spawn(async move {
        coordinator_monitor.start_monitoring().await;
    });

    // Start the session manager
    if let Err(e) = session_manager.start().await {
        error!("Failed to start session manager: {}", e);
        return Err(e.into());
    }

    // Wait for shutdown signal
    match signal::ctrl_c().await {
        Ok(()) => {
            info!("🛑 Shutdown signal received");
        },
        Err(err) => {
            error!("❌ Unable to listen for shutdown signal: {}", err);
        },
    }

    info!("🔄 Shutting down conference server...");
    
    // Cancel monitoring task
    monitoring_task.abort();
    
    // Brief cleanup delay
    sleep(Duration::from_millis(500)).await;
    
    info!("✅ Conference server shutdown complete");
    Ok(())
} 