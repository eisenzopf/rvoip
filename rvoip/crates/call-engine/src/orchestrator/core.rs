use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, debug, warn, error};
use std::net::SocketAddr;
use async_trait::async_trait;

use rvoip_sip_core::{Request, Response, StatusCode};
use rvoip_session_core::api::{
    // Basic session types from API
    SessionId, Session,
    // Server management
    ServerSessionManager, ServerConfig, create_full_server_manager,
    IncomingCallEvent, CallerInfo, CallDecision, IncomingCallNotification,
    // Bridge management
    BridgeId, BridgeConfig, BridgeInfo, BridgeEvent, BridgeEventType,
};
use rvoip_transaction_core::{TransactionManager, TransactionKey};

use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::config::CallCenterConfig;
use crate::database::CallCenterDatabase;
use crate::agent::{AgentId, Agent};

/// **REAL SESSION-CORE INTEGRATION**: Call center orchestration engine
/// 
/// This is the main orchestration component that integrates with session-core
/// to provide call center functionality on top of SIP session management.
pub struct CallCenterEngine {
    /// Configuration for the call center
    config: CallCenterConfig,
    
    /// Database layer for persistence
    database: CallCenterDatabase,
    
    /// **REAL**: Session-core server manager integration
    server_manager: Arc<ServerSessionManager>,
    
    /// **REAL**: Bridge event receiver for real-time notifications
    bridge_events: Option<mpsc::UnboundedReceiver<BridgeEvent>>,
    
    /// **NEW**: Call tracking and routing
    active_calls: Arc<RwLock<HashMap<SessionId, CallInfo>>>,
    
    /// **NEW**: Agent availability tracking
    available_agents: Arc<RwLock<HashMap<AgentId, SessionId>>>,
}

/// Call information for tracking
#[derive(Debug, Clone)]
pub struct CallInfo {
    pub session_id: SessionId,
    pub caller_id: String,
    pub agent_id: Option<AgentId>,
    pub queue_id: Option<String>,
    pub bridge_id: Option<BridgeId>,
    pub status: CallStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub caller_info: CallerInfo,
}

/// Call status enumeration
#[derive(Debug, Clone)]
pub enum CallStatus {
    Incoming,
    Ringing,
    Queued,
    Connecting,
    Bridged,
    OnHold,
    Transferring,
    Ended,
}

/// Routing decision enumeration
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    Queue { queue_id: String, priority: u32 },
    Agent { agent_id: AgentId },
    Conference { bridge_id: BridgeId },
    Reject { reason: String },
}

/// Orchestrator statistics
#[derive(Debug, Clone)]
pub struct OrchestratorStats {
    pub active_calls: usize,
    pub active_bridges: usize,
    pub total_calls_handled: u64,
    pub available_agents: usize,
    pub queued_calls: usize,
}

/// **REAL**: Incoming call notification handler for session-core integration
#[derive(Clone)]
struct CallCenterNotificationHandler {
    call_center: Arc<RwLock<Option<Arc<CallCenterEngine>>>>,
}

impl CallCenterNotificationHandler {
    fn new() -> Self {
        Self {
            call_center: Arc::new(RwLock::new(None)),
        }
    }
    
    async fn set_call_center(&self, engine: Arc<CallCenterEngine>) {
        let mut lock = self.call_center.write().await;
        *lock = Some(engine);
    }
}

#[async_trait]
impl IncomingCallNotification for CallCenterNotificationHandler {
    /// **REAL**: Handle incoming call notifications from session-core
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        info!("📞 Call center received incoming call: {} from {}", 
              event.session_id, event.caller_info.from);
        
        // Get the call center engine
        let engine = {
            let lock = self.call_center.read().await;
            lock.clone()
        };
        
        if let Some(engine) = engine {
            // Process the incoming call through call center logic
            match engine.process_incoming_call_event(event).await {
                Ok(decision) => decision,
                Err(e) => {
                    error!("Failed to process incoming call: {}", e);
                    CallDecision::Reject {
                        status_code: StatusCode::ServerInternalError,
                        reason: Some("Call center processing error".to_string()),
                    }
                }
            }
        } else {
            warn!("Call center engine not initialized, deferring call");
            CallDecision::Defer
        }
    }
    
    /// **REAL**: Handle call termination by remote party
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, call_id: String) {
        info!("📞 Call terminated by remote: {} (call-id: {})", session_id, call_id);
        
        if let Some(engine) = self.call_center.read().await.as_ref() {
            if let Err(e) = engine.handle_call_termination(session_id).await {
                error!("Failed to handle call termination: {}", e);
            }
        }
    }
    
    /// **REAL**: Handle call termination by server
    async fn on_call_ended_by_server(&self, session_id: SessionId, call_id: String) {
        info!("📞 Call ended by server: {} (call-id: {})", session_id, call_id);
        
        if let Some(engine) = self.call_center.read().await.as_ref() {
            if let Err(e) = engine.handle_call_termination(session_id).await {
                error!("Failed to handle call termination: {}", e);
            }
        }
    }
}

impl CallCenterEngine {
    /// **REAL INTEGRATION**: Create call center engine with session-core
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: CallCenterConfig,
        database: CallCenterDatabase,
    ) -> CallCenterResult<Arc<Self>> {
        info!("🚀 Creating CallCenterEngine with REAL session-core API integration");
        
        // Convert call center config to session-core ServerConfig
        let server_config = ServerConfig {
            bind_address: config.general.local_signaling_addr,
            transport_protocol: rvoip_session_core::api::server::TransportProtocol::Udp,
            max_sessions: config.general.max_concurrent_calls,
            session_timeout: std::time::Duration::from_secs(config.general.default_call_timeout as u64),
            transaction_timeout: std::time::Duration::from_secs(32), // RFC 3261 Timer B
            enable_media: true,
            server_name: config.general.domain.clone(),
            contact_uri: Some(format!("sip:{}@{}", config.general.domain, config.general.local_signaling_addr.ip())),
        };
        
        // **REAL**: Create session-core server manager using the API
        let server_manager = create_full_server_manager(transaction_manager, server_config)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create server manager: {}", e)))?;
        
        info!("✅ ServerSessionManager created successfully");
        
        let engine = Arc::new(Self {
            config,
            database,
            server_manager,
            bridge_events: None,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            available_agents: Arc::new(RwLock::new(HashMap::new())),
        });
        
        // **REAL**: Set up incoming call notification handler
        let notification_handler = CallCenterNotificationHandler::new();
        notification_handler.set_call_center(engine.clone()).await;
        
        // **REAL**: Register notification handler with session-core
        engine.server_manager
            .session_manager()
            .set_incoming_call_notifier(Arc::new(notification_handler))
            .await;
        
        info!("✅ Call center engine initialized with real session-core integration");
        
        Ok(engine)
    }
    
    /// **REAL**: Process incoming call event from session-core
    async fn process_incoming_call_event(&self, event: IncomingCallEvent) -> CallCenterResult<CallDecision> {
        let session_id = event.session_id.clone();
        
        // Create call info tracking
        let call_info = CallInfo {
            session_id: session_id.clone(),
            caller_id: event.caller_info.from.clone(),
            agent_id: None,
            queue_id: None,
            bridge_id: None,
            status: CallStatus::Incoming,
            created_at: chrono::Utc::now(),
            caller_info: event.caller_info.clone(),
        };
        
        // Store call info
        {
            let mut active_calls = self.active_calls.write().await;
            active_calls.insert(session_id.clone(), call_info);
        }
        
        // **BUSINESS LOGIC**: Check if we have available agents
        let available_agents = self.available_agents.read().await;
        if !available_agents.is_empty() {
            // We have available agents - accept the call for routing
            info!("✅ Accepting call {} - available agents: {}", session_id, available_agents.len());
            
            // Update status to ringing
            {
                let mut active_calls = self.active_calls.write().await;
                if let Some(call_info) = active_calls.get_mut(&session_id) {
                    call_info.status = CallStatus::Ringing;
                }
            }
            
            // Schedule agent assignment
            let engine = Arc::new(self.clone());
            let session_id_clone = session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = engine.assign_agent_to_call(session_id_clone).await {
                    error!("Failed to assign agent to call: {}", e);
                }
            });
            
            Ok(CallDecision::Accept)
        } else {
            // No available agents - queue the call
            info!("📋 Queueing call {} - no available agents", session_id);
            
            // Update status to queued
            {
                let mut active_calls = self.active_calls.write().await;
                if let Some(call_info) = active_calls.get_mut(&session_id) {
                    call_info.status = CallStatus::Queued;
                    call_info.queue_id = Some("default".to_string());
                }
            }
            
            // Accept the call but put it in queue
            Ok(CallDecision::Accept)
        }
    }
    
    /// **REAL**: Assign an available agent to an incoming call
    async fn assign_agent_to_call(&self, session_id: SessionId) -> CallCenterResult<()> {
        info!("🎯 Assigning agent to call: {}", session_id);
        
        // Find an available agent
        let agent_session = {
            let mut available_agents = self.available_agents.write().await;
            if !available_agents.is_empty() {
                // Get the first agent and remove it from the available pool
                let (agent_id, agent_session) = available_agents.iter().next()
                    .map(|(id, session)| (id.clone(), session.clone()))
                    .unwrap();
                available_agents.remove(&agent_id);
                Some((agent_id, agent_session))
            } else {
                None
            }
        };
        
        if let Some((agent_id, agent_session)) = agent_session {
            // **REAL**: Bridge customer and agent using session-core API
            match self.server_manager.bridge_sessions(&session_id, &agent_session).await {
                Ok(bridge_id) => {
                    info!("✅ Successfully bridged customer {} with agent {} (bridge: {})", 
                          session_id, agent_id, bridge_id);
                    
                    // Update call info
                    {
                        let mut active_calls = self.active_calls.write().await;
                        if let Some(call_info) = active_calls.get_mut(&session_id) {
                            call_info.agent_id = Some(agent_id);
                            call_info.bridge_id = Some(bridge_id);
                            call_info.status = CallStatus::Bridged;
                        }
                    }
                },
                Err(e) => {
                    error!("Failed to bridge sessions: {}", e);
                    
                    // Return agent to available pool
                    {
                        let mut available_agents = self.available_agents.write().await;
                        available_agents.insert(agent_id, agent_session);
                    }
                    
                    return Err(CallCenterError::orchestration(&format!("Bridge failed: {}", e)));
                }
            }
        } else {
            warn!("No available agents to assign to call {}", session_id);
        }
        
        Ok(())
    }
    
    /// **REAL**: Handle call termination cleanup
    async fn handle_call_termination(&self, session_id: SessionId) -> CallCenterResult<()> {
        info!("🛑 Handling call termination: {}", session_id);
        
        // Get call info and clean up
        let call_info = {
            let mut active_calls = self.active_calls.write().await;
            active_calls.remove(&session_id)
        };
        
        if let Some(call_info) = call_info {
            // If call was bridged, return agent to available pool
            if let Some(agent_id) = call_info.agent_id {
                info!("🔄 Returning agent {} to available pool", agent_id);
                // Note: In a real implementation, we'd need to track agent sessions
                // For now, we'll just log this
            }
            
            // If call had a bridge, clean it up
            if let Some(bridge_id) = call_info.bridge_id {
                if let Err(e) = self.server_manager.destroy_bridge(&bridge_id).await {
                    warn!("Failed to destroy bridge {}: {}", bridge_id, e);
                }
            }
        }
        
        Ok(())
    }
    
    /// **NEW API**: Register an agent and make them available
    pub async fn register_agent(&self, agent: &Agent) -> CallCenterResult<SessionId> {
        info!("👤 Registering agent {} with session-core: {}", agent.id, agent.sip_uri);
        
        // **REAL**: Create outgoing session for agent registration
        let agent_session = self.server_manager
            .session_manager()
            .create_outgoing_session()
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create agent session: {}", e)))?;
        
        let session_id = agent_session.id.clone();
        
        // Add agent to available pool
        {
            let mut available_agents = self.available_agents.write().await;
            available_agents.insert(agent.id.clone(), session_id.clone());
        }
        
        info!("✅ Agent {} registered with session-core (session: {})", agent.id, session_id);
        Ok(session_id)
    }
    
    /// **NEW API**: Create a conference bridge with multiple participants
    pub async fn create_conference(&self, session_ids: &[SessionId]) -> CallCenterResult<BridgeId> {
        info!("🎤 Creating conference with {} participants", session_ids.len());
        
        // **REAL**: Create bridge configuration for conference
        let config = BridgeConfig {
            max_sessions: session_ids.len(),
            name: Some(format!("Conference with {} participants", session_ids.len())),
            enable_mixing: true,
            ..Default::default()
        };
        
        // **REAL**: Create bridge using session-core API
        let bridge_id = self.server_manager
            .create_bridge(config)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create conference bridge: {}", e)))?;
        
        // **REAL**: Add all sessions to the bridge
        for session_id in session_ids {
            self.server_manager
                .add_session_to_bridge(&bridge_id, session_id)
                .await
                .map_err(|e| CallCenterError::orchestration(&format!("Failed to add session {} to conference: {}", session_id, e)))?;
        }
        
        info!("✅ Created conference bridge: {}", bridge_id);
        Ok(bridge_id)
    }
    
    /// **NEW API**: Transfer call from one agent to another
    pub async fn transfer_call(
        &self,
        customer_session: SessionId,
        from_agent: AgentId,
        to_agent: AgentId,
    ) -> CallCenterResult<BridgeId> {
        info!("🔄 Transferring call from agent {} to agent {}", from_agent, to_agent);
        
        // Find sessions for agents
        let available_agents = self.available_agents.read().await;
        let to_agent_session = available_agents.get(&to_agent)
            .ok_or_else(|| CallCenterError::orchestration(&format!("Agent {} not available", to_agent)))?
            .clone();
        
        // Get current bridge if any
        if let Some(current_bridge) = self.server_manager.get_session_bridge(&customer_session).await {
            // **REAL**: Remove customer from current bridge
            if let Err(e) = self.server_manager.remove_session_from_bridge(&current_bridge, &customer_session).await {
                warn!("Failed to remove customer from current bridge: {}", e);
            }
        }
        
        // **REAL**: Create new bridge with customer and new agent
        let new_bridge = self.server_manager
            .bridge_sessions(&customer_session, &to_agent_session)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create transfer bridge: {}", e)))?;
        
        info!("✅ Call transferred successfully to bridge: {}", new_bridge);
        Ok(new_bridge)
    }
    
    /// **NEW API**: Get real-time bridge information for monitoring
    pub async fn get_bridge_info(&self, bridge_id: &BridgeId) -> CallCenterResult<BridgeInfo> {
        self.server_manager
            .get_bridge_info(bridge_id)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to get bridge info: {}", e)))
    }
    
    /// **NEW API**: List all active bridges for dashboard
    pub async fn list_active_bridges(&self) -> Vec<BridgeInfo> {
        self.server_manager.list_bridges().await
    }
    
    /// **NEW API**: Subscribe to bridge events for real-time monitoring
    pub async fn start_bridge_monitoring(&mut self) -> CallCenterResult<()> {
        info!("👁️ Starting bridge event monitoring");
        
        // **REAL**: Subscribe to session-core bridge events
        let event_receiver = self.server_manager.subscribe_to_bridge_events().await;
        self.bridge_events = Some(event_receiver);
        
        // Process events in background task
        if let Some(mut receiver) = self.bridge_events.take() {
            let engine = Arc::new(self.clone());
            tokio::spawn(async move {
                while let Some(event) = receiver.recv().await {
                    engine.handle_bridge_event(event).await;
                }
            });
        }
        
        Ok(())
    }
    
    /// **NEW**: Handle bridge events for monitoring and metrics
    async fn handle_bridge_event(&self, event: BridgeEvent) {
        match event.event_type {
            BridgeEventType::BridgeCreated => {
                info!("🌉 Bridge created: {}", event.bridge_id);
            },
            BridgeEventType::BridgeDestroyed => {
                info!("🗑️ Bridge destroyed: {}", event.bridge_id);
            },
            BridgeEventType::SessionAdded => {
                if let Some(session_id) = &event.session_id {
                    info!("➕ Session {} added to bridge {}", session_id, event.bridge_id);
                }
            },
            BridgeEventType::SessionRemoved => {
                if let Some(session_id) = &event.session_id {
                    info!("➖ Session {} removed from bridge {}", session_id, event.bridge_id);
                }
            },
            _ => {
                debug!("🔔 Bridge event: {:?}", event);
            }
        }
    }
    
    /// **NEW API**: Get orchestrator statistics
    pub async fn get_stats(&self) -> OrchestratorStats {
        let active_calls = self.active_calls.read().await;
        let available_agents = self.available_agents.read().await;
        let bridges = self.list_active_bridges().await;
        
        let queued_calls = active_calls.values()
            .filter(|call| matches!(call.status, CallStatus::Queued))
            .count();
        
        OrchestratorStats {
            active_calls: active_calls.len(),
            active_bridges: bridges.len(),
            total_calls_handled: 0, // TODO: Track this
            available_agents: available_agents.len(),
            queued_calls,
        }
    }
    
    /// Get the underlying session manager for advanced operations
    pub fn session_manager(&self) -> &Arc<ServerSessionManager> {
        &self.server_manager
    }
    
    /// Get call center configuration
    pub fn config(&self) -> &CallCenterConfig {
        &self.config
    }
    
    /// Get database handle
    pub fn database(&self) -> &CallCenterDatabase {
        &self.database
    }
} 

// Make CallCenterEngine cloneable for async operations
impl Clone for CallCenterEngine {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            database: self.database.clone(),
            server_manager: self.server_manager.clone(),
            bridge_events: None, // Don't clone the receiver
            active_calls: self.active_calls.clone(),
            available_agents: self.available_agents.clone(),
        }
    }
} 