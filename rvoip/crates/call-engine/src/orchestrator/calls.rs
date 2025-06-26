//! Call handling logic for the call center
//!
//! This module implements the core call processing functionality including
//! incoming call handling, agent assignment, and call lifecycle management.

use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, debug, warn, error};
use tokio::time::{timeout, Duration};

use rvoip_session_core::api::{
    types::{IncomingCall, CallDecision, SessionId, CallState},
    control::SessionControl,
    media::MediaControl,
};

use crate::agent::{AgentId, AgentStatus};
use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::queue::{QueuedCall, QueueStats};
use super::core::CallCenterEngine;
use super::types::{CallInfo, CallStatus, CustomerType, RoutingDecision};

impl CallCenterEngine {
    /// Process incoming call with sophisticated routing
    pub(super) async fn process_incoming_call(&self, call: IncomingCall) -> CallCenterResult<CallDecision> {
        let session_id = call.id.clone();
        let routing_start = std::time::Instant::now();
        
        info!("📞 Processing incoming call: {} from {} to {}", 
              call.id, call.from, call.to);
        
        // PHASE 17.3: IncomingCall doesn't have dialog_id, we'll get it from events
        // For now, just track that we don't have the dialog ID yet
        let incoming_dialog_id = None; // Will be populated when we get dialog events
        info!("🔍 Incoming call - dialog ID will be set from events");
        
        // Create a session ID for internal tracking - IncomingCall.id is already a SessionId
        let session_id = call.id.clone();
        
        // Extract and log the SDP from the incoming call
        if let Some(ref sdp) = call.sdp {
            info!("📄 Incoming call has SDP offer ({} bytes)", sdp.len());
            debug!("📄 Customer SDP content:\n{}", sdp);
        } else {
            warn!("⚠️ Incoming call has no SDP offer");
        }
        
        // Analyze customer information and determine routing requirements
        let (customer_type, priority, required_skills) = self.analyze_customer_info(&call).await;
        
        // Store the call information
        let mut call_info = CallInfo {
            session_id: session_id.clone(),
            caller_id: call.from.clone(),
            from: call.from.clone(),
            to: call.to.clone(),
            agent_id: None,
            queue_id: None,
            status: CallStatus::Ringing,
            priority,
            customer_type,
            required_skills,
            created_at: chrono::Utc::now(),
            answered_at: None,
            ended_at: None,
            bridge_id: None,
            duration_seconds: 0,
            wait_time_seconds: 0,
            talk_time_seconds: 0,
            queue_time_seconds: 0,
            hold_time_seconds: 0,
            queued_at: None,
            transfer_count: 0,
            hold_count: 0,
            customer_sdp: call.sdp.clone(), // Store the customer's SDP for later use
            customer_dialog_id: incoming_dialog_id, // PHASE 17.3: Store the dialog ID
            agent_dialog_id: None,
        };
        
        // Store call info
        self.active_calls.insert(session_id.clone(), call_info);
        
        // B2BUA: Initially defer the call to send 180 Ringing
        // We'll accept with 200 OK only after agent answers
        info!("📞 B2BUA: Deferring customer call {} to send 180 Ringing", session_id);
        
        // Spawn the routing task
        let engine = Arc::new(self.clone());
        let session_id_clone = session_id.clone();
        tokio::spawn(async move {
            engine.route_call_to_agent(session_id_clone).await;
        });
        
        // Return Defer to send 180 Ringing to customer
        Ok(CallDecision::Defer)
    }
    
    /// Route an already-accepted call to an agent
    async fn route_call_to_agent(&self, session_id: SessionId) {
        info!("🚦 Routing call {} to find available agent", session_id);
        
        let routing_start = std::time::Instant::now();
        
        // Make intelligent routing decision based on multiple factors
        let routing_decision = self.make_routing_decision(&session_id, &CustomerType::Standard, 50, &[]).await;
        
        match routing_decision {
            Ok(decision) => {
                info!("🎯 Routing decision for call {}: {:?}", session_id, decision);
                
                // Handle the routing decision
                match decision {
                    RoutingDecision::DirectToAgent { agent_id, reason } => {
                        info!("📞 Direct routing to agent {} for call {}: {}", agent_id, session_id, reason);
                        
                        // Update call status
                        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                            call_info.status = CallStatus::Connecting;
                            call_info.agent_id = Some(agent_id.clone());
                        }
                        
                        // Assign to specific agent
                        if let Err(e) = self.assign_specific_agent_to_call(session_id.clone(), agent_id.clone()).await {
                            error!("Failed to assign call {} to agent {}: {}", session_id, agent_id, e);
                            
                            // TODO: Re-queue or find another agent
                            if let Some(coordinator) = &self.session_coordinator {
                                let _ = coordinator.terminate_session(&session_id).await;
                            }
                        }
                    },
                    
                    RoutingDecision::Queue { queue_id, priority, reason } => {
                        info!("📥 Queueing call {} to {} (priority: {}, reason: {})", 
                              session_id, queue_id, priority, reason);
                        
                        // Create queued call entry
                        let queued_call = QueuedCall {
                            session_id: session_id.clone(),
                            caller_id: self.active_calls.get(&session_id)
                                .map(|c| c.caller_id.clone())
                                .unwrap_or_else(|| "unknown".to_string()),
                            priority,
                            queued_at: chrono::Utc::now(),
                            estimated_wait_time: None,
                            retry_count: 0,
                        };
                        
                        // Add to queue
                        {
                            let mut queue_manager = self.queue_manager.write().await;
                            if let Err(e) = queue_manager.enqueue_call(&queue_id, queued_call) {
                                error!("Failed to enqueue call {}: {}", session_id, e);
                                
                                // Terminate the call if we can't queue it
                                if let Some(coordinator) = &self.session_coordinator {
                                    let _ = coordinator.terminate_session(&session_id).await;
                                }
                                return;
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
                        self.monitor_queue_for_agents(queue_id).await;
                    },
                    
                    RoutingDecision::Reject { reason } => {
                        warn!("❌ Rejecting call {} after acceptance: {}", session_id, reason);
                        
                        // Since we already accepted, we need to terminate
                        if let Some(coordinator) = &self.session_coordinator {
                            let _ = coordinator.terminate_session(&session_id).await;
                        }
                        
                        // Update routing stats
                        {
                            let mut stats = self.routing_stats.write().await;
                            stats.calls_rejected += 1;
                        }
                    },
                    
                    _ => {
                        warn!("Unhandled routing decision for call {}: {:?}", session_id, decision);
                    }
                }
            }
            Err(e) => {
                error!("Routing decision failed for call {}: {}", session_id, e);
                
                // Terminate the call
                if let Some(coordinator) = &self.session_coordinator {
                    let _ = coordinator.terminate_session(&session_id).await;
                }
            }
        }
        
        // Update routing time metrics
        let routing_time = routing_start.elapsed().as_millis() as u64;
        {
            let mut stats = self.routing_stats.write().await;
            stats.average_routing_time_ms = (stats.average_routing_time_ms + routing_time) / 2;
        }
        
        info!("✅ Call {} routing completed in {}ms", session_id, routing_time);
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
        
        if let Some(mut agent_info) = agent_info {
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
            
            // Step 1: B2BUA prepares its own SDP offer for the agent
            // This allocates media resources and generates our SDP
            let agent_contact_uri = agent_info.contact_uri.clone(); // Use the contact URI from REGISTER
            let call_center_uri = format!("sip:call-center@{}", self.config.general.domain);
            
            info!("📞 B2BUA: Preparing outgoing call to agent {} at {}", 
                  agent_id, agent_contact_uri);
            
            // Prepare the call - this allocates media resources and generates SDP
            let prepared_call = match SessionControl::prepare_outgoing_call(
                coordinator,
                &call_center_uri,    // FROM: The call center is making the call
                &agent_contact_uri,  // TO: The agent is receiving the call
            ).await {
                Ok(prepared) => {
                    info!("✅ B2BUA: Prepared call with SDP offer ({} bytes), allocated RTP port: {}", 
                          prepared.sdp_offer.len(), prepared.local_rtp_port);
                    prepared
                }
                Err(e) => {
                    error!("Failed to prepare outgoing call to agent {}: {}", agent_id, e);
                    let mut restored_agent = agent_info;
                    restored_agent.status = AgentStatus::Available;
                    restored_agent.current_calls = restored_agent.current_calls.saturating_sub(1);
                    self.available_agents.insert(agent_id, restored_agent);
                    return Err(CallCenterError::orchestration(&format!("Failed to prepare call to agent: {}", e)));
                }
            };
            
            // Step 2: Initiate the prepared call with our SDP offer
            let agent_call_session = match SessionControl::initiate_prepared_call(
                coordinator,
                &prepared_call,
            ).await {
                Ok(call_session) => {
                    info!("✅ Created outgoing call {:?} to agent {} with SDP", call_session.id, agent_id);
                    
                    // PHASE 17.3: CallSession doesn't have dialog_id field
                    // We need to get the dialog ID from the dialog events instead
                    let agent_dialog_id: Option<String> = None; // Will be populated when we get dialog events
                    info!("🔍 Agent call created - dialog ID will be set from events");
                    
                    // Store the agent's dialog ID in the call info
                    if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                        // For now, we can't set agent_dialog_id since we don't have it yet
                        // The proper dialog tracking will happen through session-core events
                    }
                    
                    // Track session-to-session mappings for call center operations
                    self.dialog_mappings.insert(session_id.0.clone(), call_session.id.0.clone());
                    self.dialog_mappings.insert(call_session.id.0.clone(), session_id.0.clone());
                    info!("📋 Tracked SESSION mapping: {} ↔ {}", session_id, call_session.id);
                    
                    // PHASE 17.1: Add detailed dialog ID logging for debugging
                    info!("🔍 B2BUA Session Mapping Details:");
                    info!("  Customer Session ID: {}", session_id);
                    info!("  Agent Session ID: {}", call_session.id);
                    info!("  Stored mappings: {} entries total", self.dialog_mappings.len());
                    
                    call_session
                }
                Err(e) => {
                    error!("Failed to initiate call to agent {}: {}", agent_id, e);
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
            agent_info.session_id = agent_session_id.clone();
            
            // Step 3: Wait for the agent to answer the call
            info!("⏳ Waiting for agent {} to answer...", agent_id);
            
            match coordinator.wait_for_answer(&agent_session_id, std::time::Duration::from_secs(30)).await {
                Ok(_) => {
                    info!("✅ Agent {} answered the call", agent_id);
                    
                    // Get agent's SDP from their 200 OK response
                    let agent_sdp = match SessionControl::get_media_info(coordinator, &agent_session_id).await {
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
                    
                    info!("📞 B2BUA: Bridging customer {} with agent {}", session_id, agent_id);
                    
                    // CRITICAL: Accept customer's deferred call now that agent has answered
                    // Generate B2BUA's SDP answer for the customer based on their offer
                    if let Some(customer_sdp_offer) = self.active_calls.get(&session_id)
                        .and_then(|info| info.customer_sdp.clone()) {
                        
                        info!("📄 B2BUA: Generating SDP answer for customer");
                        match MediaControl::generate_sdp_answer(coordinator, &session_id, &customer_sdp_offer).await {
                            Ok(b2bua_answer) => {
                                info!("✅ B2BUA: Generated SDP answer ({} bytes)", b2bua_answer.len());
                                
                                // Accept the customer's call with our SDP answer
                                match coordinator.dialog_manager.accept_incoming_call(&session_id, Some(b2bua_answer.clone())).await {
                                    Ok(_) => {
                                        info!("✅ Successfully accepted customer call {} with B2BUA's SDP", session_id);
                                    }
                                    Err(e) => {
                                        error!("❌ Failed to accept customer call {}: {}", session_id, e);
                                        // Continue anyway - try to bridge
                                    }
                                }
                            }
                            Err(e) => {
                                error!("❌ Failed to generate SDP answer for customer: {}", e);
                                // Accept without SDP as fallback
                                if let Err(e) = coordinator.dialog_manager.accept_incoming_call(&session_id, None).await {
                                    error!("❌ Failed to accept customer call without SDP: {}", e);
                                }
                            }
                        }
                    } else {
                        warn!("⚠️ No customer SDP offer found - accepting without SDP");
                        if let Err(e) = coordinator.dialog_manager.accept_incoming_call(&session_id, None).await {
                            error!("❌ Failed to accept customer call without SDP: {}", e);
                        }
                    }
                    
                    // Update the customer's media session with the agent's SDP for media routing
                    // This is internal B2BUA media bridging configuration
                    
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
                            self.available_agents.insert(agent_id, agent_info);
                        },
                        Err(e) => {
                            error!("Failed to bridge sessions: {}", e);
                            
                            // Hang up the agent call
                            let _ = coordinator.terminate_session(&agent_session_id).await;
                            
                            // Return agent to available pool
                            agent_info.status = AgentStatus::Available;
                            agent_info.current_calls = agent_info.current_calls.saturating_sub(1);
                            self.available_agents.insert(agent_id, agent_info);
                            
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
                    agent_info.status = AgentStatus::Available;
                    agent_info.current_calls = agent_info.current_calls.saturating_sub(1);
                    self.available_agents.insert(agent_id, agent_info);
                    
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
        
        // First, update the call end time and calculate metrics
        let now = chrono::Utc::now();
        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
            call_info.ended_at = Some(now);
            
            // Calculate total duration
            call_info.duration_seconds = now.signed_duration_since(call_info.created_at).num_seconds() as u64;
            
            // Calculate wait time (time until answered or ended if never answered)
            if let Some(answered_at) = call_info.answered_at {
                call_info.wait_time_seconds = answered_at.signed_duration_since(call_info.created_at).num_seconds() as u64;
                // Calculate talk time (answered until ended)
                call_info.talk_time_seconds = now.signed_duration_since(answered_at).num_seconds() as u64;
            } else {
                // Never answered - entire duration was wait time
                call_info.wait_time_seconds = call_info.duration_seconds;
                call_info.talk_time_seconds = 0;
            }
            
            // Calculate queue time if the call was queued
            if let (Some(queued_at), Some(answered_at)) = (call_info.queued_at, call_info.answered_at) {
                call_info.queue_time_seconds = answered_at.signed_duration_since(queued_at).num_seconds() as u64;
            } else if let Some(queued_at) = call_info.queued_at {
                // Still in queue when ended
                call_info.queue_time_seconds = now.signed_duration_since(queued_at).num_seconds() as u64;
            }
            
            info!("📊 Call {} metrics - Total: {}s, Wait: {}s, Talk: {}s, Queue: {}s, Hold: {}s", 
                  session_id, 
                  call_info.duration_seconds,
                  call_info.wait_time_seconds,
                  call_info.talk_time_seconds,
                  call_info.queue_time_seconds,
                  call_info.hold_time_seconds);
        }
        
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
    
    /// Put a call on hold
    pub async fn put_call_on_hold(&self, session_id: &SessionId) -> CallCenterResult<()> {
        if let Some(mut call_info) = self.active_calls.get_mut(session_id) {
            if call_info.status == CallStatus::Bridged {
                call_info.status = CallStatus::OnHold;
                call_info.hold_count += 1;
                
                // Track hold start time (would need additional field for accurate tracking)
                info!("☎️ Call {} put on hold (count: {})", session_id, call_info.hold_count);
                
                // TODO: Actually put the call on hold via session coordinator
                if let Some(coordinator) = &self.session_coordinator {
                    coordinator.hold_session(session_id).await
                        .map_err(|e| CallCenterError::orchestration(&format!("Failed to hold call: {}", e)))?;
                }
            }
        }
        Ok(())
    }
    
    /// Resume a call from hold
    pub async fn resume_call_from_hold(&self, session_id: &SessionId) -> CallCenterResult<()> {
        if let Some(mut call_info) = self.active_calls.get_mut(session_id) {
            if call_info.status == CallStatus::OnHold {
                call_info.status = CallStatus::Bridged;
                
                // TODO: Calculate and add to total hold time
                info!("📞 Call {} resumed from hold", session_id);
                
                // TODO: Actually resume the call via session coordinator
                if let Some(coordinator) = &self.session_coordinator {
                    coordinator.resume_session(session_id).await
                        .map_err(|e| CallCenterError::orchestration(&format!("Failed to resume call: {}", e)))?;
                }
            }
        }
        Ok(())
    }
    
    /// Transfer a call to another agent or queue (simple version)
    pub async fn transfer_call_simple(&self, session_id: &SessionId, target: &str) -> CallCenterResult<()> {
        if let Some(mut call_info) = self.active_calls.get_mut(session_id) {
            call_info.transfer_count += 1;
            call_info.status = CallStatus::Transferring;
            
            info!("📞 Transferring call {} to {} (count: {})", 
                  session_id, target, call_info.transfer_count);
            
            // TODO: Implement actual transfer logic
            // This would involve:
            // 1. Finding the target agent/queue
            // 2. Creating new session to target
            // 3. Bridging or re-routing the call
        }
        Ok(())
    }
} 