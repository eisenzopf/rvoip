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
use super::types::{CallInfo, CallStatus, CustomerType, RoutingDecision, PendingAssignment};

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
            related_session_id: None, // Will be set when agent is assigned
        };
        
        // Store call info
        self.active_calls.insert(session_id.clone(), call_info);
        
        // B2BUA: Accept customer call immediately with our SDP answer
        // Customer will wait (with hold music) until an agent is available
        info!("📞 B2BUA: Accepting customer call {} immediately", session_id);
        
        // Generate B2BUA's SDP answer for the customer
        let sdp_answer = if let Some(ref customer_sdp) = call.sdp {
            // Generate our SDP answer based on customer's offer
            match self.session_coordinator.as_ref().unwrap()
                .generate_sdp_answer(&session_id, customer_sdp).await {
                Ok(answer) => {
                    info!("✅ Generated SDP answer for customer ({} bytes)", answer.len());
                    Some(answer)
                }
                Err(e) => {
                    error!("Failed to generate SDP answer: {}", e);
                    None
                }
            }
        } else {
            warn!("⚠️ No SDP from customer, accepting without SDP");
            None
        };
        
        // Update call status to Connecting since we're accepting
        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
            call_info.status = CallStatus::Connecting;
            call_info.answered_at = Some(chrono::Utc::now());
        }
        
        // Spawn the routing task to find an agent
        let engine = Arc::new(self.clone());
        let session_id_clone = session_id.clone();
        tokio::spawn(async move {
            // Route immediately - call is already accepted
            engine.route_call_to_agent(session_id_clone).await;
        });
        
        // Return Accept with SDP to immediately answer the customer
        Ok(CallDecision::Accept(sdp_answer))
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
                        info!("📥 Queueing call {} to {} (reason: {})", session_id, queue_id, reason);
                        
                        // Ensure the queue exists before trying to enqueue
                        if let Err(e) = self.ensure_queue_exists(&queue_id).await {
                            error!("Failed to ensure queue {} exists: {}", queue_id, e);
                            // Terminate the call if we can't create the queue
                            if let Some(coordinator) = &self.session_coordinator {
                                let _ = coordinator.terminate_session(&session_id).await;
                            }
                            return;
                        }
                        
                        // Create queued call entry
                        let call_id = uuid::Uuid::new_v4().to_string();
                        let customer_info = self.active_calls.get(&session_id)
                            .map(|c| serde_json::json!({
                                "caller_id": c.caller_id.clone(),
                                "customer_type": format!("{:?}", c.customer_type),
                            }));
                        
                        // Create the QueuedCall struct
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
                        
                        // Add to database queue
                        let mut enqueue_success = false;
                        if let Some(db_manager) = &self.db_manager {
                            match db_manager.enqueue_call(&queue_id, &queued_call).await {
                                Ok(_) => {
                                    info!("✅ Call {} enqueued to database queue '{}'", session_id, queue_id);
                                    
                                    // Log queue depth after enqueue
                                    if let Ok(depth) = db_manager.get_queue_depth(&queue_id).await {
                                        info!("📊 Queue '{}' status after enqueue: {} calls waiting", 
                                              queue_id, depth);
                                    }
                                    enqueue_success = true;
                                }
                                Err(e) => {
                                    error!("Failed to enqueue call {} to database: {}", session_id, e);
                                }
                            }
                        }
                        
                        // Always add to in-memory queue as well (database is just for persistence)
                        if !enqueue_success {
                            // Only use in-memory queue if database enqueue failed
                            let mut queue_manager = self.queue_manager.write().await;
                            match queue_manager.enqueue_call(&queue_id, queued_call) {
                                Ok(_) => {
                                    // Log queue depth after enqueue
                                    if let Ok(stats) = queue_manager.get_queue_stats(&queue_id) {
                                        info!("📊 Queue '{}' status after enqueue: {} calls waiting", 
                                              queue_id, stats.total_calls);
                                    }
                                    enqueue_success = true;
                                }
                                Err(e) => {
                                    error!("Failed to enqueue call {}: {}", session_id, e);
                                }
                            }
                        } else {
                            // Also add to in-memory queue
                            let mut queue_manager = self.queue_manager.write().await;
                            let _ = queue_manager.enqueue_call(&queue_id, queued_call);
                        }
                        
                        if !enqueue_success {
                            // Terminate the call if we can't queue it
                            if let Some(coordinator) = &self.session_coordinator {
                                let _ = coordinator.terminate_session(&session_id).await;
                            }
                            return;
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
                        
                        // PHASE 0.10: Start monitoring for agent availability immediately
                        info!("🔄 Starting queue monitor for '{}' immediately", queue_id);
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
        
        // Get agent information - agent should already be marked as busy by the atomic assignment
        let mut agent_info = match self.available_agents.get(&agent_id) {
            Some(entry) => entry.value().clone(),
            None => {
                error!("Agent {} not found in available agents", agent_id);
                return Err(CallCenterError::orchestration(&format!("Agent {} not found", agent_id)));
            }
        };
        
        // Verify agent is marked as busy (should have been done by try_assign_to_specific_agent)
        if !matches!(agent_info.status, AgentStatus::Busy(_)) {
            warn!("Agent {} is not marked as busy - possible race condition", agent_id);
            // Still proceed but log the warning
        }
            // Update database with agent's new BUSY status and incremented call count
            if let Some(db_manager) = &self.db_manager {
                // Update call count in database
                if let Err(e) = db_manager.update_agent_call_count(&agent_id.0, 1).await {
                    error!("Failed to update agent call count in database: {}", e);
                }
                
                // Update agent status to BUSY in database
                if let Err(e) = db_manager.update_agent_status(&agent_id.0, AgentStatus::Busy(vec![])).await {
                    error!("Failed to update agent status to BUSY in database: {}", e);
                } else {
                    info!("✅ Updated agent {} status to BUSY in database", agent_id);
                }
            }
            
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
                    
                    // Restore agent to available state
                    if let Some(mut entry) = self.available_agents.get_mut(&agent_id) {
                        let agent_info = entry.value_mut();
                        agent_info.status = AgentStatus::Available;
                        agent_info.current_calls = agent_info.current_calls.saturating_sub(1);
                        info!("🔓 Restored agent {} to available after prepare failure", agent_id);
                    }
                    
                    // Update database to reflect agent is available again
                    if let Some(db_manager) = &self.db_manager {
                        if let Err(e) = db_manager.update_agent_call_count(&agent_id.0, -1).await {
                            error!("Failed to decrement agent call count in database: {}", e);
                        }
                        if let Err(e) = db_manager.update_agent_status(&agent_id.0, AgentStatus::Available).await {
                            error!("Failed to restore agent status to Available in database: {}", e);
                        }
                    }
                    
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
                    
                    // Create CallInfo for the agent's session with proper tracking
                    let agent_call_info = CallInfo {
                        session_id: call_session.id.clone(),
                        caller_id: "Call Center".to_string(),
                        from: "sip:call-center@127.0.0.1".to_string(),
                        to: agent_info.sip_uri.clone(),
                        agent_id: Some(agent_id.clone()), // Important: Set agent_id for agent's session
                        queue_id: None,
                        bridge_id: None,
                        status: CallStatus::Connecting,
                        priority: 0, // Highest priority for agent calls
                        customer_type: CustomerType::Standard,
                        required_skills: vec![],
                        created_at: chrono::Utc::now(),
                        queued_at: None,
                        answered_at: None,
                        ended_at: None,
                        customer_sdp: None,
                        duration_seconds: 0,
                        wait_time_seconds: 0,
                        talk_time_seconds: 0,
                        hold_time_seconds: 0,
                        queue_time_seconds: 0,
                        transfer_count: 0,
                        hold_count: 0,
                        customer_dialog_id: None,
                        agent_dialog_id: None,
                        related_session_id: Some(session_id.clone()), // Link to customer session
                    };
                    
                    // Store the agent's call info
                    self.active_calls.insert(call_session.id.clone(), agent_call_info);
                    info!("📋 Created CallInfo for agent session {} with agent_id={}", call_session.id, agent_id);
                    
                    // Update the customer's call info with the agent session ID
                    if let Some(mut customer_call_info) = self.active_calls.get_mut(&session_id) {
                        customer_call_info.related_session_id = Some(call_session.id.clone());
                        info!("📋 Updated customer session {} with related agent session {}", session_id, call_session.id);
                    }
                    
                    call_session
                }
                Err(e) => {
                    error!("Failed to initiate call to agent {}: {}", agent_id, e);
                    
                    // Restore agent to available state
                    if let Some(mut entry) = self.available_agents.get_mut(&agent_id) {
                        let agent_info = entry.value_mut();
                        agent_info.status = AgentStatus::Available;
                        agent_info.current_calls = agent_info.current_calls.saturating_sub(1);
                        info!("🔓 Restored agent {} to available after initiate failure", agent_id);
                    }
                    
                    // Update database to reflect agent is available again
                    if let Some(db_manager) = &self.db_manager {
                        if let Err(e) = db_manager.update_agent_call_count(&agent_id.0, -1).await {
                            error!("Failed to decrement agent call count in database: {}", e);
                        }
                        if let Err(e) = db_manager.update_agent_status(&agent_id.0, AgentStatus::Available).await {
                            error!("Failed to restore agent status to Available in database: {}", e);
                        }
                    }
                    
                    return Err(CallCenterError::orchestration(&format!("Failed to call agent: {}", e)));
                }
            };
            
            // Get the session ID from the CallSession
            let agent_session_id = agent_call_session.id.clone();
            
            // Update agent info with the new session ID
            agent_info.session_id = agent_session_id.clone();
            
            // Step 3: Store pending assignment instead of waiting
            info!("📝 Storing pending assignment for agent {} to answer", agent_id);
            
            // Get customer's SDP from the call info
            let customer_sdp = self.active_calls.get(&session_id)
                .and_then(|call_info| call_info.customer_sdp.clone());
            
            let pending_assignment = PendingAssignment {
                customer_session_id: session_id.clone(),
                agent_session_id: agent_session_id.clone(),
                agent_id: agent_id.clone(),
                timestamp: chrono::Utc::now(),
                customer_sdp: customer_sdp,
            };
            
            // Store in pending assignments collection using agent session ID as key
            self.pending_assignments.insert(agent_session_id.clone(), pending_assignment);
            
            // Update agent info with the new session ID
            if let Some(mut entry) = self.available_agents.get_mut(&agent_id) {
                entry.session_id = agent_session_id.clone();
            }
            
            info!("✅ Agent {} call initiated - waiting for answer event", agent_id);
            
            // Start timeout task for agent answer (30 seconds)
            let engine = Arc::new(self.clone());
            let timeout_agent_id = agent_id.clone();
            let timeout_agent_session_id = agent_session_id.clone();
            let timeout_customer_session_id = session_id.clone();
            
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                
                // Check if assignment is still pending
                if engine.pending_assignments.contains_key(&timeout_agent_session_id) {
                    warn!("⏰ Agent {} failed to answer within 30 seconds", timeout_agent_id);
                    
                    // Remove from pending
                    engine.pending_assignments.remove(&timeout_agent_session_id);
                    
                    // Terminate the agent call
                    if let Some(coordinator) = &engine.session_coordinator {
                        let _ = coordinator.terminate_session(&timeout_agent_session_id).await;
                    }
                    
                    // Return agent to available pool
                    if let Some(mut agent_info) = engine.available_agents.get_mut(&timeout_agent_id) {
                        agent_info.status = AgentStatus::Available;
                        agent_info.current_calls = agent_info.current_calls.saturating_sub(1);
                    }
                    
                    // Update database
                    if let Some(db_manager) = &engine.db_manager {
                        let _ = db_manager.update_agent_call_count(&timeout_agent_id.0, -1).await;
                        let _ = db_manager.update_agent_status(&timeout_agent_id.0, AgentStatus::Available).await;
                    }
                    
                    // Re-queue the customer call
                    if let Some(mut call_info) = engine.active_calls.get_mut(&timeout_customer_session_id) {
                        call_info.status = CallStatus::Queued;
                        call_info.agent_id = None;
                        
                        if let Some(queue_id) = &call_info.queue_id {
                            let mut queue_manager = engine.queue_manager.write().await;
                            let requeued_call = QueuedCall {
                                session_id: timeout_customer_session_id.clone(),
                                caller_id: call_info.caller_id.clone(),
                                priority: call_info.priority.saturating_sub(5), // Higher priority
                                queued_at: call_info.queued_at.unwrap_or_else(chrono::Utc::now),
                                estimated_wait_time: None,
                                retry_count: 0,
                            };
                            
                            if let Err(e) = queue_manager.enqueue_call(queue_id, requeued_call) {
                                error!("Failed to re-queue call after timeout: {}", e);
                            } else {
                                info!("📞 Re-queued call {} after agent timeout", timeout_customer_session_id);
                            }
                        }
                    }
                }
            });
            
            // Return immediately - the event handler will complete the bridge when agent answers
        
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
        
        // Get call info and clean up
        let call_info = self.active_calls.remove(&session_id).map(|(_, v)| v);
        
        // Remove from database queue if the call was queued
        if let Some(db_manager) = &self.db_manager {
            // Create a cleanup method that removes the call from both tables
            if let Err(e) = db_manager.remove_call_from_queue(&session_id.0).await {
                debug!("Failed to remove call {} from queue: {}", session_id, e);
            } else {
                debug!("🧹 Cleaned up call {} from database", session_id);
            }
        }
        
        // Update agent status if this call had an agent assigned
        if let Some(call_info) = &call_info {
            if let Some(agent_id) = &call_info.agent_id {
                info!("🔄 Updating agent {} status after call termination", agent_id);
                
                // Update agent status
                if let Some(mut agent_info) = self.available_agents.get_mut(agent_id) {
                    agent_info.current_calls = agent_info.current_calls.saturating_sub(1);
                    agent_info.last_call_end = Some(chrono::Utc::now());
                    
                    // If agent has no active calls, mark as post-call wrap-up
                    if agent_info.current_calls == 0 {
                        // PHASE 0.10: Log agent status transition
                        info!("🔄 Agent {} status change: Busy → PostCallWrapUp (entering wrap-up time)", agent_id);
                        agent_info.status = AgentStatus::PostCallWrapUp;
                        info!("⏰ Agent {} entering 10-second post-call wrap-up", agent_id);
                        
                        // Schedule transition to Available after 10 seconds
                        let engine = Arc::new(self.clone());
                        let wrap_up_agent_id = agent_id.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                            
                            // Transition from PostCallWrapUp to Available
                            if let Some(mut agent_info) = engine.available_agents.get_mut(&wrap_up_agent_id) {
                                if matches!(agent_info.status, AgentStatus::PostCallWrapUp) {
                                    info!("🔄 Agent {} status change: PostCallWrapUp → Available (wrap-up complete)", wrap_up_agent_id);
                                    agent_info.status = AgentStatus::Available;
                                    info!("✅ Agent {} is now available for new calls", wrap_up_agent_id);
                                    
                                    // Update database
                                    if let Some(db_manager) = &engine.db_manager {
                                        if let Err(e) = db_manager.update_agent_status(&wrap_up_agent_id.0, AgentStatus::Available).await {
                                            error!("Failed to update agent status to Available in database: {}", e);
                                        } else {
                                            info!("✅ Updated agent {} status to Available in database", wrap_up_agent_id);
                                        }
                                    }
                                    
                                    // Check for stuck assignments and queued calls
                                    engine.check_stuck_assignments().await;
                                    
                                    // Check if there are queued calls that can be assigned to this agent
                                    engine.try_assign_queued_calls_to_agent(wrap_up_agent_id.clone()).await;
                                }
                            }
                        });
                    } else {
                        info!("📞 Agent {} still busy with {} active calls", agent_id, agent_info.current_calls);
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
                
                // Update database with new agent status
                if let Some(db_manager) = &self.db_manager {
                    // Update call count in database
                    if let Err(e) = db_manager.update_agent_call_count(&agent_id.0, -1).await {
                        error!("Failed to update agent call count in database: {}", e);
                    }
                    
                    // Update agent status in database based on current status
                    if let Some(agent_info) = self.available_agents.get(agent_id) {
                        // Use the actual status (which might be PostCallWrapUp)
                        if let Err(e) = db_manager.update_agent_status(&agent_id.0, agent_info.status.clone()).await {
                            error!("Failed to update agent status in database: {}", e);
                        } else {
                            match &agent_info.status {
                                AgentStatus::PostCallWrapUp => {
                                    info!("✅ Updated agent {} status to PostCallWrapUp in database", agent_id);
                                }
                                AgentStatus::Available => {
                                    info!("✅ Updated agent {} status to Available in database", agent_id);
                                }
                                status => {
                                    info!("✅ Updated agent {} status to {:?} in database", agent_id, status);
                                }
                            }
                        }
                    }
                }
                
                // Check if there are queued calls that can be assigned to this agent
                self.try_assign_queued_calls_to_agent(agent_id.clone()).await;
            }
        }
        
        // Clean up bridge if call had one
        if let Some(call_info) = &call_info {
            if let Some(bridge_id) = &call_info.bridge_id {
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
    
    /// Check for calls stuck in "being assigned" state and re-queue them
    pub(super) async fn check_stuck_assignments(&self) {
        debug!("🔍 Checking for stuck assignments");
        
        // Get list of calls that might be stuck
        let mut stuck_calls = Vec::new();
        
        // Check active calls for those in Connecting state without an agent actively handling them
        for entry in self.active_calls.iter() {
            let (session_id, call_info) = (entry.key(), entry.value());
            
            // A call is stuck if:
            // 1. It's in Connecting state (being assigned)
            // 2. It has a queue_id (was queued)
            // 3. It's not in pending_assignments (no agent is handling it)
            if matches!(call_info.status, super::types::CallStatus::Connecting) &&
               call_info.queue_id.is_some() &&
               !self.pending_assignments.contains_key(session_id) {
                
                // Check how long it's been in this state
                let duration = chrono::Utc::now().signed_duration_since(call_info.created_at);
                if duration.num_seconds() > 5 {  // Stuck for more than 5 seconds
                    warn!("⚠️ Found stuck call {} in Connecting state for {}s", 
                          session_id, duration.num_seconds());
                    stuck_calls.push((session_id.clone(), call_info.queue_id.clone().unwrap()));
                }
            }
        }
        
        // Re-queue stuck calls
        let stuck_count = stuck_calls.len();
        for (session_id, queue_id) in stuck_calls {
            info!("🔄 Re-queuing stuck call {} to queue {}", session_id, queue_id);
            
            // Update call status back to Queued
            if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                call_info.status = super::types::CallStatus::Queued;
                
                // Create a QueuedCall to re-enqueue
                let queued_call = QueuedCall {
                    session_id: session_id.clone(),
                    caller_id: call_info.caller_id.clone(),
                    priority: call_info.priority.saturating_sub(10), // Higher priority for stuck calls
                    queued_at: call_info.queued_at.unwrap_or_else(chrono::Utc::now),
                    estimated_wait_time: None,
                    retry_count: 1,  // Mark as retry
                };
                
                // Re-enqueue the call
                let mut queue_manager = self.queue_manager.write().await;
                
                // First, clear any "being assigned" flag
                queue_manager.mark_as_not_assigned(&session_id);
                
                // Then re-queue
                match queue_manager.enqueue_call(&queue_id, queued_call) {
                    Ok(position) => {
                        info!("✅ Successfully re-queued stuck call {} with higher priority at position {}", session_id, position);
                        
                        // Start queue monitor if needed
                        self.monitor_queue_for_agents(queue_id).await;
                    }
                    Err(e) => {
                        error!("Failed to re-queue stuck call {}: {}", session_id, e);
                        // As a last resort, terminate the call
                        if let Some(coordinator) = &self.session_coordinator {
                            let _ = coordinator.terminate_session(&session_id).await;
                        }
                    }
                }
            }
        }
        
        if stuck_count > 0 {
            info!("🔄 Re-queued {} stuck calls", stuck_count);
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