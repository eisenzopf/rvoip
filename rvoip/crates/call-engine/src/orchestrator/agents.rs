//! Agent management functionality for the call center
//!
//! This module handles agent registration, status updates, and monitoring.

use std::sync::Arc;
use tracing::info;
use rvoip_session_core::{SessionId, SessionControl};

use crate::agent::{Agent, AgentId, AgentStatus};
use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::queue::QueueStats;
use super::core::CallCenterEngine;
use super::types::AgentInfo;

impl CallCenterEngine {
    /// Register an agent with skills and performance tracking
    pub async fn register_agent(&self, agent: &Agent) -> CallCenterResult<SessionId> {
        info!("👤 Registering agent {} with session-core: {} (skills: {:?})", 
              agent.id, agent.sip_uri, agent.skills);
        
        // Use SessionControl trait to create outgoing call for agent registration
        let session = SessionControl::create_outgoing_call(
            self.session_manager(),
            &agent.sip_uri,  // From: agent's SIP URI
            "sip:registrar@callcenter.local",  // To: local registrar
            None  // No SDP for registration
        )
        .await
        .map_err(|e| CallCenterError::orchestration(&format!("Failed to create agent session: {}", e)))?;
        
        let session_id = session.id;
        
        // Add agent to available pool with enhanced information
        self.available_agents.insert(agent.id.clone(), AgentInfo {
            agent_id: agent.id.clone(),
            session_id: session_id.clone(),
            status: AgentStatus::Available,
            sip_uri: agent.sip_uri.clone(),          // Store the agent's SIP URI
            contact_uri: agent.sip_uri.clone(),      // For manual registration, use sip_uri as contact
            skills: agent.skills.clone(),
            current_calls: 0,
            max_calls: agent.max_concurrent_calls as usize,
            last_call_end: None,
            performance_score: 0.5, // Start with neutral performance
        });
        
        info!("✅ Agent {} registered with session-core (session: {}, max calls: {})", 
              agent.id, session_id, agent.max_concurrent_calls);
        Ok(session_id)
    }
    
    /// Update agent status (Available, Busy, Away, etc.)
    pub async fn update_agent_status(&self, agent_id: &AgentId, new_status: AgentStatus) -> CallCenterResult<()> {
        info!("🔄 Updating agent {} status to {:?}", agent_id, new_status);
        
        if let Some(mut agent_info) = self.available_agents.get_mut(agent_id) {
            let old_status = agent_info.status.clone();
            agent_info.status = new_status.clone();
            
            info!("✅ Agent {} status updated from {:?} to {:?}", agent_id, old_status, new_status);
            
            // If agent became available, check for queued calls
            let should_check_queue = matches!(new_status, AgentStatus::Available) && agent_info.current_calls == 0;
            drop(agent_info); // Release the mutable reference
            
            if should_check_queue {
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
    
    /// Get detailed agent information
    pub async fn get_agent_info(&self, agent_id: &AgentId) -> Option<AgentInfo> {
        self.available_agents.get(agent_id).map(|entry| entry.clone())
    }
    
    /// List all agents with their current status
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        self.available_agents.iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get queue statistics for monitoring
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
} 