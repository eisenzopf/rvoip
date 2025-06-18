//! Core call center engine
//!
//! This module contains the main CallCenterEngine struct that coordinates
//! all call center operations through session-core integration.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, RwLock, Mutex};
use tracing::info;

use rvoip_session_core::{SessionCoordinator, SessionManagerBuilder, SessionId, BridgeEvent};
use rvoip_session_core::prelude::SessionEvent;

use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::config::CallCenterConfig;
use crate::database::CallCenterDatabase;
use crate::agent::{Agent, AgentId, AgentRegistry, AgentStatus, SipRegistrar};
use crate::queue::{CallQueue, QueueManager};
use crate::routing::RoutingEngine;

use super::types::{CallInfo, AgentInfo, RoutingStats, OrchestratorStats, CallStatus};
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
    pub(super) available_agents: Arc<RwLock<HashMap<AgentId, AgentInfo>>>,
    
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
            available_agents: Arc::new(RwLock::new(HashMap::new())),
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
            .with_handler(handler)
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
            available_agents: Arc::new(RwLock::new(HashMap::new())),
            routing_stats: Arc::new(RwLock::new(RoutingStats::default())),
            agent_registry: Arc::new(Mutex::new(AgentRegistry::new(database))),
            sip_registrar: Arc::new(Mutex::new(SipRegistrar::new())),
        });
        
        info!("✅ Call center engine initialized with session-core integration");
        
        Ok(engine)
    }
    
    /// Get orchestrator statistics with Phase 2 details
    pub async fn get_stats(&self) -> OrchestratorStats {
        let active_calls = self.active_calls.read().await;
        let available_agents = self.available_agents.read().await;
        let bridges = self.list_active_bridges().await;
        
        let queued_calls = active_calls.values()
            .filter(|call| matches!(call.status, CallStatus::Queued))
            .count();
            
        // Count available vs busy agents
        let (available_count, busy_count) = available_agents.values()
            .fold((0, 0), |(avail, busy), agent| {
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