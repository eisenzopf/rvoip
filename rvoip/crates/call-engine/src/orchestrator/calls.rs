//! Call handling logic for the call center
//!
//! This module implements the core call processing functionality including
//! incoming call handling, agent assignment, and call lifecycle management.

use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, debug, warn, error};
use rvoip_session_core::{IncomingCall, CallDecision, SessionId, SessionControl};

use crate::agent::{AgentId, AgentStatus};
use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::queue::{QueuedCall, QueueStats};
use super::core::CallCenterEngine;
use super::types::{CallInfo, CallStatus, RoutingDecision};

impl CallCenterEngine {
    /// Process incoming call with sophisticated routing
    pub(super) async fn process_incoming_call(&self, call: IncomingCall) -> CallCenterResult<CallDecision> {
        let session_id = call.id.clone();
        let routing_start = std::time::Instant::now();
        
        info!("📞 Processing incoming call: {} from {}", session_id, call.from);
        
        // Analyze customer information and determine routing requirements
        let (customer_type, priority, required_skills) = self.analyze_customer_info(&call).await;
        
        // Create enhanced call info tracking
        let call_info = CallInfo {
            session_id: session_id.clone(),
            caller_id: call.from.clone(),
            from: call.from.clone(),
            to: call.to.clone(),
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
        
        // Make intelligent routing decision based on multiple factors
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
                let engine = self.session_coordinator.as_ref()
                    .ok_or_else(|| CallCenterError::orchestration("Session coordinator not initialized"))?
                    .clone();
                let session_id_clone = session_id.clone();
                let agent_id_clone = agent_id.clone();
                let self_clone = self.clone();
                tokio::spawn(async move {
                    // Wait briefly for the call to be accepted at SIP level
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    if let Err(e) = self_clone.assign_specific_agent_to_call(session_id_clone, agent_id_clone).await {
                        error!("Failed to assign specific agent to call: {}", e);
                    }
                });
                
                // Update routing stats
                {
                    let mut stats = self.routing_stats.write().await;
                    stats.calls_routed_directly += 1;
                }
                
                // Return Accept to send 200 OK and establish the customer's call
                CallDecision::Accept(None)
            },
            
            RoutingDecision::Queue { queue_id, priority, reason } => {
                info!("📋 Queueing call {} in queue {} with priority {} ({})", session_id, queue_id, priority, reason);
                
                // Add call to queue
                let queued_call = QueuedCall {
                    session_id: session_id.clone(),
                    caller_id: call.from.clone(),
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
                        return Ok(CallDecision::Reject("Queue full".to_string()));
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
                
                // Return Defer to send 180 Ringing, not Accept which sends 200 OK
                CallDecision::Defer
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
                // Return Defer, not Accept
                CallDecision::Defer
            },
            
            RoutingDecision::Reject { reason } => {
                warn!("❌ Rejecting call {} ({})", session_id, reason);
                
                // Update routing stats
                {
                    let mut stats = self.routing_stats.write().await;
                    stats.calls_rejected += 1;
                }
                
                CallDecision::Reject(reason)
            },
            
            RoutingDecision::Conference { bridge_id } => {
                info!("🎤 Routing call {} to conference {}", session_id, bridge_id);
                // TODO: Implement conference routing
                CallDecision::Accept(None)
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
    
    /// Assign a specific agent to an incoming call
    pub(super) async fn assign_specific_agent_to_call(&self, session_id: SessionId, agent_id: AgentId) -> CallCenterResult<()> {
        info!("🎯 Assigning specific agent {} to call: {}", agent_id, session_id);
        
        // Get agent information and update status
        let agent_info = if let Some((_key, mut agent_info)) = self.available_agents.remove(&agent_id) {
            agent_info.status = AgentStatus::Busy { active_calls: (agent_info.current_calls + 1) as u32 };
            agent_info.current_calls += 1;
            Some(agent_info)
        } else {
            return Err(CallCenterError::orchestration(&format!("Agent {} not available", agent_id)));
        };
        
        if let Some(agent_info) = agent_info {
            let coordinator = self.session_coordinator.as_ref().unwrap();
            
            // The customer call should already be accepted at this point (200 OK sent)
            // Now we just need to call the agent and bridge them
            
            // Step 1: Create an outgoing call to the agent
            let agent_uri = agent_info.contact_uri.clone(); // Use the contact URI from REGISTER
            info!("📞 Creating outgoing call to agent {} at {}", agent_id, agent_uri);
            
            // Use the configured domain for the From URI
            let from_uri = format!("sip:call-center@{}", self.config.general.domain);
            
            let agent_call_session = match coordinator.create_outgoing_call(
                &agent_uri,
                &from_uri,
                None, // No specific SDP
            ).await {
                Ok(call_session) => {
                    info!("✅ Created outgoing call {:?} to agent {}", call_session.id, agent_id);
                    call_session
                }
                Err(e) => {
                    error!("Failed to create outgoing call to agent {}: {}", agent_id, e);
                    // TODO: Hang up the customer call or re-queue it
                    let mut restored_agent = agent_info;
                    restored_agent.status = AgentStatus::Available;
                    restored_agent.current_calls = restored_agent.current_calls.saturating_sub(1);
                    self.available_agents.insert(agent_id, restored_agent);
                    return Err(CallCenterError::orchestration(&format!("Failed to call agent: {}", e)));
                }
            };
            
            // Get the session ID from the CallSession
            let agent_session_id = agent_call_session.id.clone();
            
            // Update agent info with the new session ID
            let mut updated_agent = agent_info;
            updated_agent.session_id = agent_session_id.clone();
            
            // Step 3: Wait a moment for the agent call to establish
            // In a real system, we'd wait for the agent to answer (200 OK from agent)
            // For now, we'll proceed immediately to bridging
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            
            // Step 4: Bridge the customer and agent calls
            info!("🌉 Bridging customer {} with agent {} (session {:?})", 
                  session_id, agent_id, agent_session_id);
            
            match coordinator.bridge_sessions(&session_id, &agent_session_id).await {
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
                    
                    // Store updated agent info
                    self.available_agents.insert(agent_id, updated_agent);
                },
                Err(e) => {
                    error!("Failed to bridge sessions: {}", e);
                    
                    // Hang up the agent call
                    let _ = coordinator.terminate_session(&agent_session_id).await;
                    
                    // Return agent to available pool
                    updated_agent.status = AgentStatus::Available;
                    updated_agent.current_calls = updated_agent.current_calls.saturating_sub(1);
                    self.available_agents.insert(agent_id, updated_agent);
                    
                    return Err(CallCenterError::orchestration(&format!("Bridge failed: {}", e)));
                }
            }
        }
        
        Ok(())
    }
    
    /// Update call state when call is established
    pub(super) async fn update_call_established(&self, session_id: SessionId) {
        let mut active_calls = self.active_calls.write().await;
        if let Some(call_info) = active_calls.get_mut(&session_id) {
            call_info.status = CallStatus::Bridged;
            call_info.answered_at = Some(chrono::Utc::now());
            info!("📞 Call {} marked as established/bridged", session_id);
        }
    }
    
    /// Handle call termination cleanup with agent status management
    pub(super) async fn handle_call_termination(&self, session_id: SessionId) -> CallCenterResult<()> {
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
                if let Some(mut agent_info) = self.available_agents.get_mut(&agent_id) {
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
                
                // Check if there are queued calls that can be assigned to this agent
                self.try_assign_queued_calls_to_agent(agent_id).await;
            }
            
            // If call had a bridge, clean it up
            if let Some(bridge_id) = call_info.bridge_id {
                if let Err(e) = self.session_coordinator.as_ref().unwrap().destroy_bridge(&bridge_id).await {
                    warn!("Failed to destroy bridge {}: {}", bridge_id, e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Try to assign queued calls to a newly available agent
    pub(super) async fn try_assign_queued_calls_to_agent(&self, agent_id: AgentId) {
        debug!("🔍 Checking queued calls for newly available agent {}", agent_id);
        
        // Get agent skills to find matching queued calls
        let agent_skills = self.available_agents.get(&agent_id)
            .map(|entry| entry.skills.clone())
            .unwrap_or_default();
        
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
} 