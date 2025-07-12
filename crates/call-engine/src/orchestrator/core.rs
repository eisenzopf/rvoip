//! Core call center orchestration engine
//!
//! This module contains the main [`CallCenterEngine`] struct that coordinates
//! all call center operations through session-core integration. It serves as
//! the central orchestrator for agents, calls, queues, and routing decisions.

use std::sync::Arc;
use std::collections::HashMap;
use dashmap::{DashMap, DashSet};
use tokio::sync::{mpsc, RwLock, Mutex};
use tracing::{info, error, warn, debug};

use rvoip_session_core::{
    SessionCoordinator, SessionManagerBuilder, SessionId, BridgeEvent, CallState,
    MediaQualityAlertLevel, MediaFlowDirection, WarningCategory, IncomingCall,
    SessionControl
};
use rvoip_session_core::prelude::SessionEvent;

use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::config::CallCenterConfig;
use crate::agent::{Agent, AgentId, AgentRegistry, AgentStatus, SipRegistrar};
use crate::queue::{CallQueue, QueueManager};
use crate::routing::RoutingEngine;
use crate::monitoring::MetricsCollector;
use crate::database::DatabaseManager;

use super::types::{CallInfo, AgentInfo, RoutingStats, OrchestratorStats, CallStatus, RoutingDecision, CustomerType, BridgeInfo, PendingAssignment};
use super::handler::CallCenterCallHandler;

/// Internal call center engine state
///
/// This structure holds all the shared state components used by the call center engine.
/// It's used internally to organize the various subsystems and their interactions.
pub(super) struct CallCenterState {
    /// Configuration for the call center
    pub(super) config: CallCenterConfig,
    /// Session coordinator for SIP operations
    pub(super) session_coordinator: Arc<SessionCoordinator>,
    /// Active calls tracking with detailed information
    pub(super) active_calls: Arc<DashMap<SessionId, CallInfo>>,
    /// Active SIP bridges between agents and customers
    pub(super) active_bridges: Arc<DashMap<String, BridgeInfo>>,
    /// Queue manager for call queuing and routing
    pub(super) queue_manager: Arc<RwLock<QueueManager>>,
    /// Routing statistics and performance metrics
    pub(super) routing_stats: Arc<RwLock<RoutingStats>>,
    /// Agent registry for agent management
    pub(super) agent_registry: Arc<Mutex<AgentRegistry>>,
    /// SIP registrar for handling agent registrations
    pub(super) sip_registrar: Arc<Mutex<SipRegistrar>>,
    /// Track active queue monitors to prevent duplicates
    pub(super) active_queue_monitors: Arc<DashSet<String>>,
    /// Database manager for persistent storage
    pub(super) db_manager: Option<Arc<DatabaseManager>>,
    /// Pending agent assignments waiting for answer
    pub(super) pending_assignments: Arc<DashMap<SessionId, PendingAssignment>>,
}

/// Primary call center orchestration engine
/// 
/// This is the main orchestration component that integrates with rvoip-session-core
/// to provide comprehensive call center functionality on top of SIP session management.
/// 
/// The `CallCenterEngine` serves as the central coordinator for:
/// 
/// - **Agent Management**: Registration, authentication, and availability tracking
/// - **Call Routing**: Intelligent routing based on skills, availability, and business rules
/// - **Queue Management**: Call queuing with priorities and overflow handling
/// - **Bridge Operations**: SIP bridge creation and management between agents and customers
/// - **Real-time Monitoring**: Statistics collection and supervisor notifications
/// - **Database Integration**: Persistent storage of call data and agent information
///
/// # Architecture
///
/// The engine integrates multiple subsystems:
///
/// ```text
/// ┌─────────────────────────────────────┐
/// │         CallCenterEngine            │
/// ├─────────────────────────────────────┤
/// │ ┌─────────────┐ ┌─────────────────┐ │
/// │ │ Agent       │ │ Queue           │ │
/// │ │ Registry    │ │ Manager         │ │
/// │ └─────────────┘ └─────────────────┘ │
/// │ ┌─────────────┐ ┌─────────────────┐ │
/// │ │ Routing     │ │ Database        │ │
/// │ │ Engine      │ │ Manager         │ │
/// │ └─────────────┘ └─────────────────┘ │
/// └─────────────────────────────────────┘
///                    │
///           ┌─────────────────┐
///           │ Session         │
///           │ Coordinator     │ (rvoip-session-core)
///           └─────────────────┘
/// ```
///
/// # Examples
///
/// ## Basic Setup
///
/// ```
/// use rvoip_call_engine::prelude::*;
/// 
/// # async fn example() -> Result<()> {
/// let config = CallCenterConfig::default();
/// let engine = CallCenterEngine::new(config, None).await?;
/// 
/// println!("Call center engine started successfully!");
/// # Ok(())
/// # }
/// ```
///
/// ## Agent Registration
///
/// ```
/// use rvoip_call_engine::prelude::*;
/// use std::sync::Arc;
/// 
/// # async fn example(engine: Arc<CallCenterEngine>) -> Result<()> {
/// let agent = Agent {
///     id: "agent-001".to_string(),
///     sip_uri: "sip:alice@call-center.local".to_string(),
///     display_name: "Alice Johnson".to_string(),
///     skills: vec!["english".to_string(), "sales".to_string()],
///     max_concurrent_calls: 3,
///     status: AgentStatus::Available,
///     department: Some("sales".to_string()),
///     extension: Some("1001".to_string()),
/// };
/// 
/// let session_id = engine.register_agent(&agent).await?;
/// println!("Agent registered with session: {}", session_id);
/// # Ok(())
/// # }
/// ```
///
/// ## Statistics Monitoring
///
/// ```
/// use rvoip_call_engine::prelude::*;
/// use std::sync::Arc;
/// 
/// # async fn example(engine: Arc<CallCenterEngine>) -> Result<()> {
/// let stats = engine.get_stats().await;
/// println!("Call center status:");
/// println!("  Active calls: {}", stats.active_calls);
/// println!("  Available agents: {}", stats.available_agents);
/// println!("  Queued calls: {}", stats.queued_calls);
/// # Ok(())
/// # }
/// ```
pub struct CallCenterEngine {
    /// Configuration for the call center
    pub(super) config: CallCenterConfig,
    
    /// Database manager for persistent storage
    pub(super) db_manager: Option<Arc<DatabaseManager>>,
    
    /// Session-core coordinator integration
    pub(super) session_coordinator: Option<Arc<SessionCoordinator>>,
    
    /// Queue manager for call queuing and routing
    pub(super) queue_manager: Arc<RwLock<QueueManager>>,
    
    /// Bridge event receiver for real-time notifications
    pub(super) bridge_events: Option<mpsc::UnboundedReceiver<BridgeEvent>>,
    
    /// Call tracking and routing with detailed info
    pub(super) active_calls: Arc<DashMap<SessionId, CallInfo>>,
    
    /// Call routing statistics and metrics
    pub(super) routing_stats: Arc<RwLock<RoutingStats>>,
    
    /// Agent registry for agent management
    pub(crate) agent_registry: Arc<Mutex<AgentRegistry>>,
    
    /// SIP Registrar for handling agent registrations
    pub(crate) sip_registrar: Arc<Mutex<SipRegistrar>>,
    
    /// Track active queue monitors to prevent duplicates
    pub(super) active_queue_monitors: Arc<DashSet<String>>,
    
    /// Session ID to Dialog ID mappings for robust lookup
    pub session_to_dialog: Arc<DashMap<String, String>>,
    
    /// Pending agent assignments waiting for answer
    pub(super) pending_assignments: Arc<DashMap<SessionId, PendingAssignment>>,
}

impl CallCenterEngine {
    /// Create a new call center engine with configuration and optional database
    ///
    /// This is the primary constructor for the call center engine. It initializes
    /// all subsystems including session management, agent registry, queue management,
    /// and database connectivity.
    ///
    /// # Arguments
    ///
    /// * `config` - Call center configuration including networking, routing, and system limits
    /// * `db_path` - Optional database file path. Use `None` for in-memory operation,
    ///               `Some(":memory:")` for explicit in-memory, or `Some("path.db")` for persistent storage
    ///
    /// # Returns
    ///
    /// Returns an `Arc<CallCenterEngine>` for shared ownership across the application,
    /// or a `CallCenterError` if initialization fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// 
    /// # async fn example() -> Result<()> {
    /// // In-memory call center for testing
    /// let engine1 = CallCenterEngine::new(
    ///     CallCenterConfig::default(), 
    ///     None
    /// ).await?;
    /// 
    /// // Persistent call center for production
    /// let engine2 = CallCenterEngine::new(
    ///     CallCenterConfig::default(), 
    ///     Some("callcenter.db".to_string())
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// - `CallCenterError::Configuration` - Invalid configuration settings
    /// - `CallCenterError::Database` - Database initialization failure
    /// - `CallCenterError::Integration` - Session-core integration failure
    /// - `CallCenterError::Orchestration` - General initialization failure
    pub async fn new(
        config: CallCenterConfig,
        db_path: Option<String>,
    ) -> CallCenterResult<Arc<Self>> {
        info!("🚀 Creating CallCenterEngine with session-core CallHandler integration");
        
        // Initialize the database manager
        let db_manager = if let Some(path) = db_path.as_ref() {
            match DatabaseManager::new(path).await {
                Ok(mgr) => {
                    info!("✅ Initialized database at: {}", path);
                    Some(Arc::new(mgr))
                }
                Err(e) => {
                    warn!("Failed to initialize database manager: {}. Continuing with in-memory operations.", e);
                    None
                }
            }
        } else {
            None
        };
        
        // First, create a placeholder engine that will be updated
        let placeholder_engine = Arc::new(Self {
            config: config.clone(),
            db_manager: db_manager.clone(),
            session_coordinator: None,
            queue_manager: Arc::new(RwLock::new(QueueManager::new())),
            bridge_events: None,
            active_calls: Arc::new(DashMap::new()),
            routing_stats: Arc::new(RwLock::new(RoutingStats::default())),
            agent_registry: Arc::new(Mutex::new(AgentRegistry::new())),
            sip_registrar: Arc::new(Mutex::new(SipRegistrar::new())),
            active_queue_monitors: Arc::new(DashSet::new()),
            session_to_dialog: Arc::new(DashMap::new()),
            pending_assignments: Arc::new(DashMap::new()),
        });
        
        // Create CallHandler with weak reference to placeholder
        let handler = Arc::new(CallCenterCallHandler {
            engine: Arc::downgrade(&placeholder_engine),
        });
        
        // Create session coordinator with our CallHandler
        // CRITICAL: Configure both SIP address and media bind address to use the configured IP
        let sip_uri = format!("sip:call-center@{}", config.general.local_ip);
        let session_coordinator = SessionManagerBuilder::new()
            .with_sip_port(config.general.local_signaling_addr.port())
            .with_local_address(sip_uri)  // Use configured IP for SIP URIs
            .with_local_bind_addr(config.general.local_signaling_addr)  // Use configured IP for binding
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
            db_manager,
            session_coordinator: Some(session_coordinator),
            queue_manager: Arc::new(RwLock::new(QueueManager::new())),
            bridge_events: None,
            active_calls: Arc::new(DashMap::new()),
            routing_stats: Arc::new(RwLock::new(RoutingStats::default())),
            agent_registry: Arc::new(Mutex::new(AgentRegistry::new())),
            sip_registrar: Arc::new(Mutex::new(SipRegistrar::new())),
            active_queue_monitors: Arc::new(DashSet::new()),
            session_to_dialog: Arc::new(DashMap::new()),
            pending_assignments: Arc::new(DashMap::new()),
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
    
    /// Get comprehensive orchestrator statistics and performance metrics
    ///
    /// Returns a snapshot of the current call center state including active calls,
    /// agent availability, queue status, and routing performance metrics.
    ///
    /// # Returns
    ///
    /// [`OrchestratorStats`] containing:
    /// - Current active calls and bridges
    /// - Agent availability (available vs busy)
    /// - Queued calls waiting for agents
    /// - Total calls handled since startup
    /// - Detailed routing statistics
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// use std::sync::Arc;
    /// 
    /// # async fn example(engine: Arc<CallCenterEngine>) -> Result<()> {
    /// let stats = engine.get_stats().await;
    /// 
    /// println!("📊 Call Center Dashboard");
    /// println!("   Active calls: {}", stats.active_calls);
    /// println!("   Available agents: {}", stats.available_agents);
    /// println!("   Busy agents: {}", stats.busy_agents);
    /// println!("   Queued calls: {}", stats.queued_calls);
    /// println!("   Total handled: {}", stats.total_calls_handled);
    /// 
    /// println!("📈 Routing Performance");
    /// println!("   Direct routes: {}", stats.routing_stats.calls_routed_directly);
    /// println!("   Queued routes: {}", stats.routing_stats.calls_queued);
    /// println!("   Rejected calls: {}", stats.routing_stats.calls_rejected);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_stats(&self) -> OrchestratorStats {
        let bridges = self.list_active_bridges().await;
        
        let queued_calls = self.active_calls
            .iter()
            .filter(|entry| matches!(entry.value().status, CallStatus::Queued))
            .count();
            
        // Count available vs busy agents from database
        let (available_count, busy_count) = if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent_stats().await {
                Ok(stats) => (stats.available_agents, stats.busy_agents + stats.post_call_wrap_up_agents),
                Err(e) => {
                    error!("Failed to get agent stats from database: {}", e);
                    (0, 0)
                }
            }
        } else {
            (0, 0)
        };
        
        let routing_stats = self.routing_stats.read().await;
        
        OrchestratorStats {
            active_calls: self.active_calls.len(),
            active_bridges: bridges.len(),
            total_calls_handled: routing_stats.calls_routed_directly + routing_stats.calls_queued,
            available_agents: available_count as usize,
            busy_agents: busy_count as usize,
            queued_calls,
            routing_stats: routing_stats.clone(),
        }
    }
    
    /// Get the underlying session coordinator for advanced SIP operations
    ///
    /// Provides access to the rvoip-session-core coordinator for advanced
    /// SIP operations, custom call handling, or direct session management.
    ///
    /// # Returns
    ///
    /// A reference to the [`SessionCoordinator`] for direct session operations.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// use std::sync::Arc;
    /// 
    /// # async fn example(engine: Arc<CallCenterEngine>) -> Result<()> {
    /// let session_manager = engine.session_manager();
    /// 
    /// // Access session-core functionality directly
    /// // let custom_session = session_manager.create_session(...).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Use Cases
    ///
    /// - Custom SIP message handling
    /// - Advanced session configuration
    /// - Direct bridge management
    /// - Low-level SIP debugging
    pub fn session_manager(&self) -> &Arc<SessionCoordinator> {
        self.session_coordinator.as_ref().unwrap()
    }
    
    /// Get call center configuration
    ///
    /// Returns a reference to the current call center configuration.
    /// Useful for accessing configuration values in application logic.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// use std::sync::Arc;
    /// 
    /// # async fn example(engine: Arc<CallCenterEngine>) {
    /// let config = engine.config();
    /// 
    /// println!("Max concurrent calls: {}", config.general.max_concurrent_calls);
    /// println!("Max agents: {}", config.general.max_agents);
    /// println!("Routing strategy: {:?}", config.routing.default_strategy);
    /// # }
    /// ```
    pub fn config(&self) -> &CallCenterConfig {
        &self.config
    }
    
    /// Get database manager reference for direct database operations
    ///
    /// Provides access to the database manager for custom queries,
    /// reporting, or data export operations.
    ///
    /// # Returns
    ///
    /// `Some(&DatabaseManager)` if database is configured, `None` for in-memory operation.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// use std::sync::Arc;
    /// 
    /// # async fn example(engine: Arc<CallCenterEngine>) -> Result<()> {
    /// if let Some(db) = engine.database_manager() {
    ///     match db.get_available_agents().await {
    ///         Ok(agents) => println!("Found {} available agents in database", agents.len()),
    ///         Err(e) => println!("Error fetching agents: {}", e),
    ///     }
    /// } else {
    ///     println!("Running in memory-only mode");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn database_manager(&self) -> Option<&Arc<DatabaseManager>> {
        self.db_manager.as_ref()
    }
    
    /// Start monitoring session events including REGISTER requests and call events
    ///
    /// Begins monitoring all session events from the underlying SIP stack,
    /// including agent registrations, incoming calls, and call state changes.
    /// This method should be called after engine initialization to enable
    /// full call center functionality.
    ///
    /// # Returns
    ///
    /// `Ok(())` if monitoring started successfully, or `CallCenterError` if
    /// event subscription failed.
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// 
    /// # async fn example() -> Result<()> {
    /// let engine = CallCenterEngine::new(CallCenterConfig::default(), None).await?;
    /// 
    /// // Start event monitoring (essential for call center operation)
    /// engine.clone().start_event_monitoring().await?;
    /// 
    /// println!("Call center is now monitoring for calls and registrations");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Background Processing
    ///
    /// This method spawns a background task that continuously processes:
    /// - Agent REGISTER requests
    /// - Incoming call INVITE requests  
    /// - Call state changes
    /// - Media quality events
    /// - Call termination events
    ///
    /// # Errors
    ///
    /// - `CallCenterError::Orchestration` - Failed to subscribe to session events
    /// - `CallCenterError::Integration` - Session-core integration failure
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
    
    /// Handle session events from the SIP stack
    ///
    /// Internal method that processes various session events and routes them
    /// to appropriate handlers within the call center.
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
    
    /// Update call state tracking when call states change
    ///
    /// Internal method that maintains consistency between session-core call states
    /// and call center call tracking.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session ID of the call to update
    /// * `new_state` - The new call state from session-core
    pub async fn update_call_state(&self, session_id: &SessionId, new_state: &CallState) -> CallCenterResult<()> {
        if let Some(mut call_info) = self.active_calls.get_mut(session_id) {
            match new_state {
                CallState::Active => call_info.status = CallStatus::Bridged,
                CallState::Terminated => call_info.status = CallStatus::Disconnected,
                CallState::Failed(_) => call_info.status = CallStatus::Failed,
                _ => {} // Keep existing status for other states
            }
        }
        Ok(())
    }
    
    /// Route incoming call when it starts ringing
    ///
    /// Internal method called when a new call arrives that needs to be routed
    /// to an appropriate agent or queue.
    pub async fn route_incoming_call(&self, session_id: &SessionId) -> CallCenterResult<()> {
        // This is handled in process_incoming_call
        Ok(())
    }
    
    /// Clean up resources when call terminates
    ///
    /// Internal method that handles cleanup when a call ends, including
    /// removing call tracking, updating agent availability, and database cleanup.
    pub async fn cleanup_call(&self, session_id: &SessionId) -> CallCenterResult<()> {
        // This is handled in handle_call_termination
        self.handle_call_termination(session_id.clone()).await
    }
    
    /// Record quality metrics for a call
    ///
    /// Records call quality information for monitoring and reporting purposes.
    /// Quality metrics are used for supervisor dashboards and quality alerts.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session ID of the call
    /// * `mos_score` - Mean Opinion Score (1.0 to 5.0, higher is better)
    /// * `packet_loss` - Packet loss percentage (0.0 to 100.0)
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// use std::sync::Arc;
    /// 
    /// # async fn example(engine: Arc<CallCenterEngine>, session_id: SessionId) -> Result<()> {
    /// // Record good quality metrics
    /// engine.record_quality_metrics(&session_id, 4.2, 0.1).await?;
    /// 
    /// // Record poor quality metrics  
    /// engine.record_quality_metrics(&session_id, 2.1, 5.5).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn record_quality_metrics(
        &self, 
        session_id: &SessionId, 
        mos_score: f32, 
        packet_loss: f32
    ) -> CallCenterResult<()> {
        // TODO: Store quality metrics in database
        debug!("Recording quality metrics for {}: MOS={}, Loss={}%", session_id, mos_score, packet_loss);
        Ok(())
    }
    
    /// Alert supervisors about poor call quality
    ///
    /// Generates alerts for supervisors when call quality falls below acceptable
    /// thresholds. Alerts can trigger supervisor intervention or call recording.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session ID of the call with poor quality
    /// * `mos_score` - Current Mean Opinion Score
    /// * `alert_level` - Severity level of the quality issue
    pub async fn alert_poor_quality(
        &self, 
        session_id: &SessionId, 
        mos_score: f32, 
        alert_level: MediaQualityAlertLevel
    ) -> CallCenterResult<()> {
        // TODO: Send alerts to supervisors
        warn!("Poor quality alert for {}: MOS={}, Level={:?}", session_id, mos_score, alert_level);
        Ok(())
    }
    
    /// Process DTMF input for IVR or call features
    ///
    /// Handles DTMF (touch-tone) input from callers for Interactive Voice Response
    /// systems, call transfers, or other telephony features.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session ID generating the DTMF
    /// * `digit` - The DTMF digit pressed ('0'-'9', '*', '#')
    pub async fn process_dtmf_input(
        &self, 
        session_id: &SessionId, 
        digit: char
    ) -> CallCenterResult<()> {
        // TODO: Implement DTMF handling for IVR
        info!("DTMF '{}' received for session {}", digit, session_id);
        Ok(())
    }
    
    /// Update media flow status for a call
    ///
    /// Tracks media flow status (audio start/stop) for monitoring and
    /// troubleshooting purposes.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The session ID with media flow changes
    /// * `direction` - Media flow direction (incoming/outgoing)
    /// * `active` - Whether media flow is active or stopped
    /// * `codec` - Audio codec being used
    pub async fn update_media_flow(
        &self, 
        session_id: &SessionId, 
        direction: MediaFlowDirection, 
        active: bool, 
        codec: &str
    ) -> CallCenterResult<()> {
        // TODO: Track media flow status
        debug!("Media flow update for {}: {:?} {} ({})", session_id, direction, if active { "started" } else { "stopped" }, codec);
        Ok(())
    }
    
    /// Log warning for monitoring and troubleshooting
    ///
    /// Logs warnings with appropriate categorization for monitoring systems
    /// and troubleshooting.
    ///
    /// # Arguments
    ///
    /// * `session_id` - Optional session ID related to the warning
    /// * `category` - Warning category for filtering and routing
    /// * `message` - Warning message
    pub async fn log_warning(
        &self, 
        session_id: Option<&SessionId>, 
        category: WarningCategory, 
        message: &str
    ) -> CallCenterResult<()> {
        // TODO: Log to monitoring system
        match session_id {
            Some(id) => warn!("[{:?}] Warning for {}: {}", category, id, message),
            None => warn!("[{:?}] General warning: {}", category, message),
        }
        Ok(())
    }
    
    // === Public accessor methods for APIs ===
    
    /// Get read access to active calls tracking
    ///
    /// Provides access to the active calls collection for API implementations
    /// and monitoring systems.
    pub fn active_calls(&self) -> &Arc<DashMap<SessionId, CallInfo>> {
        &self.active_calls
    }
    
    /// Get read access to routing statistics
    ///
    /// Provides access to routing performance metrics for APIs and reporting.
    pub fn routing_stats(&self) -> &Arc<RwLock<RoutingStats>> {
        &self.routing_stats
    }
    
    /// Get read access to queue manager
    ///
    /// Provides access to the queue manager for API implementations
    /// and queue monitoring.
    pub fn queue_manager(&self) -> &Arc<RwLock<QueueManager>> {
        &self.queue_manager
    }
    
    /// Assign a specific agent to a call (public for supervisor API)
    ///
    /// Allows supervisors to manually assign agents to specific calls,
    /// overriding automatic routing decisions.
    ///
    /// # Arguments
    ///
    /// * `session_id` - The call session to assign
    /// * `agent_id` - The agent to assign to the call
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// use std::sync::Arc;
    /// 
    /// # async fn example(engine: Arc<CallCenterEngine>) -> Result<()> {
    /// let session_id = SessionId::new(); // From incoming call
    /// let agent_id = AgentId::from("agent-specialist-001");
    /// 
    /// // Supervisor assigns specialist agent to complex call
    /// engine.assign_agent_to_call(session_id, agent_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn assign_agent_to_call(&self, session_id: SessionId, agent_id: AgentId) -> CallCenterResult<()> {
        self.assign_specific_agent_to_call(session_id, agent_id).await
    }
    
    /// Ensure a queue exists (public for admin API)
    ///
    /// Creates a queue if it doesn't already exist. Used by administrative
    /// APIs for queue management.
    ///
    /// # Arguments
    ///
    /// * `queue_id` - The queue identifier to create
    pub async fn create_queue(&self, queue_id: &str) -> CallCenterResult<()> {
        self.ensure_queue_exists(queue_id).await
    }
    
    /// Process all queues to assign waiting calls to available agents
    ///
    /// Attempts to match queued calls with available agents across all queues.
    /// This method is called periodically and can also be triggered manually
    /// for immediate queue processing.
    ///
    /// # Queue Processing Logic
    ///
    /// 1. Iterate through all configured queues
    /// 2. For each queue, attempt to dequeue calls
    /// 3. Find available agents with matching skills
    /// 4. Assign calls to agents or re-queue if no agents available
    /// 5. Update routing statistics
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_call_engine::prelude::*;
    /// use std::sync::Arc;
    /// 
    /// # async fn example(engine: Arc<CallCenterEngine>) -> Result<()> {
    /// // Manually trigger queue processing
    /// engine.process_all_queues().await?;
    /// 
    /// println!("Queue processing completed");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Performance Considerations
    ///
    /// - Processing time scales with queue size and agent count
    /// - Database queries are performed for agent availability
    /// - Should not be called too frequently to avoid performance impact
    pub async fn process_all_queues(&self) -> CallCenterResult<()> {
        let mut queue_manager = self.queue_manager.write().await;
        
        // Get all queue IDs
        let queue_ids: Vec<String> = queue_manager.get_queue_ids();
        
        for queue_id in queue_ids {
            // Process each queue
            while let Some(queued_call) = queue_manager.dequeue_for_agent(&queue_id)? {
                // Find an available agent from database
                let available_agents = if let Some(db_manager) = &self.db_manager {
                    match db_manager.get_available_agents().await {
                        Ok(agents) => agents,
                        Err(e) => {
                            error!("Failed to get available agents from database: {}", e);
                            vec![]
                        }
                    }
                } else {
                    vec![]
                };
                
                if let Some(agent) = available_agents.first() {
                    let agent_id = AgentId::from(agent.agent_id.clone());
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
            db_manager: self.db_manager.clone(),
            session_coordinator: self.session_coordinator.clone(),
            queue_manager: self.queue_manager.clone(),
            bridge_events: None, // Don't clone the receiver
            active_calls: self.active_calls.clone(),
            routing_stats: self.routing_stats.clone(),
            agent_registry: self.agent_registry.clone(),
            sip_registrar: self.sip_registrar.clone(),
            active_queue_monitors: self.active_queue_monitors.clone(),
            session_to_dialog: self.session_to_dialog.clone(),
            pending_assignments: self.pending_assignments.clone(),
        }
    }
} 