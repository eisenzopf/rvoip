//! Agent management functionality for the call center
//!
//! This module handles agent registration, status updates, and monitoring.

use std::sync::Arc;
use tracing::{info, error};
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
            &self.config.general.registrar_uri(),  // To: local registrar
            None  // No SDP for registration
        )
        .await
        .map_err(|e| CallCenterError::orchestration(&format!("Failed to create agent session: {}", e)))?;
        
        let session_id = session.id;
        let agent_id = AgentId(agent.id.clone());
        
        // Register agent in database
        if let Some(db_manager) = &self.db_manager {
            // Extract username from SIP URI
            let username = agent.sip_uri
                .strip_prefix("sip:")
                .and_then(|s| s.split('@').next())
                .unwrap_or(&agent.id)
                .to_string();
            
            db_manager.upsert_agent(&agent_id.0, &username, Some(&agent.sip_uri)).await
                .map_err(|e| CallCenterError::database(&format!("Failed to register agent in database: {}", e)))?;
            
            info!("✅ Agent {} registered in database", agent_id);
        }
        
        info!("✅ Agent {} registered with session-core (session: {}, max calls: {})", 
              agent.id, session_id, agent.max_concurrent_calls);
        Ok(session_id)
    }
    
    /// Update agent status (Available, Busy, Away, etc.)
    pub async fn update_agent_status(&self, agent_id: &AgentId, new_status: AgentStatus) -> CallCenterResult<()> {
        info!("🔄 Updating agent {} status to {:?}", agent_id, new_status);
        
        // Get current agent info from database
        let old_status = if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&agent_id.0).await {
                Ok(Some(db_agent)) => {
                    // Update status in database
                    db_manager.update_agent_status(&agent_id.0, new_status.clone()).await
                        .map_err(|e| CallCenterError::database(&format!("Failed to update agent status: {}", e)))?;
                    
                    // Return old status for logging
                    match db_agent.status {
                        crate::database::DbAgentStatus::Available => AgentStatus::Available,
                        crate::database::DbAgentStatus::Busy => AgentStatus::Busy(vec![]),
                        crate::database::DbAgentStatus::PostCallWrapUp => AgentStatus::PostCallWrapUp,
                        _ => AgentStatus::Offline,
                    }
                }
                Ok(None) => {
                    return Err(CallCenterError::not_found(format!("Agent not found: {}", agent_id)));
                }
                Err(e) => {
                    return Err(CallCenterError::database(&format!("Failed to get agent: {}", e)));
                }
            }
        } else {
            return Err(CallCenterError::database("Database not configured"));
        };
        
        info!("✅ Agent {} status updated from {:?} to {:?}", agent_id, old_status, new_status);
        
        // If agent became available, check for queued calls
        if matches!(new_status, AgentStatus::Available) {
            let agent_id_clone = agent_id.clone();
            let engine = Arc::new(self.clone());
            tokio::spawn(async move {
                engine.try_assign_queued_calls_to_agent(agent_id_clone).await;
            });
        }
        
        Ok(())
    }
    
    /// Get detailed agent information
    pub async fn get_agent_info(&self, agent_id: &AgentId) -> Option<AgentInfo> {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&agent_id.0).await {
                Ok(Some(db_agent)) => {
                    let contact_uri = self.config.general.agent_sip_uri(&db_agent.username);
                    Some(AgentInfo::from_db_agent(&db_agent, contact_uri, &self.config.general))
                }
                Ok(None) => None,
                Err(e) => {
                    error!("Failed to get agent {} from database: {}", agent_id, e);
                    None
                }
            }
        } else {
            None
        }
    }
    
    /// List all agents with their current status
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.list_agents().await {
                Ok(db_agents) => {
                    db_agents.into_iter()
                        .map(|db_agent| {
                            let contact_uri = self.config.general.agent_sip_uri(&db_agent.username);
                            AgentInfo::from_db_agent(&db_agent, contact_uri, &self.config.general)
                        })
                        .collect()
                }
                Err(e) => {
                    error!("Failed to list agents from database: {}", e);
                    vec![]
                }
            }
        } else {
            vec![]
        }
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