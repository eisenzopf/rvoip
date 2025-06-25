//! Core call center engine
//!
//! This module contains the main CallCenterEngine struct that coordinates
//! all call center operations through session-core integration.

use std::sync::Arc;
use std::collections::HashMap;
use dashmap::DashMap;
use tokio::sync::{mpsc, RwLock, Mutex};
use tracing::{info, error, warn};

use rvoip_session_core::{
    SessionCoordinator, SessionManagerBuilder, SessionId, BridgeEvent, CallState,
    MediaQualityAlertLevel, MediaFlowDirection, WarningCategory, IncomingCall,
    SessionControl
};
use rvoip_session_core::prelude::SessionEvent;

use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::config::CallCenterConfig;
use crate::database::CallCenterDatabase;
use crate::agent::{Agent, AgentId, AgentRegistry, AgentStatus, SipRegistrar};
use crate::queue::{CallQueue, QueueManager};
use crate::routing::RoutingEngine;

use super::types::{CallInfo, AgentInfo, RoutingStats, OrchestratorStats, CallStatus, RoutingDecision};
use super::handler::CallCenterCallHandler;

/// Call center orchestration engine
/// 
/// This is the main orchestration component that integrates with session-core
/// to provide call center functionality on top of SIP session management.
pub struct CallCenterEngine {
    /// Configuration for the call center
    pub(super) config: CallCenterConfig,
    
    /// Database layer for persistence
    pub(super) database: CallCenterDatabase,
    
    /// Session-core coordinator integration
    pub(super) session_coordinator: Option<Arc<SessionCoordinator>>,
    
    /// Queue manager for call queuing and routing
    pub(super) queue_manager: Arc<RwLock<QueueManager>>,
    
    /// Bridge event receiver for real-time notifications
    pub(super) bridge_events: Option<mpsc::UnboundedReceiver<BridgeEvent>>,
    
    /// Call tracking and routing with detailed info
    pub(super) active_calls: Arc<RwLock<HashMap<SessionId, CallInfo>>>,
    
    /// Agent availability and skill tracking
    pub(super) available_agents: Arc<DashMap<AgentId, AgentInfo>>,
    
    /// Call routing statistics and metrics
    pub(super) routing_stats: Arc<RwLock<RoutingStats>>,
    
    /// Agent registry
    pub(crate) agent_registry: Arc<Mutex<AgentRegistry>>,
    
    /// SIP Registrar for handling agent registrations
    pub(crate) sip_registrar: Arc<Mutex<SipRegistrar>>,
}

impl CallCenterEngine {
    /// Create call center engine with session-core integration
    pub async fn new(
        config: CallCenterConfig,
        database: CallCenterDatabase,
    ) -> CallCenterResult<Arc<Self>> {
        info!("🚀 Creating CallCenterEngine with session-core CallHandler integration");
        
        // First, create a placeholder engine that will be updated
        let placeholder_engine = Arc::new(Self {
            config: config.clone(),
            database: database.clone(),
            session_coordinator: None,
            queue_manager: Arc::new(RwLock::new(QueueManager::new())),
            bridge_events: None,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            available_agents: Arc::new(DashMap::new()),
            routing_stats: Arc::new(RwLock::new(RoutingStats::default())),
            agent_registry: Arc::new(Mutex::new(AgentRegistry::new(database.clone()))),
            sip_registrar: Arc::new(Mutex::new(SipRegistrar::new())),
        });
        
        // Create CallHandler with weak reference to placeholder
        let handler = Arc::new(CallCenterCallHandler {
            engine: Arc::downgrade(&placeholder_engine),
        });
        
        // Create session coordinator with our CallHandler
        let session_coordinator = SessionManagerBuilder::new()
            .with_sip_port(config.general.local_signaling_addr.port())
            .with_media_ports(
                config.general.local_media_addr.port(),
                config.general.local_media_addr.port() + 1000
            )
            .with_handler(handler.clone())
            .build()
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create session coordinator: {}", e)))?;
        
        info!("✅ SessionCoordinator created with CallCenterCallHandler");
        
        // Drop the placeholder and create the real engine with coordinator
        drop(placeholder_engine);
        
        let engine = Arc::new(Self {
            config,
            database: database.clone(),
            session_coordinator: Some(session_coordinator),
            queue_manager: Arc::new(RwLock::new(QueueManager::new())),
            bridge_events: None,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            available_agents: Arc::new(DashMap::new()),
            routing_stats: Arc::new(RwLock::new(RoutingStats::default())),
            agent_registry: Arc::new(Mutex::new(AgentRegistry::new(database))),
            sip_registrar: Arc::new(Mutex::new(SipRegistrar::new())),
        });
        
        // CRITICAL FIX: Update the handler's weak reference to point to the real engine
        // Since handler is Arc, we need to get a mutable reference
        // We'll use unsafe to cast away the Arc's immutability for this one-time update
        unsafe {
            let handler_ptr = Arc::as_ptr(&handler) as *mut CallCenterCallHandler;
            (*handler_ptr).engine = Arc::downgrade(&engine);
        }
        
        info!("✅ Call center engine initialized with session-core integration");
        
        Ok(engine)
    }
    
    /// Get orchestrator statistics with Phase 2 details
    pub async fn get_stats(&self) -> OrchestratorStats {
        let active_calls = self.active_calls.read().await;
        let bridges = self.list_active_bridges().await;
        
        let queued_calls = active_calls.values()
            .filter(|call| matches!(call.status, CallStatus::Queued))
            .count();
            
        // Count available vs busy agents
        let (available_count, busy_count) = self.available_agents
            .iter()
            .fold((0, 0), |(avail, busy), entry| {
                let agent = entry.value();
                match agent.status {
                    AgentStatus::Available if agent.current_calls == 0 => (avail + 1, busy),
                    _ => (avail, busy + 1),
                }
            });
        
        let routing_stats = self.routing_stats.read().await;
        
        OrchestratorStats {
            active_calls: active_calls.len(),
            active_bridges: bridges.len(),
            total_calls_handled: routing_stats.calls_routed_directly + routing_stats.calls_queued,
            available_agents: available_count,
            busy_agents: busy_count,
            queued_calls,
            routing_stats: routing_stats.clone(),
        }
    }
    
    /// Get the underlying session coordinator for advanced operations
    pub fn session_manager(&self) -> &Arc<SessionCoordinator> {
        self.session_coordinator.as_ref().unwrap()
    }
    
    /// Get call center configuration
    pub fn config(&self) -> &CallCenterConfig {
        &self.config
    }
    
    /// Get database handle
    pub fn database(&self) -> &CallCenterDatabase {
        &self.database
    }
    
    /// Start monitoring session events (including REGISTER requests)
    pub async fn start_event_monitoring(self: Arc<Self>) -> CallCenterResult<()> {
        info!("Starting session event monitoring for REGISTER and other events");
        
        let session_manager = self.session_manager();
        
        // Subscribe to session events
        let mut event_subscriber = session_manager.event_processor.subscribe().await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to subscribe to events: {}", e)))?;
        
        // Spawn event processing task
        let engine = self.clone();
        tokio::spawn(async move {
            while let Ok(event) = event_subscriber.receive().await {
                if let Err(e) = engine.handle_session_event(event).await {
                    tracing::error!("Error handling session event: {}", e);
                }
            }
        });
        
        Ok(())
    }
    
    /// Handle session events
    async fn handle_session_event(&self, event: SessionEvent) -> CallCenterResult<()> {
        match event {
            SessionEvent::RegistrationRequest { transaction_id, from_uri, contact_uri, expires } => {
                info!("Received REGISTER request: {} -> {} (expires: {})", from_uri, contact_uri, expires);
                self.handle_register_request(&transaction_id, from_uri, contact_uri, expires).await?;
            }
            _ => {
                // Other events are handled by existing mechanisms
            }
        }
        Ok(())
    }
    
    /// Update call state tracking
    pub async fn update_call_state(&self, session_id: &SessionId, new_state: &CallState) -> CallCenterResult<()> {
        let mut calls = self.active_calls.write().await;
        if let Some(call_info) = calls.get_mut(session_id) {
            info!("Updating call {} state to {:?}", session_id, new_state);
            call_info.status = match new_state {
                CallState::Initiating => CallStatus::Incoming,
                CallState::Ringing => CallStatus::Ringing,
                CallState::Active => CallStatus::Bridged,
                CallState::Terminated => CallStatus::Ended,
                CallState::Failed(_) => CallStatus::Ended,
                // For any other states, keep the current status
                _ => call_info.status.clone(),
            };
        }
        Ok(())
    }
    
    /// Route incoming call when it starts ringing
    pub async fn route_incoming_call(&self, session_id: &SessionId) -> CallCenterResult<()> {
        info!("Routing incoming call {} to available agent", session_id);
        
        // Get call info
        let calls = self.active_calls.read().await;
        let call_info = calls.get(session_id)
            .ok_or_else(|| CallCenterError::not_found(format!("Call {} not found", session_id)))?;
        
        // Create an IncomingCall structure for the routing engine
        let incoming_call = IncomingCall {
            id: session_id.clone(),
            from: call_info.caller_id.clone(),
            to: "support".to_string(), // Default destination
            sdp: None,
            headers: std::collections::HashMap::new(),
            received_at: std::time::Instant::now(),
        };
        
        drop(calls); // Release the lock
        
        // Analyze customer and make routing decision
        let (customer_type, priority, required_skills) = self.analyze_customer_info(&incoming_call).await;
        let routing_decision = self.make_routing_decision(
            session_id,
            &customer_type,
            priority,
            &required_skills
        ).await?;
        
        // Execute the routing decision
        match routing_decision {
            RoutingDecision::DirectToAgent { agent_id, reason } => {
                info!("📞 Direct routing to agent {}: {}", agent_id, reason);
                self.assign_specific_agent_to_call(session_id.clone(), agent_id).await?;
            }
            RoutingDecision::Queue { queue_id, priority, reason } => {
                info!("📋 Queueing call {} in queue {} with priority {} ({})", 
                      session_id, queue_id, priority, reason);
                
                // Ensure queue exists
                self.ensure_queue_exists(&queue_id).await?;
                
                // Create queued call entry
                let queued_call = crate::queue::QueuedCall {
                    session_id: session_id.clone(),
                    caller_id: incoming_call.from,
                    priority,
                    queued_at: chrono::Utc::now(),
                    estimated_wait_time: None,
                };
                
                // Enqueue the call
                let mut queue_manager = self.queue_manager.write().await;
                queue_manager.enqueue_call(&queue_id, queued_call)?;
                
                // Update call status
                drop(queue_manager);
                let mut calls = self.active_calls.write().await;
                if let Some(call_info) = calls.get_mut(session_id) {
                    call_info.status = CallStatus::Queued;
                    call_info.queue_id = Some(queue_id.clone());
                }
                
                // Update routing stats
                let mut stats = self.routing_stats.write().await;
                stats.calls_queued += 1;
            }
            RoutingDecision::Overflow { target_queue, reason } => {
                info!("📤 Overflow routing to queue {}: {}", target_queue, reason);
                // Handle overflow (similar to queue)
                self.ensure_queue_exists(&target_queue).await?;
                
                let queued_call = crate::queue::QueuedCall {
                    session_id: session_id.clone(),
                    caller_id: incoming_call.from,
                    priority: 200, // Lower priority for overflow
                    queued_at: chrono::Utc::now(),
                    estimated_wait_time: None,
                };
                
                let mut queue_manager = self.queue_manager.write().await;
                queue_manager.enqueue_call(&target_queue, queued_call)?;
                
                // Update routing stats - no calls_overflowed field, increment calls_queued instead
                let mut stats = self.routing_stats.write().await;
                stats.calls_queued += 1;
            }
            RoutingDecision::Reject { reason } => {
                warn!("❌ Rejecting incoming call {}: {}", session_id, reason);
                // Update routing stats
                let mut stats = self.routing_stats.write().await;
                stats.calls_rejected += 1;
            }
            RoutingDecision::Conference { bridge_id: _ } => {
                info!("🎤 Conference routing not yet implemented for call {}", session_id);
                // TODO: Implement conference routing
            }
        }
        
        Ok(())
    }
    
    /// Clean up resources when call terminates
    pub async fn cleanup_call(&self, session_id: &SessionId) -> CallCenterResult<()> {
        info!("Cleaning up terminated call {}", session_id);
        self.active_calls.write().await.remove(session_id);
        Ok(())
    }
    
    /// Record quality metrics for a call
    pub async fn record_quality_metrics(
        &self, 
        session_id: &SessionId, 
        mos_score: f32, 
        packet_loss: f32
    ) -> CallCenterResult<()> {
        info!("Recording quality metrics for call {}: MOS={}, Loss={}%", 
              session_id, mos_score, packet_loss);
        // TODO: Store in database
        Ok(())
    }
    
    /// Alert supervisors about poor call quality
    pub async fn alert_poor_quality(
        &self, 
        session_id: &SessionId, 
        mos_score: f32, 
        alert_level: MediaQualityAlertLevel
    ) -> CallCenterResult<()> {
        tracing::warn!("Poor quality alert for call {}: MOS={}, Level={:?}", 
                      session_id, mos_score, alert_level);
        // TODO: Send notification to supervisors
        Ok(())
    }
    
    /// Process DTMF input for IVR or features
    pub async fn process_dtmf_input(
        &self, 
        session_id: &SessionId, 
        digit: char
    ) -> CallCenterResult<()> {
        info!("Processing DTMF '{}' for call {}", digit, session_id);
        // TODO: Implement DTMF processing (IVR navigation, agent codes, etc.)
        Ok(())
    }
    
    /// Update media flow status
    pub async fn update_media_flow(
        &self, 
        session_id: &SessionId, 
        direction: MediaFlowDirection, 
        active: bool, 
        codec: &str
    ) -> CallCenterResult<()> {
        info!("Media flow update for call {}: {:?} {} ({})", 
              session_id, direction, if active { "active" } else { "inactive" }, codec);
        // TODO: Track media flow state
        Ok(())
    }
    
    /// Log warning for monitoring
    pub async fn log_warning(
        &self, 
        session_id: Option<&SessionId>, 
        category: WarningCategory, 
        message: &str
    ) -> CallCenterResult<()> {
        match session_id {
            Some(id) => tracing::warn!("Warning for call {} ({:?}): {}", id, category, message),
            None => tracing::warn!("General warning ({:?}): {}", category, message),
        }
        // TODO: Store in monitoring system
        Ok(())
    }
    
    // === Public accessor methods for APIs ===
    
    /// Get read access to active calls
    pub fn active_calls(&self) -> &Arc<RwLock<HashMap<SessionId, CallInfo>>> {
        &self.active_calls
    }
    
    /// Get read access to routing stats
    pub fn routing_stats(&self) -> &Arc<RwLock<RoutingStats>> {
        &self.routing_stats
    }
    
    /// Get read access to queue manager
    pub fn queue_manager(&self) -> &Arc<RwLock<QueueManager>> {
        &self.queue_manager
    }
    
    /// Assign a specific agent to a call (public for supervisor API)
    pub async fn assign_agent_to_call(&self, session_id: SessionId, agent_id: AgentId) -> CallCenterResult<()> {
        self.assign_specific_agent_to_call(session_id, agent_id).await
    }
    
    /// Ensure a queue exists (public for admin API)
    pub async fn create_queue(&self, queue_id: &str) -> CallCenterResult<()> {
        self.ensure_queue_exists(queue_id).await
    }
    
    /// Process all queues to assign waiting calls to available agents
    pub async fn process_all_queues(&self) -> CallCenterResult<()> {
        let mut queue_manager = self.queue_manager.write().await;
        
        // Get all queue IDs
        let queue_ids: Vec<String> = queue_manager.get_queue_ids();
        
        for queue_id in queue_ids {
            // Process each queue
            while let Some(queued_call) = queue_manager.dequeue_for_agent(&queue_id)? {
                // Find an available agent
                let available_agent = self.available_agents.iter()
                    .find(|entry| {
                        let agent = entry.value();
                        matches!(agent.status, AgentStatus::Available) && agent.current_calls == 0
                    })
                    .map(|entry| (entry.key().clone(), entry.value().clone()));
                
                if let Some((agent_id, _agent_info)) = available_agent {
                    // Assign the call to the agent
                    info!("🎯 Assigning queued call {} to available agent {}", 
                          queued_call.session_id, agent_id);
                    
                    // We need to drop the queue_manager lock before calling assign_specific_agent_to_call
                    drop(queue_manager);
                    
                    if let Err(e) = self.assign_specific_agent_to_call(
                        queued_call.session_id.clone(), 
                        agent_id
                    ).await {
                        error!("Failed to assign call to agent: {}", e);
                        // Re-queue the call if assignment fails
                        queue_manager = self.queue_manager.write().await;
                        let _ = queue_manager.enqueue_call(&queue_id, queued_call);
                    } else {
                        // Successfully assigned, get the lock again for the next iteration
                        queue_manager = self.queue_manager.write().await;
                    }
                } else {
                    // No available agents, put the call back in the queue
                    let _ = queue_manager.enqueue_call(&queue_id, queued_call);
                    break; // Stop processing this queue
                }
            }
        }
        
        Ok(())
    }
} 

// Make CallCenterEngine cloneable for async operations
impl Clone for CallCenterEngine {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            database: self.database.clone(),
            session_coordinator: self.session_coordinator.clone(),
            queue_manager: self.queue_manager.clone(),
            bridge_events: None, // Don't clone the receiver
            active_calls: self.active_calls.clone(),
            available_agents: self.available_agents.clone(),
            routing_stats: self.routing_stats.clone(),
            agent_registry: self.agent_registry.clone(),
            sip_registrar: self.sip_registrar.clone(),
        }
    }
} 