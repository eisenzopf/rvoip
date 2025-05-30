use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, debug, warn};

use rvoip_sip_core::Request;
use rvoip_session_core::{SessionManager, SessionId};

use crate::error::{CallCenterError, Result};
use crate::config::CallCenterConfig;
use crate::database::CallCenterDatabase;

/// Main call center orchestrator
/// 
/// This is the central coordination component that handles:
/// - Incoming call routing and distribution
/// - Agent-customer call bridging using session-core APIs
/// - Call lifecycle management
/// - Integration with all call center subsystems
pub struct CallOrchestrator {
    /// Session-core integration for SIP and bridge management
    session_manager: Arc<SessionManager>,
    
    /// Call center database for persistence
    database: CallCenterDatabase,
    
    /// Call center configuration
    config: CallCenterConfig,
    
    /// Active call tracking
    active_calls: HashMap<SessionId, CallInfo>,
    
    /// Active bridge tracking  
    active_bridges: HashMap<String, BridgeInfo>,
}

/// Information about an active call
#[derive(Debug, Clone)]
pub struct CallInfo {
    pub session_id: SessionId,
    pub caller_id: String,
    pub agent_id: Option<String>,
    pub queue_id: Option<String>,
    pub bridge_id: Option<String>,
    pub status: CallStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Call status enumeration
#[derive(Debug, Clone)]
pub enum CallStatus {
    Incoming,
    Queued,
    Ringing,
    Connected,
    OnHold,
    Transferring,
    Terminated,
}

/// Information about an active bridge
#[derive(Debug, Clone)]
pub struct BridgeInfo {
    pub bridge_id: String,
    pub sessions: Vec<SessionId>,
    pub agent_id: Option<String>,
    pub customer_session: Option<SessionId>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl CallOrchestrator {
    /// Create a new call orchestrator
    pub async fn new(
        session_manager: Arc<SessionManager>,
        database: CallCenterDatabase,
        config: CallCenterConfig,
    ) -> Result<Self> {
        info!("🎯 Initializing CallOrchestrator");
        
        Ok(Self {
            session_manager,
            database,
            config,
            active_calls: HashMap::new(),
            active_bridges: HashMap::new(),
        })
    }
    
    /// Handle an incoming customer call
    /// 
    /// This is the main entry point for incoming calls. It will:
    /// 1. Create a session using session-core
    /// 2. Apply routing policies to determine how to handle the call
    /// 3. Either queue the call or directly bridge to an available agent
    pub async fn handle_incoming_call(&self, request: Request) -> Result<SessionId> {
        info!("📞 Handling incoming call from {}", request.uri);
        
        // Step 1: Create session using session-core (FIXED API)
        let session = self.session_manager
            .create_session_for_invite(request.clone(), true)
            .await
            .map_err(CallCenterError::Session)?;
        
        let session_id = session.id.clone();
        
        // Step 2: Extract caller information
        let caller_id = self.extract_caller_id(&request)?;
        
        // Step 3: Create call info and track it
        let call_info = CallInfo {
            session_id: session_id.clone(),
            caller_id: caller_id.clone(),
            agent_id: None,
            queue_id: None,
            bridge_id: None,
            status: CallStatus::Incoming,
            created_at: chrono::Utc::now(),
        };
        
        // TODO: Store call record in database
        debug!("📋 Created call record for session: {}", session_id);
        
        // Step 4: Apply routing logic (TODO: implement routing)
        info!("🎯 Call {} from {} ready for routing", session_id, caller_id);
        
        Ok(session_id)
    }
    
    /// Bridge a customer call with an agent
    /// 
    /// This creates a session-core bridge to connect the customer and agent sessions
    pub async fn bridge_to_agent(&self, customer_session: SessionId, agent_id: String) -> Result<String> {
        info!("🌉 Bridging customer {} to agent {}", customer_session, agent_id);
        
        // TODO: Get agent session (agent must be logged in)
        // TODO: Create bridge using session-core APIs
        // TODO: Add both sessions to the bridge
        // TODO: Update call tracking
        
        warn!("🚧 Bridge implementation not yet complete");
        Err(CallCenterError::orchestration("Bridge implementation pending"))
    }
    
    /// Route a call based on business rules
    /// 
    /// This applies the call center's routing policies to determine
    /// where the call should go (queue, specific agent, etc.)
    pub async fn route_call(&self, call_info: CallInfo) -> Result<RoutingDecision> {
        debug!("🗺️ Routing call {} from {}", call_info.session_id, call_info.caller_id);
        
        // TODO: Implement routing logic
        // TODO: Check agent availability
        // TODO: Apply skill-based routing
        // TODO: Apply business rules (time, geography, etc.)
        
        // For now, default to queuing
        Ok(RoutingDecision::Queue {
            queue_id: "default".to_string(),
            priority: 5,
        })
    }
    
    /// Get orchestrator statistics
    pub fn get_statistics(&self) -> OrchestratorStats {
        OrchestratorStats {
            active_calls: self.active_calls.len(),
            active_bridges: self.active_bridges.len(),
            total_calls_handled: 0, // TODO: implement counter
        }
    }
    
    /// Extract caller ID from SIP request
    fn extract_caller_id(&self, request: &Request) -> Result<String> {
        // TODO: Implement proper SIP header parsing
        // For now, use the request URI
        Ok(request.uri.to_string())
    }
}

/// Routing decision enumeration
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    /// Route directly to a specific agent
    DirectToAgent { agent_id: String },
    
    /// Add to a queue
    Queue { queue_id: String, priority: u8 },
    
    /// Reject the call
    Reject { reason: String },
    
    /// Forward to external destination
    Forward { destination: String },
}

/// Orchestrator statistics
#[derive(Debug, Clone)]
pub struct OrchestratorStats {
    pub active_calls: usize,
    pub active_bridges: usize,
    pub total_calls_handled: u64,
} 