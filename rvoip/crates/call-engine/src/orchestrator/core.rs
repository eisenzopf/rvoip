use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, debug, warn};

use rvoip_sip_core::Request;
use rvoip_session_core::{
    SessionId, Session, 
    api::{ServerSessionManager, ServerConfig, create_full_server_manager},
    session::bridge::{BridgeId, BridgeConfig, BridgeInfo as SessionBridgeInfo, BridgeEvent}
};
use rvoip_transaction_core::TransactionManager;
use tokio::sync::mpsc;

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
    
    /// **NEW**: Real session-core server manager integration
    server_manager: Arc<ServerSessionManager>,
    
    /// **NEW**: Bridge event receiver for real-time notifications
    bridge_events: Option<mpsc::UnboundedReceiver<BridgeEvent>>,
}

/// Call information for tracking
#[derive(Debug, Clone)]
pub struct CallInfo {
    pub session_id: SessionId,
    pub caller_id: String,
    pub agent_id: Option<String>,
    pub queue_id: Option<String>,
    pub bridge_id: Option<BridgeId>,
    pub status: CallStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Call status enumeration
#[derive(Debug, Clone)]
pub enum CallStatus {
    Incoming,
    Queued,
    Bridged,
    Ended,
}

/// Routing decision enumeration
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    Queue { queue_id: String, priority: u32 },
    Agent { agent_id: String },
    Reject { reason: String },
}

/// Orchestrator statistics
#[derive(Debug, Clone)]
pub struct OrchestratorStats {
    pub active_calls: usize,
    pub active_bridges: usize,
    pub total_calls_handled: u64,
}

impl CallCenterEngine {
    /// **REAL INTEGRATION**: Create call center engine with session-core
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: CallCenterConfig,
        database: CallCenterDatabase,
    ) -> CallCenterResult<Self> {
        tracing::info!("🚀 Creating CallCenterEngine with REAL session-core integration");
        
        // Convert call center config to session-core ServerConfig
        let server_config = ServerConfig {
            server_name: config.general.domain.clone(),
            max_sessions: config.general.max_concurrent_calls,
            // Map additional fields as they become available
            ..Default::default()
        };
        
        // **REAL**: Create session-core server manager
        let server_manager = create_full_server_manager(transaction_manager, server_config)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create server manager: {}", e)))?;
        
        tracing::info!("✅ ServerSessionManager created successfully");
        
        Ok(Self {
            config,
            database,
            server_manager,
            bridge_events: None, // Will be initialized when subscribing to events
        })
    }
    
    /// **REAL IMPLEMENTATION**: Handle incoming customer call using session-core
    /// 
    /// This replaces the Phase 1 stub and actually creates a real SIP session.
    pub async fn handle_incoming_call(&self, request: Request) -> CallCenterResult<SessionId> {
        tracing::info!("📞 Handling incoming call with REAL session-core integration");
        
        // **REAL**: Use session-core to handle the incoming call
        let session = self.server_manager
            .handle_incoming_call(&request)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to handle incoming call: {}", e)))?;
        
        let session_id = session.id.clone();
        tracing::info!("✅ Created real session: {}", session_id);
        
        // TODO Phase 2: Add to call queue, find available agent, etc.
        // For now, just return the real session ID
        
        Ok(session_id)
    }
    
    /// **NEW**: Bridge customer call with agent using session-core bridge API
    /// 
    /// This is the core call center functionality - connecting customer and agent.
    pub async fn bridge_customer_to_agent(
        &self,
        customer_session: SessionId,
        agent_session: SessionId,
    ) -> CallCenterResult<BridgeId> {
        tracing::info!("🌉 Bridging customer {} to agent {} using session-core", customer_session, agent_session);
        
        // **REAL**: Use session-core bridge API
        let bridge_id = self.server_manager
            .bridge_sessions(&customer_session, &agent_session)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to bridge sessions: {}", e)))?;
        
        tracing::info!("✅ Created bridge: {}", bridge_id);
        
        // Record the bridge in our database for monitoring
        // TODO: Add bridge tracking to database schema
        
        Ok(bridge_id)
    }
    
    /// **NEW**: Create a call center conference with multiple participants
    pub async fn create_conference(&self, session_ids: &[SessionId]) -> CallCenterResult<BridgeId> {
        tracing::info!("🎤 Creating conference with {} participants", session_ids.len());
        
        // Create bridge configuration for conference
        let config = BridgeConfig {
            max_sessions: session_ids.len(),
            name: Some(format!("Conference with {} participants", session_ids.len())),
            ..Default::default()
        };
        
        // **REAL**: Create bridge using session-core
        let bridge_id = self.server_manager
            .create_bridge(config)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create conference bridge: {}", e)))?;
        
        // Add all sessions to the bridge
        for session_id in session_ids {
            self.server_manager
                .add_session_to_bridge(&bridge_id, session_id)
                .await
                .map_err(|e| CallCenterError::orchestration(&format!("Failed to add session {} to conference: {}", session_id, e)))?;
        }
        
        tracing::info!("✅ Created conference bridge: {}", bridge_id);
        Ok(bridge_id)
    }
    
    /// **NEW**: Transfer call from one agent to another
    pub async fn transfer_call(
        &self,
        customer_session: SessionId,
        from_agent: SessionId,
        to_agent: SessionId,
    ) -> CallCenterResult<BridgeId> {
        tracing::info!("🔄 Transferring call from agent {} to agent {}", from_agent, to_agent);
        
        // Get current bridge if any
        if let Some(current_bridge) = self.server_manager.get_session_bridge(&customer_session).await {
            // Remove from current bridge
            self.server_manager
                .remove_session_from_bridge(&current_bridge, &from_agent)
                .await
                .map_err(|e| CallCenterError::orchestration(&format!("Failed to remove agent from bridge: {}", e)))?;
        }
        
        // Create new bridge with customer and new agent
        let new_bridge = self.bridge_customer_to_agent(customer_session, to_agent).await?;
        
        tracing::info!("✅ Call transferred successfully");
        Ok(new_bridge)
    }
    
    /// **NEW**: Subscribe to bridge events for real-time monitoring
    pub async fn start_bridge_monitoring(&mut self) -> CallCenterResult<()> {
        tracing::info!("👁️ Starting bridge event monitoring");
        
        // **REAL**: Subscribe to session-core bridge events
        let event_receiver = self.server_manager.subscribe_to_bridge_events().await;
        self.bridge_events = Some(event_receiver);
        
        // TODO: Process events in background task
        // tokio::spawn(async move {
        //     while let Some(event) = event_receiver.recv().await {
        //         // Handle bridge events for monitoring, metrics, etc.
        //     }
        // });
        
        Ok(())
    }
    
    /// **NEW**: Get real-time bridge information for monitoring
    pub async fn get_bridge_info(&self, bridge_id: &BridgeId) -> CallCenterResult<SessionBridgeInfo> {
        self.server_manager
            .get_bridge_info(bridge_id)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to get bridge info: {}", e)))
    }
    
    /// **NEW**: List all active bridges for dashboard
    pub async fn list_active_bridges(&self) -> Vec<SessionBridgeInfo> {
        self.server_manager.list_bridges().await
    }
    
    /// **NEW**: Register an agent with session-core
    pub async fn register_agent_with_session_core(&self, agent: &Agent) -> CallCenterResult<()> {
        tracing::info!("👤 Registering agent {} with session-core: {}", agent.id, agent.sip_uri);
        
        // Create user registration for session-core
        let registration = rvoip_session_core::api::server::UserRegistration {
            user_uri: agent.sip_uri.clone(),
            contact_uri: agent.sip_uri.clone(), // Same as user URI for now
            expires: std::time::SystemTime::now() + std::time::Duration::from_secs(3600), // 1 hour
            user_agent: Some(format!("CallEngine-Agent-{}", agent.id)),
        };
        
        // **REAL**: Register with session-core
        self.server_manager
            .register_user(registration)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to register agent: {}", e)))?;
        
        tracing::info!("✅ Agent {} registered with session-core", agent.id);
        Ok(())
    }
    
    /// **NEW**: Get server statistics from session-core
    pub async fn get_server_statistics(&self) -> CallCenterResult<rvoip_session_core::api::server::ServerStats> {
        Ok(self.server_manager.get_server_stats().await)
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