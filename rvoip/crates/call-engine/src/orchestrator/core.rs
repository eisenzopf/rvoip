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
use crate::agent::{AgentId, Agent, AgentStatus};
use crate::queue::{QueueManager, QueuedCall, QueueStats};

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
    
    /// **PHASE 2**: Queue manager for call queuing and routing
    queue_manager: Arc<RwLock<QueueManager>>,
    
    /// **REAL**: Bridge event receiver for real-time notifications
    bridge_events: Option<mpsc::UnboundedReceiver<BridgeEvent>>,
    
    /// **ENHANCED**: Call tracking and routing with detailed info
    active_calls: Arc<RwLock<HashMap<SessionId, CallInfo>>>,
    
    /// **ENHANCED**: Agent availability and skill tracking
    available_agents: Arc<RwLock<HashMap<AgentId, AgentInfo>>>,
    
    /// **PHASE 2**: Call routing statistics and metrics
    routing_stats: Arc<RwLock<RoutingStats>>,
}

/// Enhanced call information for tracking
#[derive(Debug, Clone)]
pub struct CallInfo {
    pub session_id: SessionId,
    pub caller_id: String,
    pub caller_info: CallerInfo,
    pub agent_id: Option<AgentId>,
    pub queue_id: Option<String>,
    pub bridge_id: Option<BridgeId>,
    pub status: CallStatus,
    pub priority: u8, // 0 = highest, 255 = lowest
    pub customer_type: CustomerType,
    pub required_skills: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub queued_at: Option<chrono::DateTime<chrono::Utc>>,
    pub answered_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Enhanced agent information for tracking
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub status: AgentStatus,
    pub skills: Vec<String>,
    pub current_calls: usize,
    pub max_calls: usize,
    pub last_call_end: Option<chrono::DateTime<chrono::Utc>>,
    pub performance_score: f64, // 0.0-1.0 for routing decisions
}

/// Customer type for priority routing
#[derive(Debug, Clone)]
pub enum CustomerType {
    VIP,
    Premium,
    Standard,
    Trial,
}

/// Call status enumeration
#[derive(Debug, Clone)]
pub enum CallStatus {
    Incoming,
    Routing,     // NEW: Being processed by routing engine
    Queued,
    Ringing,     // NEW: Ringing at agent
    Connecting,
    Bridged,
    OnHold,
    Transferring,
    Ended,
}

/// Routing decision enumeration  
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    DirectToAgent { agent_id: AgentId, reason: String },
    Queue { queue_id: String, priority: u8, reason: String },
    Conference { bridge_id: BridgeId },
    Reject { reason: String },
    Overflow { target_queue: String, reason: String },
}

/// Routing statistics for monitoring
#[derive(Debug, Clone)]
pub struct RoutingStats {
    pub calls_routed_directly: u64,
    pub calls_queued: u64,
    pub calls_rejected: u64,
    pub average_routing_time_ms: u64,
    pub skill_match_success_rate: f64,
}

/// Orchestrator statistics
#[derive(Debug, Clone)]
pub struct OrchestratorStats {
    pub active_calls: usize,
    pub active_bridges: usize,
    pub total_calls_handled: u64,
    pub available_agents: usize,
    pub busy_agents: usize,
    pub queued_calls: usize,
    pub routing_stats: RoutingStats,
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
            queue_manager: Arc::new(RwLock::new(QueueManager::new())),
            bridge_events: None,
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            available_agents: Arc::new(RwLock::new(HashMap::new())),
            routing_stats: Arc::new(RwLock::new(RoutingStats {
                calls_routed_directly: 0,
                calls_queued: 0,
                calls_rejected: 0,
                average_routing_time_ms: 0,
                skill_match_success_rate: 0.0,
            })),
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
    
    /// **PHASE 2**: Process incoming call event with sophisticated routing
    async fn process_incoming_call_event(&self, event: IncomingCallEvent) -> CallCenterResult<CallDecision> {
        let session_id = event.session_id.clone();
        let routing_start = std::time::Instant::now();
        
        info!("📞 Processing incoming call: {} from {} with PHASE 2 routing", session_id, event.caller_info.from);
        
        // **PHASE 2**: Analyze customer information and determine routing requirements
        let (customer_type, priority, required_skills) = self.analyze_customer_info(&event.caller_info).await;
        
        // Create enhanced call info tracking
        let call_info = CallInfo {
            session_id: session_id.clone(),
            caller_id: event.caller_info.from.clone(),
            caller_info: event.caller_info.clone(),
            agent_id: None,
            queue_id: None,
            bridge_id: None,
            status: CallStatus::Routing,
            priority,
            customer_type: customer_type.clone(),
            required_skills: required_skills.clone(),
            created_at: chrono::Utc::now(),
            queued_at: None,
            answered_at: None,
        };
        
        // Store call info
        {
            let mut active_calls = self.active_calls.write().await;
            active_calls.insert(session_id.clone(), call_info);
        }
        
        // **PHASE 2**: Intelligent routing decision based on multiple factors
        let routing_decision = self.make_routing_decision(&session_id, &customer_type, priority, &required_skills).await?;
        
        info!("🎯 Routing decision for call {}: {:?}", session_id, routing_decision);
        
        // Execute routing decision
        let call_decision = match routing_decision {
            RoutingDecision::DirectToAgent { agent_id, reason } => {
                info!("📞 Routing call {} directly to agent {} ({})", session_id, agent_id, reason);
                
                // Update call status and assign agent
                {
                    let mut active_calls = self.active_calls.write().await;
                    if let Some(call_info) = active_calls.get_mut(&session_id) {
                        call_info.status = CallStatus::Ringing;
                        call_info.agent_id = Some(agent_id.clone());
                    }
                }
                
                // Schedule immediate agent assignment
                let engine_clone = Arc::new(self.clone());
                let session_id_clone = session_id.clone();
                let agent_id_clone = agent_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = engine_clone.assign_specific_agent_to_call(session_id_clone, agent_id_clone).await {
                        error!("Failed to assign specific agent to call: {}", e);
                    }
                });
                
                // Update routing stats
                {
                    let mut stats = self.routing_stats.write().await;
                    stats.calls_routed_directly += 1;
                }
                
                CallDecision::Accept
            },
            
            RoutingDecision::Queue { queue_id, priority, reason } => {
                info!("📋 Queueing call {} in queue {} with priority {} ({})", session_id, queue_id, priority, reason);
                
                // Add call to queue
                let queued_call = QueuedCall {
                    session_id: session_id.clone(),
                    caller_id: event.caller_info.from.clone(),
                    priority,
                    queued_at: chrono::Utc::now(),
                    estimated_wait_time: None,
                };
                
                // Ensure queue exists
                self.ensure_queue_exists(&queue_id).await?;
                
                // Enqueue the call
                {
                    let mut queue_manager = self.queue_manager.write().await;
                    if let Err(e) = queue_manager.enqueue_call(&queue_id, queued_call) {
                        error!("Failed to enqueue call {}: {}", session_id, e);
                        return Ok(CallDecision::Reject {
                            status_code: StatusCode::ServerInternalError,
                            reason: Some("Queue full".to_string()),
                        });
                    }
                }
                
                // Update call status
                {
                    let mut active_calls = self.active_calls.write().await;
                    if let Some(call_info) = active_calls.get_mut(&session_id) {
                        call_info.status = CallStatus::Queued;
                        call_info.queue_id = Some(queue_id.clone());
                        call_info.queued_at = Some(chrono::Utc::now());
                    }
                }
                
                // Update routing stats
                {
                    let mut stats = self.routing_stats.write().await;
                    stats.calls_queued += 1;
                }
                
                // Start monitoring for agent availability
                self.monitor_queue_for_agents(queue_id.clone()).await;
                
                CallDecision::Accept
            },
            
            RoutingDecision::Overflow { target_queue, reason } => {
                info!("🔄 Overflowing call {} to queue {} ({})", session_id, target_queue, reason);
                
                // Recursive call with overflow queue
                let overflow_decision = RoutingDecision::Queue { 
                    queue_id: target_queue, 
                    priority: priority + 10, // Lower priority for overflow
                    reason: format!("Overflow: {}", reason)
                };
                
                // Process overflow decision (simplified)
                CallDecision::Accept
            },
            
            RoutingDecision::Reject { reason } => {
                warn!("❌ Rejecting call {} ({})", session_id, reason);
                
                // Update routing stats
                {
                    let mut stats = self.routing_stats.write().await;
                    stats.calls_rejected += 1;
                }
                
                CallDecision::Reject {
                    status_code: StatusCode::ServiceUnavailable,
                    reason: Some(reason),
                }
            },
            
            RoutingDecision::Conference { bridge_id } => {
                info!("🎤 Routing call {} to conference {}", session_id, bridge_id);
                // TODO: Implement conference routing
                CallDecision::Accept
            }
        };
        
        // Update routing time metrics
        let routing_time = routing_start.elapsed().as_millis() as u64;
        {
            let mut stats = self.routing_stats.write().await;
            stats.average_routing_time_ms = (stats.average_routing_time_ms + routing_time) / 2;
        }
        
        info!("✅ Call {} routing completed in {}ms", session_id, routing_time);
        Ok(call_decision)
    }
    
    /// **PHASE 2**: Analyze customer information to determine routing requirements
    async fn analyze_customer_info(&self, caller_info: &CallerInfo) -> (CustomerType, u8, Vec<String>) {
        // **FUTURE**: This would integrate with CRM systems, customer databases, etc.
        // For now, use simple heuristics based on caller information
        
        let caller_number = &caller_info.from;
        
        // Determine customer type (would be from database lookup in production)
        let customer_type = if caller_number.contains("+1800") || caller_number.contains("vip") {
            CustomerType::VIP
        } else if caller_number.contains("+1900") {
            CustomerType::Premium  
        } else if caller_number.contains("trial") {
            CustomerType::Trial
        } else {
            CustomerType::Standard
        };
        
        // Determine priority (0 = highest, 255 = lowest)
        let priority = match customer_type {
            CustomerType::VIP => 0,
            CustomerType::Premium => 10,
            CustomerType::Standard => 50,
            CustomerType::Trial => 100,
        };
        
        // Determine required skills (would be more sophisticated in production)
        let required_skills = if caller_number.contains("support") {
            vec!["technical_support".to_string()]
        } else if caller_number.contains("sales") {
            vec!["sales".to_string()]
        } else if caller_number.contains("billing") {
            vec!["billing".to_string()]
        } else {
            vec!["general".to_string()]
        };
        
        debug!("📊 Customer analysis - Type: {:?}, Priority: {}, Skills: {:?}", 
               customer_type, priority, required_skills);
        
        (customer_type, priority, required_skills)
    }
    
    /// **PHASE 2**: Make intelligent routing decision based on multiple factors
    async fn make_routing_decision(
        &self,
        session_id: &SessionId,
        customer_type: &CustomerType,
        priority: u8,
        required_skills: &[String],
    ) -> CallCenterResult<RoutingDecision> {
        
        // **STEP 1**: Try to find available agents with matching skills
        if let Some(agent_id) = self.find_best_available_agent(required_skills, priority).await {
            return Ok(RoutingDecision::DirectToAgent {
                agent_id,
                reason: "Skilled agent available".to_string(),
            });
        }
        
        // **STEP 2**: Check if we should queue based on customer type and current load
        let queue_decision = self.determine_queue_strategy(customer_type, priority, required_skills).await;
        
        // **STEP 3**: Check for overflow conditions
        if self.should_overflow_call(customer_type, priority).await {
            return Ok(RoutingDecision::Overflow {
                target_queue: "overflow".to_string(),
                reason: "Primary queues full".to_string(),
            });
        }
        
        // **STEP 4**: Default to queueing with appropriate queue selection
        Ok(queue_decision)
    }
    
    /// **PHASE 2**: Find the best available agent based on skills and performance
    async fn find_best_available_agent(&self, required_skills: &[String], priority: u8) -> Option<AgentId> {
        let available_agents = self.available_agents.read().await;
        
        // Find agents with matching skills and availability
        let mut suitable_agents: Vec<(&AgentId, &AgentInfo)> = available_agents
            .iter()
            .filter(|(_, agent_info)| {
                // Check if agent is available
                matches!(agent_info.status, AgentStatus::Available) &&
                // Check if agent has capacity
                agent_info.current_calls < agent_info.max_calls &&
                // Check skill match (if no specific skills required, any agent works)
                (required_skills.is_empty() || 
                 required_skills.iter().any(|skill| agent_info.skills.contains(skill)))
            })
            .collect();
        
        if suitable_agents.is_empty() {
            debug!("❌ No suitable agents found for skills: {:?}", required_skills);
            return None;
        }
        
        // Sort by performance score and last call end time (round-robin effect)
        suitable_agents.sort_by(|a, b| {
            // Primary: performance score (higher is better)
            let score_cmp = b.1.performance_score.partial_cmp(&a.1.performance_score).unwrap_or(std::cmp::Ordering::Equal);
            if score_cmp != std::cmp::Ordering::Equal {
                return score_cmp;
            }
            
            // Secondary: longest idle time (for round-robin)
            match (&a.1.last_call_end, &b.1.last_call_end) {
                (Some(a_end), Some(b_end)) => a_end.cmp(b_end), // Earlier end time first
                (None, Some(_)) => std::cmp::Ordering::Less,     // Never handled call first
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
        
        let best_agent = suitable_agents.first().map(|(agent_id, _)| (*agent_id).clone());
        
        if let Some(ref agent_id) = best_agent {
            info!("🎯 Selected agent {} for skills {:?} (priority {})", agent_id, required_skills, priority);
        }
        
        best_agent
    }
    
    /// **PHASE 2**: Determine appropriate queue strategy
    async fn determine_queue_strategy(
        &self,
        customer_type: &CustomerType,
        priority: u8,
        required_skills: &[String],
    ) -> RoutingDecision {
        
        // Select queue based on skills and customer type
        let queue_id = if required_skills.contains(&"technical_support".to_string()) {
            "support"
        } else if required_skills.contains(&"sales".to_string()) {
            "sales"
        } else if required_skills.contains(&"billing".to_string()) {
            "billing"
        } else {
            match customer_type {
                CustomerType::VIP => "vip",
                CustomerType::Premium => "premium",
                _ => "general",
            }
        };
        
        RoutingDecision::Queue {
            queue_id: queue_id.to_string(),
            priority,
            reason: format!("Queue selected for {} customer with skills {:?}", 
                          format!("{:?}", customer_type).to_lowercase(), required_skills),
        }
    }
    
    /// **PHASE 2**: Check if call should be overflowed to alternate routing
    async fn should_overflow_call(&self, customer_type: &CustomerType, priority: u8) -> bool {
        // **FUTURE**: Implement sophisticated overflow logic
        // For now, simple check based on queue lengths
        
        let queue_manager = self.queue_manager.read().await;
        
        // Check total queue load (simplified)
        // In production, this would check specific queue capacities, wait times, etc.
        
        false // For now, don't overflow
    }
    
    /// **PHASE 2**: Ensure a queue exists, create if necessary
    async fn ensure_queue_exists(&self, queue_id: &str) -> CallCenterResult<()> {
        let mut queue_manager = self.queue_manager.write().await;
        
        // Check if queue exists (this is a simplified check)
        // In production, we'd have better queue existence checking
        
        // Create standard queues if they don't exist
        let standard_queues = vec![
            ("general", "General Support", 100),
            ("sales", "Sales", 50),
            ("support", "Technical Support", 75),
            ("billing", "Billing", 30),
            ("vip", "VIP Support", 20),
            ("premium", "Premium Support", 40),
            ("overflow", "Overflow Queue", 200),
        ];
        
        for (id, name, max_size) in standard_queues {
            if id == queue_id {
                // Try to create queue (will succeed if doesn't exist)
                let _ = queue_manager.create_queue(id.to_string(), name.to_string(), max_size);
                break;
            }
        }
        
        Ok(())
    }
    
    /// **PHASE 2**: Monitor queue for agent availability
    async fn monitor_queue_for_agents(&self, queue_id: String) {
        // Spawn background task to monitor queue and assign agents when available
        let engine = Arc::new(self.clone());
        tokio::spawn(async move {
            // **FUTURE**: Implement intelligent queue monitoring
            // For now, just log that we're monitoring
            debug!("👁️ Monitoring queue {} for agent availability", queue_id);
            
            // This would periodically check for available agents and dequeue calls
            // Implementation would go here...
        });
    }
    
    /// **PHASE 2**: Assign a specific agent to an incoming call
    async fn assign_specific_agent_to_call(&self, session_id: SessionId, agent_id: AgentId) -> CallCenterResult<()> {
        info!("🎯 Assigning specific agent {} to call: {}", agent_id, session_id);
        
        // Get agent information and update status
        let agent_info = {
            let mut available_agents = self.available_agents.write().await;
            if let Some(mut agent_info) = available_agents.remove(&agent_id) {
                agent_info.status = AgentStatus::Busy { active_calls: (agent_info.current_calls + 1) as u32 };
                agent_info.current_calls += 1;
                Some(agent_info)
            } else {
                return Err(CallCenterError::orchestration(&format!("Agent {} not available", agent_id)));
            }
        };
        
        if let Some(agent_info) = agent_info {
            // **REAL**: Bridge customer and agent using session-core API
            match self.server_manager.bridge_sessions(&session_id, &agent_info.session_id).await {
                Ok(bridge_id) => {
                    info!("✅ Successfully bridged customer {} with agent {} (bridge: {})", 
                          session_id, agent_id, bridge_id);
                    
                    // Update call info
                    {
                        let mut active_calls = self.active_calls.write().await;
                        if let Some(call_info) = active_calls.get_mut(&session_id) {
                            call_info.agent_id = Some(agent_id.clone());
                            call_info.bridge_id = Some(bridge_id);
                            call_info.status = CallStatus::Bridged;
                            call_info.answered_at = Some(chrono::Utc::now());
                        }
                    }
                    
                    // Update agent status (keep as busy, increment call count)
                    {
                        let mut available_agents = self.available_agents.write().await;
                        available_agents.insert(agent_id, agent_info);
                    }
                },
                Err(e) => {
                    error!("Failed to bridge sessions: {}", e);
                    
                    // Return agent to available pool with original status
                    {
                        let mut available_agents = self.available_agents.write().await;
                        let mut restored_agent = agent_info;
                        restored_agent.status = AgentStatus::Available;
                        restored_agent.current_calls = restored_agent.current_calls.saturating_sub(1);
                        available_agents.insert(agent_id, restored_agent);
                    }
                    
                    return Err(CallCenterError::orchestration(&format!("Bridge failed: {}", e)));
                }
            }
        }
        
        Ok(())
    }
    
    /// **ENHANCED**: Handle call termination cleanup with agent status management
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
                info!("🔄 Returning agent {} to available pool after call completion", agent_id);
                
                // Update agent status
                {
                    let mut available_agents = self.available_agents.write().await;
                    if let Some(agent_info) = available_agents.get_mut(&agent_id) {
                        agent_info.current_calls = agent_info.current_calls.saturating_sub(1);
                        agent_info.last_call_end = Some(chrono::Utc::now());
                        
                        // If agent has no active calls, mark as available
                        if agent_info.current_calls == 0 {
                            agent_info.status = AgentStatus::Available;
                            info!("✅ Agent {} is now available for new calls", agent_id);
                        }
                        
                        // Update performance score based on call duration (simplified)
                        if let Some(answered_at) = call_info.answered_at {
                            let call_duration = chrono::Utc::now().signed_duration_since(answered_at).num_seconds();
                            // Simple performance scoring: reasonable call duration improves score
                            if call_duration > 30 && call_duration < 1800 { // 30s to 30min
                                agent_info.performance_score = (agent_info.performance_score + 0.1).min(1.0);
                            }
                        }
                    }
                }
                
                // Check if there are queued calls that can be assigned to this agent
                self.try_assign_queued_calls_to_agent(agent_id).await;
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
    
    /// **PHASE 2**: Try to assign queued calls to a newly available agent
    async fn try_assign_queued_calls_to_agent(&self, agent_id: AgentId) {
        debug!("🔍 Checking queued calls for newly available agent {}", agent_id);
        
        // Get agent skills to find matching queued calls
        let agent_skills = {
            let available_agents = self.available_agents.read().await;
            available_agents.get(&agent_id)
                .map(|info| info.skills.clone())
                .unwrap_or_default()
        };
        
        // Check relevant queues for calls that match agent skills
        let queues_to_check = vec!["general", "sales", "support", "billing", "vip", "premium"];
        
        for queue_id in queues_to_check {
            // Try to dequeue a call from this queue
            let queued_call = {
                let mut queue_manager = self.queue_manager.write().await;
                queue_manager.dequeue_for_agent(queue_id).unwrap_or(None)
            };
            
            if let Some(queued_call) = queued_call {
                info!("📤 Dequeued call {} from queue {} for agent {}", 
                      queued_call.session_id, queue_id, agent_id);
                
                // Assign the queued call to this agent
                let session_id = queued_call.session_id.clone();
                let agent_id_clone = agent_id.clone();
                let engine = Arc::new(self.clone());
                
                tokio::spawn(async move {
                    if let Err(e) = engine.assign_specific_agent_to_call(session_id, agent_id_clone).await {
                        error!("Failed to assign queued call to agent: {}", e);
                    }
                });
                
                break; // Only assign one call at a time
            }
        }
    }
    
    /// **ENHANCED**: Register an agent with skills and performance tracking
    pub async fn register_agent(&self, agent: &Agent) -> CallCenterResult<SessionId> {
        info!("👤 Registering agent {} with session-core: {} (skills: {:?})", 
              agent.id, agent.sip_uri, agent.skills);
        
        // **REAL**: Create outgoing session for agent registration
        let agent_session = self.server_manager
            .session_manager()
            .create_outgoing_session()
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create agent session: {}", e)))?;
        
        let session_id = agent_session.id.clone();
        
        // Add agent to available pool with enhanced information
        {
            let mut available_agents = self.available_agents.write().await;
            available_agents.insert(agent.id.clone(), AgentInfo {
                agent_id: agent.id.clone(),
                session_id: session_id.clone(),
                status: AgentStatus::Available,
                skills: agent.skills.clone(),
                current_calls: 0,
                max_calls: agent.max_concurrent_calls as usize,
                last_call_end: None,
                performance_score: 0.5, // Start with neutral performance
            });
        }
        
        info!("✅ Agent {} registered with session-core (session: {}, max calls: {})", 
              agent.id, session_id, agent.max_concurrent_calls);
        Ok(session_id)
    }
    
    /// **PHASE 2**: Update agent status (Available, Busy, Away, etc.)
    pub async fn update_agent_status(&self, agent_id: &AgentId, new_status: AgentStatus) -> CallCenterResult<()> {
        info!("🔄 Updating agent {} status to {:?}", agent_id, new_status);
        
        let mut available_agents = self.available_agents.write().await;
        if let Some(agent_info) = available_agents.get_mut(agent_id) {
            let old_status = agent_info.status.clone();
            agent_info.status = new_status.clone();
            
            info!("✅ Agent {} status updated from {:?} to {:?}", agent_id, old_status, new_status);
            
            // If agent became available, check for queued calls
            if matches!(new_status, AgentStatus::Available) && agent_info.current_calls == 0 {
                let agent_id_clone = agent_id.clone();
                let engine = Arc::new(self.clone());
                tokio::spawn(async move {
                    engine.try_assign_queued_calls_to_agent(agent_id_clone).await;
                });
            }
            
            Ok(())
        } else {
            Err(CallCenterError::not_found(format!("Agent not found: {}", agent_id)))
        }
    }
    
    /// **PHASE 2**: Get detailed agent information
    pub async fn get_agent_info(&self, agent_id: &AgentId) -> Option<AgentInfo> {
        let available_agents = self.available_agents.read().await;
        available_agents.get(agent_id).cloned()
    }
    
    /// **PHASE 2**: List all agents with their current status
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        let available_agents = self.available_agents.read().await;
        available_agents.values().cloned().collect()
    }
    
    /// **PHASE 2**: Get queue statistics for monitoring
    pub async fn get_queue_stats(&self) -> CallCenterResult<Vec<(String, QueueStats)>> {
        let queue_manager = self.queue_manager.read().await;
        let queue_ids = vec!["general", "sales", "support", "billing", "vip", "premium", "overflow"];
        
        let mut stats = Vec::new();
        for queue_id in queue_ids {
            if let Ok(queue_stat) = queue_manager.get_queue_stats(queue_id) {
                stats.push((queue_id.to_string(), queue_stat));
            }
        }
        
        Ok(stats)
    }
    
    /// **NEW API**: Register an agent and make them available
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
            .session_id.clone();
        
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
    
    /// **ENHANCED**: Get orchestrator statistics with Phase 2 details
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
            queue_manager: self.queue_manager.clone(),
            bridge_events: None, // Don't clone the receiver
            active_calls: self.active_calls.clone(),
            available_agents: self.available_agents.clone(),
            routing_stats: self.routing_stats.clone(),
        }
    }
} 