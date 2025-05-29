//! Bridge Server Example
//!
//! This example demonstrates a SIP server that automatically bridges incoming calls together.
//! When two calls are active, they are automatically bridged for audio exchange.
//!
//! **Bridge Features:**
//! - Automatic call acceptance with bridge logic
//! - Real session bridging using session-core bridge infrastructure 
//! - RTP forwarding between bridged sessions
//! - Complete call lifecycle with bridge management
//! - Bridge event monitoring and logging
//!
//! **Test Scenario:**
//! 1. Client A calls server → Call accepted, waiting for bridge partner
//! 2. Client B calls server → Call accepted, automatically bridged with Client A
//! 3. Audio flows: Client A ↔ Server ↔ Client B (bidirectional RTP forwarding)
//! 4. Either client hangs up → Bridge destroyed, other call continues alone
//! 5. Remaining client hangs up → All calls terminated
//!
//! Usage:
//!   cargo run --example bridge_server
//!
//! Test with SIPp:
//!   # Terminal 1: Start server
//!   cargo run --example bridge_server
//!   
//!   # Terminal 2: First client (will wait for bridge partner)
//!   sipp -sn uac 127.0.0.1:5060 -m 1 -d 30000 -ap test_audio.wav
//!   
//!   # Terminal 3: Second client (will be bridged with first)
//!   sipp -sn uac 127.0.0.1:5061 -p 5062 -m 1 -d 30000 -ap test_audio.wav
//!
//! Expected result: Audio from Client A flows to Client B and vice versa through the bridge.

use std::time::Duration;
use std::collections::HashMap;
use std::sync::Arc;
use anyhow::Result;
use tracing::{info, debug, warn, error};
use tokio::signal;
use tokio::time::{sleep, timeout};
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

/// Bridge coordinator that manages automatic call bridging
struct BridgeCoordinator {
    session_manager: Arc<SessionManager>,
    active_sessions: Arc<RwLock<HashMap<SessionId, SessionInfo>>>,
    active_bridges: Arc<RwLock<HashMap<BridgeId, BridgeInfo>>>,
}

/// Information about an active session
#[derive(Debug, Clone)]
struct SessionInfo {
    session_id: SessionId,
    caller_info: String,
    bridge_id: Option<BridgeId>,
}

/// Information about an active bridge
#[derive(Debug, Clone)]
struct BridgeInfo {
    bridge_id: BridgeId,
    session_a: SessionId,
    session_b: SessionId,
    created_at: std::time::Instant,
}

impl BridgeCoordinator {
    fn new(session_manager: Arc<SessionManager>) -> Self {
        Self {
            session_manager,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            active_bridges: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Handle a new incoming call - automatically bridge if possible
    async fn handle_incoming_call(&self, session_id: SessionId, caller_info: String) -> Result<()> {
        info!("📞 New incoming call: {} from {}", session_id, caller_info);
        
        // Add to active sessions
        {
            let mut sessions = self.active_sessions.write().await;
            sessions.insert(session_id.clone(), SessionInfo {
                session_id: session_id.clone(),
                caller_info: caller_info.clone(),
                bridge_id: None,
            });
        }
        
        // Check if we can bridge with another session
        if let Some(partner_session_id) = self.find_bridge_partner(&session_id).await {
            info!("🌉 Found bridge partner: {} ↔ {}", session_id, partner_session_id);
            self.create_bridge(&session_id, &partner_session_id).await?;
        } else {
            info!("⏳ Session {} waiting for bridge partner", session_id);
        }
        
        Ok(())
    }
    
    /// Find a suitable partner for bridging
    async fn find_bridge_partner(&self, new_session_id: &SessionId) -> Option<SessionId> {
        let sessions = self.active_sessions.read().await;
        
        // Find the first session that isn't bridged and isn't the new session
        for (session_id, session_info) in sessions.iter() {
            if session_id != new_session_id && session_info.bridge_id.is_none() {
                return Some(session_id.clone());
            }
        }
        
        None
    }
    
    /// Create a bridge between two sessions
    async fn create_bridge(&self, session_a: &SessionId, session_b: &SessionId) -> Result<()> {
        info!("🌉 Creating bridge between {} and {}", session_a, session_b);
        
        // Create bridge configuration
        let bridge_config = BridgeConfig {
            max_sessions: 2,
            name: Some(format!("Auto-Bridge-{}-{}", 
                session_a.to_string().split('-').next().unwrap_or("unknown"),
                session_b.to_string().split('-').next().unwrap_or("unknown")
            )),
            timeout_secs: Some(300), // 5 minutes
            enable_mixing: true,
        };
        
        // Create the bridge
        let bridge_id = self.session_manager.create_bridge(bridge_config).await?;
        info!("✅ Created bridge: {}", bridge_id);
        
        // Add both sessions to the bridge
        self.session_manager.add_session_to_bridge(&bridge_id, session_a).await?;
        info!("✅ Added session {} to bridge", session_a);
        
        self.session_manager.add_session_to_bridge(&bridge_id, session_b).await?;
        info!("✅ Added session {} to bridge", session_b);
        
        // Update session info
        {
            let mut sessions = self.active_sessions.write().await;
            if let Some(session_info_a) = sessions.get_mut(session_a) {
                session_info_a.bridge_id = Some(bridge_id.clone());
            }
            if let Some(session_info_b) = sessions.get_mut(session_b) {
                session_info_b.bridge_id = Some(bridge_id.clone());
            }
        }
        
        // Track the bridge
        {
            let mut bridges = self.active_bridges.write().await;
            bridges.insert(bridge_id.clone(), BridgeInfo {
                bridge_id: bridge_id.clone(),
                session_a: session_a.clone(),
                session_b: session_b.clone(),
                created_at: std::time::Instant::now(),
            });
        }
        
        // Verify bridge creation
        let bridge_info = self.session_manager.get_bridge_info(&bridge_id).await?;
        info!("🎉 Bridge created successfully!");
        info!("   Bridge ID: {}", bridge_id);
        info!("   Sessions: {:?}", bridge_info.sessions);
        info!("   State: {:?}", bridge_info.state);
        info!("   🎵 Audio should now flow between sessions!");
        
        Ok(())
    }
    
    /// Handle session termination - clean up bridges
    async fn handle_session_terminated(&self, session_id: &SessionId) -> Result<()> {
        info!("🛑 Session terminated: {}", session_id);
        
        // Find if this session was in a bridge
        let bridge_to_destroy = {
            let sessions = self.active_sessions.read().await;
            sessions.get(session_id).and_then(|info| info.bridge_id.clone())
        };
        
        // Remove session from tracking
        {
            let mut sessions = self.active_sessions.write().await;
            sessions.remove(session_id);
        }
        
        // Destroy bridge if it existed
        if let Some(bridge_id) = bridge_to_destroy {
            info!("🌉 Destroying bridge due to session termination: {}", bridge_id);
            
            // Remove the bridge
            if let Err(e) = self.session_manager.destroy_bridge(&bridge_id).await {
                warn!("Failed to destroy bridge {}: {}", bridge_id, e);
            } else {
                info!("✅ Bridge {} destroyed successfully", bridge_id);
            }
            
            // Clean up bridge tracking
            {
                let mut bridges = self.active_bridges.write().await;
                bridges.remove(&bridge_id);
            }
        }
        
        Ok(())
    }
    
    /// Get status information about active bridges
    async fn get_status(&self) -> (usize, usize) {
        let sessions = self.active_sessions.read().await;
        let bridges = self.active_bridges.read().await;
        (sessions.len(), bridges.len())
    }
    
    /// Start the bridge coordinator monitoring loop
    async fn start_monitoring(&self) {
        info!("🎧 Starting bridge coordinator monitoring");
        
        loop {
            let (session_count, bridge_count) = self.get_status().await;
            
            if session_count > 0 || bridge_count > 0 {
                debug!("📊 Bridge Status: {} sessions, {} bridges", session_count, bridge_count);
                
                // Get bridge statistics
                let stats = self.session_manager.get_bridge_statistics().await;
                for (bridge_id, stat) in stats {
                    debug!("🌉 Bridge {}: {} sessions", bridge_id, stat.session_count);
                }
            }
            
            sleep(Duration::from_secs(5)).await;
        }
    }
}

/// Custom incoming call handler that implements automatic bridging
struct BridgeCallHandler {
    coordinator: Arc<BridgeCoordinator>,
}

impl BridgeCallHandler {
    fn new(coordinator: Arc<BridgeCoordinator>) -> Self {
        Self { coordinator }
    }
}

#[async_trait]
impl IncomingCallNotification for BridgeCallHandler {
    async fn on_incoming_call(
        &self,
        event: IncomingCallEvent,
    ) -> CallDecision {
        let caller_info = format!("{}@{}", 
            event.caller_info.from,
            event.source.ip()
        );
        
        info!("📞 Bridge server receiving call from {}", caller_info);
        
        // Always accept calls for bridging
        let session_id = event.session_id.clone();
        let coordinator = self.coordinator.clone();
        let caller_info_clone = caller_info.clone();
        
        // Handle the bridge logic asynchronously
        tokio::spawn(async move {
            if let Err(e) = coordinator.handle_incoming_call(session_id, caller_info_clone).await {
                error!("Failed to handle incoming call for bridging: {}", e);
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

async fn create_bridge_server() -> Result<(Arc<SessionManager>, Arc<BridgeCoordinator>)> {
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
        display_name: Some("Bridge Server".to_string()),
        user_agent: "RVOIP-Bridge-Server/1.0".to_string(),
        max_duration: 0, // No call duration limit for bridging
        max_sessions: Some(100),
    };
    
    // Create SessionManager
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
    
    // Create bridge coordinator
    let coordinator = Arc::new(BridgeCoordinator::new(session_manager.clone()));
    
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

    info!("🌉 Starting Bridge Server Example");
    info!("📞 This server automatically bridges incoming calls together");
    info!("🎯 Call flow: Call 1 accepted → Call 2 accepted → Automatic bridge → Audio exchange");

    // Create the bridge server
    let (session_manager, coordinator) = create_bridge_server().await?;
    
    // Create the call handler
    let call_handler = Arc::new(BridgeCallHandler::new(coordinator.clone()));
    
    // Set up the incoming call notification
    session_manager.set_incoming_call_notifier(call_handler).await;
    
    info!("✅ Bridge server created successfully");
    info!("📍 Listening on 127.0.0.1:5060");
    info!("");
    info!("🧪 Test with SIPp:");
    info!("  📞 Terminal 1: sipp -sn uac 127.0.0.1:5060 -m 1 -d 30000");
    info!("  📞 Terminal 2: sipp -sn uac 127.0.0.1:5060 -p 5062 -m 1 -d 30000");
    info!("  🎵 Expected: Calls will be automatically bridged for audio exchange");
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

    info!("🔄 Shutting down bridge server...");
    
    // Cancel monitoring task
    monitoring_task.abort();
    
    // Brief cleanup delay
    sleep(Duration::from_millis(500)).await;
    
    info!("✅ Bridge server shutdown complete");
    Ok(())
} 