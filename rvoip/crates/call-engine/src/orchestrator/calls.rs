//! Call handling logic for the call center
//!
//! This module implements the core call processing functionality including
//! incoming call handling, agent assignment, and call lifecycle management.

use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, debug, warn, error};
use rvoip_session_core::{IncomingCall, CallDecision, SessionId, SessionControl, CallState};

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
        
        // Extract and log the SDP from the incoming call
        if let Some(ref sdp) = call.sdp {
            info!("📄 Incoming call has SDP offer ({} bytes)", sdp.len());
            debug!("📄 Customer SDP content:\n{}", sdp);
        } else {
            warn!("⚠️ Incoming call has no SDP offer");
        }
        
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
            customer_sdp: call.sdp.clone(),  // Store the customer's SDP
        };
        
        // Store call info
        self.active_calls.insert(session_id.clone(), call_info);
        
        // Make intelligent routing decision based on multiple factors
        let routing_decision = self.make_routing_decision(&session_id, &customer_type, priority, &required_skills).await?;
        
        info!("🎯 Routing decision for call {}: {:?}", session_id, routing_decision);
        
        // Execute routing decision
        let call_decision = match routing_decision {
            RoutingDecision::DirectToAgent { agent_id, reason } => {
                info!("📞 Routing call {} directly to agent {} ({})", session_id, agent_id, reason);
                
                // Update call status and assign agent
                if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                    call_info.status = CallStatus::Ringing;
                    call_info.agent_id = Some(agent_id.clone());
                }
                
                // Schedule immediate agent assignment
                let engine = self.session_coordinator.as_ref()
                    .ok_or_else(|| CallCenterError::orchestration("Session coordinator not initialized"))?
                    .clone();
                
                let self_clone = self.clone();
                let session_id_clone = session_id.clone();
                let agent_id_clone = agent_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = self_clone.assign_specific_agent_to_call(session_id_clone, agent_id_clone).await {
                        error!("Failed to assign agent: {}", e);
                    }
                });
                
                let routing_time = routing_start.elapsed().as_millis();
                info!("✅ Direct-to-agent routing decision made in {}ms", routing_time);
                
                // Accept the call immediately but without SDP (B2BUA behavior)
                // We'll update the customer's media session with agent's SDP after agent answers
                Ok(CallDecision::Accept(None))
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
                    retry_count: 0,  // New calls start with 0 retries
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
                if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                    call_info.status = CallStatus::Queued;
                    call_info.queue_id = Some(queue_id.clone());
                    call_info.queued_at = Some(chrono::Utc::now());
                }
                
                // Update routing stats
                {
                    let mut stats = self.routing_stats.write().await;
                    stats.calls_queued += 1;
                }
                
                // Start monitoring for agent availability
                self.monitor_queue_for_agents(queue_id.clone()).await;
                
                // Return Defer to send 180 Ringing, not Accept which sends 200 OK
                Ok(CallDecision::Defer)
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
                Ok(CallDecision::Defer)
            },
            
            RoutingDecision::Reject { reason } => {
                warn!("❌ Rejecting call {} ({})", session_id, reason);
                
                // Update routing stats
                {
                    let mut stats = self.routing_stats.write().await;
                    stats.calls_rejected += 1;
                }
                
                Ok(CallDecision::Reject(reason))
            },
            
            RoutingDecision::Conference { bridge_id } => {
                info!("🎤 Routing call {} to conference {}", session_id, bridge_id);
                // TODO: Implement conference routing
                Ok(CallDecision::Accept(None))
            }
        };
        
        // Update routing time metrics
        let routing_time = routing_start.elapsed().as_millis() as u64;
        {
            let mut stats = self.routing_stats.write().await;
            stats.average_routing_time_ms = (stats.average_routing_time_ms + routing_time) / 2;
        }
        
        info!("✅ Call {} routing completed in {}ms", session_id, routing_time);
        call_decision
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
            
            // Verify customer session is ready
            match coordinator.find_session(&session_id).await {
                Ok(Some(customer_session)) => {
                    info!("📞 Customer session {} is in state: {:?}", session_id, customer_session.state);
                    // Only proceed if customer call is in a suitable state
                    match customer_session.state {
                        CallState::Active | CallState::Ringing => {
                            // Good to proceed
                        }
                        _ => {
                            warn!("⚠️ Customer session is in unexpected state: {:?}", customer_session.state);
                        }
                    }
                }
                _ => {
                    warn!("⚠️ Could not find customer session {}", session_id);
                }
            }
            
            // Step 1: Get the customer's media info to pass SDP to the agent
            let customer_sdp = {
                // First try to get SDP from the call info (stored during incoming call processing)
                if let Some(call_info) = self.active_calls.get(&session_id) {
                    if let Some(ref sdp) = call_info.customer_sdp {
                        info!("📄 Retrieved customer SDP from call info ({} bytes)", sdp.len());
                        Some(sdp.clone())
                    } else {
                        // Fallback to media info if not in call info
                        match coordinator.get_media_info(&session_id).await {
                            Ok(Some(media_info)) => {
                                info!("📄 Retrieved customer media info from session");
                                media_info.remote_sdp.or(media_info.local_sdp)
                            }
                            Ok(None) => {
                                warn!("⚠️ No media info found for customer session");
                                None
                            }
                            Err(e) => {
                                warn!("⚠️ Failed to get customer media info: {}", e);
                                None
                            }
                        }
                    }
                } else {
                    warn!("⚠️ No call info found for session {}", session_id);
                    None
                }
            };
            
            if customer_sdp.is_none() {
                warn!("⚠️ No SDP available from customer session - agent will not receive media info");
            } else {
                info!("📄 Customer SDP length: {} bytes", customer_sdp.as_ref().unwrap().len());
                debug!("📄 Customer SDP content:\n{}", customer_sdp.as_ref().unwrap());
            }
            
            // Step 2: Create an outgoing call to the agent with customer's SDP
            let agent_contact_uri = agent_info.contact_uri.clone(); // Use the contact URI from REGISTER
            info!("📞 Creating outgoing call to agent {} at {} with SDP: {}", 
                  agent_id, agent_contact_uri, if customer_sdp.is_some() { "yes" } else { "no" });
            
            // Use the configured domain for the call center's From URI
            let call_center_uri = format!("sip:call-center@{}", self.config.general.domain);
            
            let agent_call_session = match coordinator.create_outgoing_call(
                &call_center_uri,    // FROM: The call center is making the call
                &agent_contact_uri,  // TO: The agent is receiving the call
                customer_sdp,        // Pass customer's SDP as the offer
            ).await {
                Ok(call_session) => {
                    info!("✅ Created outgoing call {:?} to agent {}", call_session.id, agent_id);
                    
                    // Track dialog relationship for B2BUA (customer → agent)
                    self.dialog_mappings.insert(session_id.0.clone(), call_session.id.0.clone());
                    self.dialog_mappings.insert(call_session.id.0.clone(), session_id.0.clone());
                    info!("📋 Tracked dialog mapping: {} ↔ {}", session_id, call_session.id);
                    
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
            
            // Step 3: Wait for the agent to answer the call
            info!("⏳ Waiting for agent {} to answer...", agent_id);
            
            match coordinator.wait_for_answer(&agent_session_id, std::time::Duration::from_secs(30)).await {
                Ok(_) => {
                    info!("✅ Agent {} answered the call", agent_id);
                    
                    // Get agent's SDP from their 200 OK response
                    let agent_sdp = match coordinator.get_media_info(&agent_session_id).await {
                        Ok(Some(media_info)) => {
                            info!("📄 Retrieved agent's SDP answer");
                            media_info.remote_sdp.or(media_info.local_sdp)
                        }
                        Ok(None) => {
                            warn!("⚠️ No media info from agent");
                            None
                        }
                        Err(e) => {
                            error!("❌ Failed to get agent media info: {}", e);
                            None
                        }
                    };
                    
                    info!("📞 B2BUA: Accepting customer call {} with agent's SDP", session_id);
                    
                    // CRITICAL: Actually accept the customer's call with agent's SDP
                    // This sends 200 OK to the customer, completing the B2BUA flow
                    match coordinator.dialog_manager.accept_incoming_call(&session_id, agent_sdp.clone()).await {
                        Ok(_) => {
                            info!("✅ Successfully accepted customer call {} with agent's SDP", session_id);
                        }
                        Err(e) => {
                            error!("❌ Failed to accept customer call {}: {}", session_id, e);
                            // Continue anyway - try to bridge even if accept failed
                        }
                    }
                    
                    // Update the customer's media session with the agent's SDP answer
                    // This completes the B2BUA flow - customer gets media info to connect to agent
                    if let Some(ref agent_sdp_str) = agent_sdp {
                        match coordinator.media_coordinator.process_sdp_answer(&session_id, agent_sdp_str).await {
                            Ok(_) => {
                                info!("✅ Customer's media session updated with agent's SDP");
                            }
                            Err(e) => {
                                error!("❌ Failed to update customer's media session: {}", e);
                                // Continue anyway - try to bridge even if media update failed
                            }
                        }
                    } else {
                        warn!("⚠️ No agent SDP available to update customer's media session");
                    }
                    
                    // Now bridge the two sessions for media flow
                    info!("🌉 Bridging customer {} with agent {} (session {:?})", 
                          session_id, agent_id, agent_session_id);
                    
                    let bridge_start = std::time::Instant::now();
                    
                    match coordinator.bridge_sessions(&session_id, &agent_session_id).await {
                        Ok(bridge_id) => {
                            let bridge_time = bridge_start.elapsed().as_millis();
                            info!("✅ Successfully bridged customer {} with agent {} (bridge: {}) in {}ms", 
                                  session_id, agent_id, bridge_id, bridge_time);
                            
                            // Update call info
                            if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                                call_info.agent_id = Some(agent_id.clone());
                                call_info.bridge_id = Some(bridge_id);
                                call_info.status = CallStatus::Bridged;
                                call_info.answered_at = Some(chrono::Utc::now());
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
                Err(e) => {
                    error!("❌ Agent {} failed to answer: {}", agent_id, e);
                    
                    // Hang up the attempted agent call
                    if let Err(term_err) = coordinator.terminate_session(&agent_session_id).await {
                        warn!("Failed to terminate unanswered agent call: {}", term_err);
                    }
                    
                    // Return agent to available pool
                    updated_agent.status = AgentStatus::Available;
                    updated_agent.current_calls = updated_agent.current_calls.saturating_sub(1);
                    self.available_agents.insert(agent_id.clone(), updated_agent);
                    
                    // Update call info to mark as queued again
                    if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                        call_info.status = CallStatus::Queued;
                        call_info.agent_id = None;
                        
                        // Re-queue the customer call with higher priority
                        if let Some(queue_id) = &call_info.queue_id {
                            let mut queue_manager = self.queue_manager.write().await;
                            let mut requeued_call = QueuedCall {
                                session_id: session_id.clone(),
                                caller_id: call_info.caller_id.clone(),
                                priority: call_info.priority.saturating_sub(5), // Higher priority
                                queued_at: call_info.queued_at.unwrap_or_else(chrono::Utc::now),
                                estimated_wait_time: None,
                                retry_count: 0,  // Reset retry count when re-queuing from failed agent assignment
                            };
                            
                            if let Err(queue_err) = queue_manager.enqueue_call(queue_id, requeued_call) {
                                error!("Failed to re-queue call {}: {}", session_id, queue_err);
                                // Last resort: terminate the customer call
                                let _ = coordinator.terminate_session(&session_id).await;
                            } else {
                                info!("📞 Re-queued call {} with higher priority", session_id);
                            }
                        }
                    }
                    
                    return Err(CallCenterError::orchestration(&format!("Agent failed to answer: {}", e)));
                }
            }
        }
        
        Ok(())
    }
    
    /// Update call state when call is established
    pub(super) async fn update_call_established(&self, session_id: SessionId) {
        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
            call_info.status = CallStatus::Bridged;
            call_info.answered_at = Some(chrono::Utc::now());
            info!("📞 Call {} marked as established/bridged", session_id);
        }
    }
    
    /// Handle call termination cleanup with agent status management
    pub(super) async fn handle_call_termination(&self, session_id: SessionId) -> CallCenterResult<()> {
        info!("🛑 Handling call termination: {}", session_id);
        
        // Check if this is part of a B2BUA dialog and terminate the related leg
        if let Some((_, related_dialog_id)) = self.dialog_mappings.remove(&session_id.0) {
            info!("📞 Terminating related dialog {} for B2BUA call", related_dialog_id);
            
            // Terminate the related dialog
            if let Some(coordinator) = &self.session_coordinator {
                let related_session_id = SessionId(related_dialog_id.clone());
                if let Err(e) = coordinator.terminate_session(&related_session_id).await {
                    warn!("Failed to terminate related dialog {}: {}", related_dialog_id, e);
                }
            }
            
            // Also remove the reverse mapping
            self.dialog_mappings.remove(&related_dialog_id);
        }
        
        // Get call info and clean up
        let call_info = self.active_calls.remove(&session_id).map(|(_, v)| v);
        
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
                    match engine.assign_specific_agent_to_call(session_id.clone(), agent_id_clone).await {
                        Ok(()) => {
                            info!("✅ Successfully assigned queued call {} to agent", session_id);
                            // On success, the call is no longer in queue or being assigned
                            let mut queue_manager = engine.queue_manager.write().await;
                            queue_manager.mark_as_not_assigned(&session_id);
                        }
                        Err(e) => {
                            error!("Failed to assign queued call {} to agent: {}", session_id, e);
                            
                            // Mark as no longer being assigned before re-queuing
                            let mut queue_manager = engine.queue_manager.write().await;
                            queue_manager.mark_as_not_assigned(&session_id);
                            
                            // Check if the call is still active before re-queuing
                            let call_still_active = engine.active_calls.contains_key(&session_id);
                            if !call_still_active {
                                warn!("Call {} is no longer active, not re-queuing", session_id);
                                return;
                            }
                            
                            // Re-queue the call with higher priority
                            let mut requeued_call = queued_call;
                            requeued_call.priority = requeued_call.priority.saturating_sub(5); // Increase priority
                            requeued_call.retry_count = requeued_call.retry_count.saturating_add(1);
                            
                            // Check retry limit (max 3 attempts)
                            if requeued_call.retry_count >= 3 {
                                error!("⚠️ Call {} exceeded maximum retry attempts, terminating", session_id);
                                // Remove from active calls
                                engine.active_calls.remove(&session_id);
                                
                                // Terminate the customer call
                                if let Some(coordinator) = engine.session_coordinator.as_ref() {
                                    let _ = coordinator.terminate_session(&session_id).await;
                                }
                                return;
                            }
                            
                            // Apply exponential backoff based on retry count
                            let backoff_ms = 500u64 * (2u64.pow(requeued_call.retry_count as u32 - 1));
                            info!("⏳ Waiting {}ms before re-queuing call {} (retry #{})", 
                                  backoff_ms, session_id, requeued_call.retry_count);
                            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                            
                            if let Err(e) = queue_manager.enqueue_call(queue_id, requeued_call) {
                                error!("Failed to re-queue call {}: {}", session_id, e);
                                
                                // Last resort: terminate the call if we can't re-queue
                                if let Some(coordinator) = engine.session_coordinator.as_ref() {
                                    let _ = coordinator.terminate_session(&session_id).await;
                                }
                            } else {
                                info!("📞 Re-queued call {} to {} with higher priority", session_id, queue_id);
                                
                                // Update call status back to queued
                                if let Some(mut call_info) = engine.active_calls.get_mut(&session_id) {
                                    call_info.status = CallStatus::Queued;
                                    call_info.queue_id = Some(queue_id.to_string());
                                }
                            }
                        }
                    }
                });
                
                break; // Only assign one call at a time
            }
        }
    }
} 