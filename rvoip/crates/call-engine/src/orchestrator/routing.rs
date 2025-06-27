//! Call routing logic for the call center
//!
//! This module implements the sophisticated routing engine that determines
//! how incoming calls are distributed to agents based on skills, availability,
//! and business rules.

use std::sync::Arc;
use tracing::{debug, info, error, warn};
use rvoip_session_core::{IncomingCall, SessionId};

use crate::agent::{AgentId, AgentStatus};
use crate::error::Result as CallCenterResult;
use crate::queue::QueuedCall;
use super::core::CallCenterEngine;
use super::types::{CustomerType, RoutingDecision, AgentInfo};

impl CallCenterEngine {
    /// Analyze customer information to determine routing requirements
    pub(super) async fn analyze_customer_info(&self, call: &IncomingCall) -> (CustomerType, u8, Vec<String>) {
        // This would integrate with CRM systems, customer databases, etc.
        // For now, use simple heuristics based on caller information
        
        let caller_number = &call.from;
        
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
    
    /// Make intelligent routing decision based on multiple factors
    pub(super) async fn make_routing_decision(
        &self,
        session_id: &SessionId,
        customer_type: &CustomerType,
        priority: u8,
        required_skills: &[String],
    ) -> CallCenterResult<RoutingDecision> {
        
        // PHASE 0.10: Queue-First Routing - Always queue calls instead of direct-to-agent
        // This ensures all calls go through the queue for fair distribution
        
        // **DISABLED FOR QUEUE-FIRST**: Try to find available agents with matching skills
        // if let Some(agent_id) = self.find_best_available_agent(required_skills, priority).await {
        //     return Ok(RoutingDecision::DirectToAgent {
        //         agent_id,
        //         reason: "Skilled agent available".to_string(),
        //     });
        // }
        
        info!("🚦 Queue-First Routing: Sending call {} to queue (priority: {})", session_id, priority);
        
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
    
    /// Find the best available agent based on skills and performance
    pub(super) async fn find_best_available_agent(&self, required_skills: &[String], priority: u8) -> Option<AgentId> {
        // Get available agents from database
        let db_manager = self.db_manager.as_ref()?;
        
        let mut suitable_agents = match db_manager.get_available_agents().await {
            Ok(agents) => agents
                .into_iter()
                .filter(|agent| {
                    // Filter by skills if specific skills are required
                    // TODO: Add skills table and filtering in database
                    required_skills.is_empty() || required_skills.contains(&"general".to_string())
                })
                .collect::<Vec<_>>(),
            Err(e) => {
                error!("Failed to get available agents from database: {}", e);
                return None;
            }
        };
        
        if suitable_agents.is_empty() {
            debug!("❌ No suitable agents found for skills: {:?}", required_skills);
            return None;
        }
        
        // Sort by current_calls (ascending) for load balancing
        suitable_agents.sort_by_key(|agent| agent.current_calls);
        
        let best_agent = suitable_agents.first().map(|agent| AgentId::from(agent.agent_id.clone()));
        
        if let Some(ref agent_id) = best_agent {
            info!("🎯 Selected agent {} for skills {:?} (priority {})", agent_id, required_skills, priority);
        }
        
        best_agent
    }
    
    /// Determine appropriate queue strategy
    pub(super) async fn determine_queue_strategy(
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
    
    /// Check if call should be overflowed to alternate routing
    pub(super) async fn should_overflow_call(&self, customer_type: &CustomerType, priority: u8) -> bool {
        // **FUTURE**: Implement sophisticated overflow logic
        // For now, simple check based on queue lengths
        
        let queue_manager = self.queue_manager.read().await;
        
        // Check total queue load (simplified)
        // In production, this would check specific queue capacities, wait times, etc.
        
        false // For now, don't overflow
    }
    
    /// Ensure a queue exists, create if necessary
    pub(super) async fn ensure_queue_exists(&self, queue_id: &str) -> CallCenterResult<()> {
        let mut queue_manager = self.queue_manager.write().await;
        
        // Try to get queue stats to see if it exists
        if queue_manager.get_queue_stats(queue_id).is_ok() {
            // Queue already exists
            return Ok(());
        }
        
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
                // Create the queue
                queue_manager.create_queue(id.to_string(), name.to_string(), max_size)?;
                info!("📋 Auto-created queue '{}' ({})", id, name);
                break;
            }
        }
        
        Ok(())
    }
    
    /// Get the current depth of a queue
    pub async fn get_queue_depth(&self, queue_id: &str) -> usize {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.get_queue_depth(queue_id).await {
                Ok(depth) => depth,
                Err(e) => {
                    error!("Failed to get queue depth from database: {}", e);
                    // Fallback to in-memory
                    self.get_in_memory_queue_depth(queue_id).await
                }
            }
        } else {
            self.get_in_memory_queue_depth(queue_id).await
        }
    }
    
    /// Get queue depth from in-memory manager
    async fn get_in_memory_queue_depth(&self, queue_id: &str) -> usize {
        let queue_manager = self.queue_manager.read().await;
        queue_manager.get_queue_stats(queue_id)
            .map(|stats| stats.total_calls)
            .unwrap_or(0)
    }
    
    /// Get list of available agents (excludes agents in post-call wrap-up)
    async fn get_available_agents(&self) -> Vec<AgentId> {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.get_available_agents().await {
                Ok(agents) => agents
                    .into_iter()
                    .map(|agent| AgentId::from(agent.agent_id))
                    .collect(),
                Err(e) => {
                    error!("Failed to get available agents from database: {}", e);
                    vec![]
                }
            }
        } else {
            vec![]
        }
    }
    
    /// Process database assignments
    async fn process_database_assignments(&self, queue_id: &str) -> CallCenterResult<()> {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.get_available_assignments(queue_id).await {
                Ok(assignments) => {
                    info!("📋 Database found {} optimal assignments for queue {}", 
                          assignments.len(), queue_id);
                    
                    for (agent_id_str, call_id, session_id_str) in assignments {
                        let agent_id = AgentId::from(agent_id_str);
                        let session_id = SessionId(session_id_str);
                        
                        // Process assignment asynchronously
                        let engine = Arc::new(self.clone());
                        tokio::spawn(async move {
                            match engine.assign_specific_agent_to_call(session_id.clone(), agent_id.clone()).await {
                                Ok(()) => {
                                    info!("✅ Successfully assigned call {} to agent {}", session_id, agent_id);
                                }
                                Err(e) => {
                                    error!("Failed to assign call {} to agent {}: {}", session_id, agent_id, e);
                                    // Database will handle re-queuing if needed
                                }
                            }
                        });
                    }
                }
                Err(e) => {
                    error!("Failed to get assignments from database: {}", e);
                    return Err(crate::error::CallCenterError::internal(
                        format!("Database assignment error: {}", e)
                    ));
                }
            }
        }
        Ok(())
    }
    
    /// Process a single in-memory assignment
    async fn process_in_memory_assignment(
        &self,
        queue_id: &str,
        agent_id: &AgentId,
    ) -> Option<QueuedCall> {
        // Check if this agent is still actually available from database
        let agent_still_available = if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&agent_id.0).await {
                Ok(Some(agent)) => {
                    // Only consider agents with Available status (not Busy or PostCallWrapUp)
                    matches!(agent.status, crate::database::DbAgentStatus::Available) &&
                    agent.current_calls < agent.max_calls
                }
                _ => false
            }
        } else {
            false
        };
        
        if !agent_still_available {
            debug!("Agent {} no longer available, skipping", agent_id);
            return None;
        }
        
        // Try to dequeue a call
        let mut queue_manager = self.queue_manager.write().await;
        queue_manager.dequeue_for_agent(queue_id).unwrap_or(None)
    }
    
    /// Atomically try to assign a call to a specific agent
    /// Returns the dequeued call only if the agent was successfully reserved
    async fn try_assign_to_specific_agent(
        &self,
        queue_id: &str,
        agent_id: &AgentId,
    ) -> Option<QueuedCall> {
        let db_manager = self.db_manager.as_ref()?;
        
        // First, try to atomically reserve the agent in the database
        let agent_reserved = match db_manager.reserve_agent(&agent_id.0).await {
            Ok(reserved) => {
                if reserved {
                    info!("🔒 Reserved agent {} for assignment", agent_id);
                    true
                } else {
                    debug!("Could not reserve agent {} - already busy or unavailable", agent_id);
                    false
                }
            }
            Err(e) => {
                error!("Failed to reserve agent {} in database: {}", agent_id, e);
                false
            }
        };
        
        if !agent_reserved {
            return None;
        }
        
        // Agent is reserved, now try to dequeue a call
        let mut queue_manager = self.queue_manager.write().await;
        match queue_manager.dequeue_for_agent(queue_id) {
            Ok(Some(call)) => {
                info!("✅ Dequeued call {} for reserved agent {}", call.session_id, agent_id);
                
                // Update agent status to BUSY and increment call count
                if let Err(e) = db_manager.update_agent_status(&agent_id.0, AgentStatus::Busy(vec![])).await {
                    error!("Failed to update agent status to BUSY: {}", e);
                }
                if let Err(e) = db_manager.update_agent_call_count(&agent_id.0, 1).await {
                    error!("Failed to increment agent call count: {}", e);
                }
                
                Some(call)
            }
            Ok(None) => {
                // No calls in queue, release the agent
                warn!("No calls in queue {} despite monitor check, releasing agent {}", queue_id, agent_id);
                drop(queue_manager); // Release lock before updating agent
                
                // Release the agent reservation in database
                if let Err(e) = db_manager.release_agent_reservation(&agent_id.0).await {
                    error!("Failed to release agent reservation in database: {}", e);
                }
                info!("🔓 Released agent {} reservation (no calls to assign)", agent_id);
                None
            }
            Err(e) => {
                error!("Failed to dequeue for agent {}: {}", agent_id, e);
                drop(queue_manager); // Release lock before updating agent
                
                // Release the agent reservation on error
                if let Err(e) = db_manager.release_agent_reservation(&agent_id.0).await {
                    error!("Failed to release agent reservation in database: {}", e);
                }
                info!("🔓 Released agent {} reservation (dequeue error)", agent_id);
                None
            }
        }
    }
    
    /// Handle assignment of a queued call to an agent
    async fn handle_call_assignment(
        engine: Arc<CallCenterEngine>,
        queue_id: String,
        queued_call: QueuedCall,
        agent_id: AgentId,
    ) {
        let session_id = queued_call.session_id.clone();
        
        match engine.assign_specific_agent_to_call(session_id.clone(), agent_id.clone()).await {
            Ok(()) => {
                info!("✅ Successfully assigned queued call {} to agent {}", session_id, agent_id);
                // Mark as no longer being assigned
                let mut queue_manager = engine.queue_manager.write().await;
                queue_manager.mark_as_not_assigned(&session_id);
            }
            Err(e) => {
                error!("Failed to assign call {} to agent {}: {}", session_id, agent_id, e);
                
                // The agent was already restored by assign_specific_agent_to_call on failure
                // We just need to handle the call re-queuing
                
                // Mark as no longer being assigned
                let mut queue_manager = engine.queue_manager.write().await;
                queue_manager.mark_as_not_assigned(&session_id);
                
                // Check if call is still active
                let call_still_active = engine.active_calls.contains_key(&session_id);
                if !call_still_active {
                    warn!("Call {} is no longer active, not re-queuing", session_id);
                    return;
                }
                
                // Re-queue the call with higher priority
                let mut requeued_call = queued_call;
                requeued_call.priority = requeued_call.priority.saturating_sub(5); // Increase priority
                
                if let Err(e) = queue_manager.enqueue_call(&queue_id, requeued_call) {
                    error!("Failed to re-queue call {}: {}", session_id, e);
                } else {
                    info!("📞 Re-queued call {} with higher priority", session_id);
                    
                    // Update call status back to queued
                    if let Some(mut call_info) = engine.active_calls.get_mut(&session_id) {
                        call_info.status = super::types::CallStatus::Queued;
                        call_info.queue_id = Some(queue_id);
                    }
                }
            }
        }
    }
    
    /// Monitor queue for agent availability
    pub async fn monitor_queue_for_agents(&self, queue_id: String) {
        // Check if queue has calls before starting monitor
        let initial_queue_size = self.get_queue_depth(&queue_id).await;
        
        if initial_queue_size == 0 {
            debug!("Queue {} is empty, not starting monitor", queue_id);
            return;
        }
        
        // Spawn background task to monitor queue and assign agents when available
        let engine = Arc::new(self.clone());
        tokio::spawn(async move {
            // Check if already monitoring this queue
            if !engine.active_queue_monitors.insert(queue_id.clone()) {
                info!("🔄 Queue monitor already active for {}, skipping duplicate", queue_id);
                return;
            }
            
            info!("👁️ Starting queue monitor for queue: {} (initial size: {})", queue_id, initial_queue_size);
            
            // BATCHING DELAY: Wait 2 seconds to allow multiple calls to accumulate
            // This enables fair round robin distribution instead of serial processing
            info!("⏱️ BATCHING: Waiting 2 seconds to accumulate calls for fair distribution");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            
            // Check queue size after batching delay
            let batched_queue_size = engine.get_queue_depth(&queue_id).await;
            info!("📊 BATCHING: Queue '{}' size after 2s delay: {} calls (was {})", 
                  queue_id, batched_queue_size, initial_queue_size);
            
            // Monitor for 5 minutes max (to prevent orphaned tasks)
            let start_time = std::time::Instant::now();
            let max_duration = std::time::Duration::from_secs(300);
            
            // Dynamic check interval - starts at 1s, backs off when no agents available
            let mut check_interval_secs = 1u64;
            let mut consecutive_no_agents = 0u32;
            
            loop {
                // Wait with current interval
                tokio::time::sleep(std::time::Duration::from_secs(check_interval_secs)).await;
                
                // Check if we've exceeded max monitoring time
                if start_time.elapsed() > max_duration {
                    info!("⏰ Queue monitor for {} exceeded max duration, stopping", queue_id);
                    break;
                }
                
                // Check current queue size
                let queue_size = engine.get_queue_depth(&queue_id).await;
                
                if queue_size == 0 {
                    info!("✅ Queue {} is now empty, stopping monitor", queue_id);
                    break;
                }
                
                debug!("📊 Queue {} status: {} calls waiting", queue_id, queue_size);
                
                // Find available agents for this queue
                let available_agents = engine.get_available_agents().await;
                
                if available_agents.is_empty() {
                    consecutive_no_agents += 1;
                    // Exponential backoff when no agents available (max 30s)
                    check_interval_secs = (check_interval_secs * 2).min(30);
                    debug!("⏳ No available agents for queue {}, backing off to {}s interval", 
                          queue_id, check_interval_secs);
                    
                    // Clean up stuck assignments periodically
                    if consecutive_no_agents % 5 == 0 {  // Every 5 checks
                        let mut queue_manager = engine.queue_manager.write().await;
                        let stuck_calls = queue_manager.cleanup_stuck_assignments(30);  // 30 second timeout
                        if !stuck_calls.is_empty() {
                            info!("🧹 Cleaned up {} stuck assignments in queue {}", stuck_calls.len(), queue_id);
                        }
                    }
                    
                    continue;
                } else {
                    // Reset backoff when agents become available
                    consecutive_no_agents = 0;
                    check_interval_secs = 1;  // Fast check when agents available
                }
                
                info!("🎯 Found {} available agents for queue {}", available_agents.len(), queue_id);
                
                // Try to atomically assign calls to agents using database if available
                if engine.db_manager.is_some() {
                    match engine.process_database_assignments(&queue_id).await {
                        Ok(()) => {
                            // Check if assignments were made
                            let queue_size_after = engine.get_queue_depth(&queue_id).await;
                            if queue_size_after < queue_size {
                                info!("✅ Database assignments successful, queue {} reduced from {} to {} calls", 
                                      queue_id, queue_size, queue_size_after);
                            }
                            continue;  // Database handled assignments
                        }
                        Err(e) => {
                            // Log the error but continue with in-memory logic
                            warn!("Database assignment failed for queue {}: {}, falling back to in-memory", queue_id, e);
                            // Fall through to in-memory logic
                        }
                    }
                }
                
                // FIXED: Sequential assignment with last agent exclusion
                let mut assignments_made = 0;
                let mut last_assigned_agent: Option<String> = None;
                
                // Process calls one at a time to ensure fair round robin
                while assignments_made < queue_size && assignments_made < available_agents.len() {
                    // Get available agents excluding the last one assigned
                    let available_agents_ordered = if assignments_made == 0 {
                        // First assignment - use normal order
                        available_agents.clone()
                    } else {
                        // Subsequent assignments - exclude last assigned agent by moving to end
                        let mut filtered_agents = available_agents.clone();
                        if let Some(ref last_agent_id) = last_assigned_agent {
                            if let Some(pos) = filtered_agents.iter().position(|agent| agent.0 == *last_agent_id) {
                                let excluded_agent = filtered_agents.remove(pos);
                                filtered_agents.push(excluded_agent);
                                info!("🚫 Moved last assigned agent '{}' to end for fairness", last_agent_id);
                            }
                        }
                        filtered_agents
                    };
                    
                    // Log current assignment order for debugging
                    info!("🔄 ASSIGNMENT ORDER for call #{}: {:?}", 
                          assignments_made + 1, 
                          available_agents_ordered.iter().map(|a| &a.0).collect::<Vec<_>>());
                    
                    // Try to assign to the FIRST available agent in ordered list
                    let mut call_assigned = false;
                    for agent_id in &available_agents_ordered {
                        if let Some(queued_call) = engine.try_assign_to_specific_agent(&queue_id, agent_id).await {
                            info!("📤 SEQUENTIAL ASSIGNMENT #{}: call {} → agent {}", 
                                  assignments_made + 1, queued_call.session_id, agent_id);
                            
                            // Log queue depth after dequeue
                            let remaining_calls = engine.get_in_memory_queue_depth(&queue_id).await;
                            info!("📊 Queue '{}' status after assignment: {} calls remaining", 
                                  queue_id, remaining_calls);
                            
                            // Update call status to indicate it's being assigned
                            if let Some(mut call_info) = engine.active_calls.get_mut(&queued_call.session_id) {
                                call_info.status = super::types::CallStatus::Connecting;
                            }
                            
                            // Spawn task to handle the actual call setup
                            let engine_clone = engine.clone();
                            let queue_id_clone = queue_id.clone();
                            let agent_id_clone = agent_id.clone();
                            tokio::spawn(async move {
                                Self::handle_call_assignment(
                                    engine_clone,
                                    queue_id_clone,
                                    queued_call,
                                    agent_id_clone,
                                ).await;
                            });
                            
                            // Track this agent as last assigned for next iteration
                            last_assigned_agent = Some(agent_id.0.clone());
                            assignments_made += 1;
                            call_assigned = true;
                            
                            // Small delay to ensure status updates propagate
                            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                            
                            break; // Move to next call
                        }
                    }
                    
                    if !call_assigned {
                        // No more agents available for assignment
                        debug!("No more agents available for assignment in queue {}", queue_id);
                        break;
                    }
                }
                
                info!("🎯 FINAL RESULT: Made {} sequential assignments in queue {}", assignments_made, queue_id);
                
                if assignments_made == 0 && !available_agents.is_empty() {
                    debug!("No calls in queue {} despite having available agents", queue_id);
                }
            }
            
            // Remove from active monitors
            engine.active_queue_monitors.remove(&queue_id);
            info!("👁️ Queue monitor for {} stopped", queue_id);
        });
    }
} 