//! Administrative API for call center management
//!
//! This module provides APIs for administrators to configure and manage
//! the call center system.

use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{State, Path},
    response::Json,
    http::StatusCode,
};
use std::sync::Arc;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::{
    CallCenterEngine,
    agent::{Agent, AgentStatus, AgentId},
    config::{CallCenterConfig, QueueConfig, RoutingConfig},
    error::{CallCenterError, Result as CallCenterResult},
    queue::QueueStats,
};

/// Administrative API for call center management
/// 
/// This provides system-level management capabilities:
/// - Agent management (add, remove, modify)
/// - Queue configuration
/// - Routing rules management
/// - System configuration updates
/// - Database maintenance
#[derive(Clone)]
pub struct AdminApi {
    engine: Arc<CallCenterEngine>,
}

impl AdminApi {
    /// Create a new admin API instance
    pub fn new(engine: Arc<CallCenterEngine>) -> Self {
        Self { engine }
    }
    
    /// Add a new agent
    pub async fn add_agent(&self, agent: Agent) -> Result<(), CallCenterError> {
        // Register with the registry
        let mut registry = self.engine.agent_registry.lock().await;
        registry.register_agent(agent.clone()).await?;
        
        // Also add to database if available
        if let Some(db) = self.engine.database_manager() {
            // Extract username from SIP URI (e.g., "alice" from "sip:alice@127.0.0.1")
            let username = agent.sip_uri
                .trim_start_matches("sip:")
                .split('@')
                .next()
                .unwrap_or(&agent.id);
            
            db.upsert_agent(
                &agent.id,
                username,  // Use the SIP username, not display_name
                Some(&agent.sip_uri)
            ).await.map_err(|e| CallCenterError::database(&format!("Failed to upsert agent: {}", e)))?;
            
            // Update status separately
            db.update_agent_status(&agent.id, agent.status.clone())
                .await.map_err(|e| CallCenterError::database(&format!("Failed to update status: {}", e)))?;
        }
        
        Ok(())
    }
    
    /// Update an existing agent
    pub async fn update_agent(&self, agent: Agent) -> Result<(), CallCenterError> {
        // Update in database if available
        if let Some(db) = self.engine.database_manager() {
            // Extract username from SIP URI (e.g., "alice" from "sip:alice@127.0.0.1")
            let username = agent.sip_uri
                .trim_start_matches("sip:")
                .split('@')
                .next()
                .unwrap_or(&agent.id);
                
            db.upsert_agent(
                &agent.id,
                username,  // Use the SIP username, not display_name
                Some(&agent.sip_uri)
            ).await.map_err(|e| CallCenterError::database(&format!("Failed to upsert agent: {}", e)))?;
            
            // Update status separately
            db.update_agent_status(&agent.id, agent.status.clone())
                .await.map_err(|e| CallCenterError::database(&format!("Failed to update status: {}", e)))?;
        } else {
            return Err(CallCenterError::internal("Database not configured"));
        }
        
        Ok(())
    }
    
    /// Remove an agent
    pub async fn remove_agent(&self, agent_id: &AgentId) -> Result<(), CallCenterError> {
        // Remove from registry
        let mut registry = self.engine.agent_registry.lock().await;
        registry.remove_agent_session(&agent_id.0)?;
        
        // Also mark as offline in database if available
        if let Some(db) = self.engine.database_manager() {
            db.mark_agent_offline(&agent_id.0)
                .await.map_err(|e| CallCenterError::database(&format!("Failed to mark agent offline: {}", e)))?;
        }
        
        Ok(())
    }
    
    /// List all agents
    pub async fn list_agents(&self) -> Result<Vec<Agent>, CallCenterError> {
        if let Some(db) = self.engine.database_manager() {
            // Get DB agents and convert to API agents
            let db_agents = db.list_agents()
                .await.map_err(|e| CallCenterError::database(&format!("Failed to list agents: {}", e)))?;
            
            // Convert DB agents to API agents
            let agents = db_agents.into_iter().map(|db_agent| {
                Agent {
                    id: db_agent.agent_id,
                    sip_uri: db_agent.contact_uri.unwrap_or_else(|| format!("sip:{}@localhost", db_agent.username)),
                    display_name: db_agent.username,
                    skills: vec![], // TODO: Load from database when skill table is implemented
                    max_concurrent_calls: db_agent.max_calls as u32,
                    status: match db_agent.status {
                                            crate::database::DbAgentStatus::Available => AgentStatus::Available,
                    crate::database::DbAgentStatus::Busy => AgentStatus::Busy(vec![]),
                    crate::database::DbAgentStatus::PostCallWrapUp => AgentStatus::PostCallWrapUp,
                    crate::database::DbAgentStatus::Offline => AgentStatus::Offline,
                    crate::database::DbAgentStatus::Reserved => AgentStatus::Available, // Treat as available
                    },
                    department: None,
                    extension: None,
                }
            }).collect();
            
            Ok(agents)
        } else {
            // Return empty list if no database
            Ok(Vec::new())
        }
    }
    
    /// Update agent skills
    pub async fn update_agent_skills(&self, agent_id: &AgentId, skills: Vec<String>) -> Result<(), CallCenterError> {
        if let Some(_db) = self.engine.database_manager() {
            // TODO: Implement skill storage in database
            // For now, just log the request
            tracing::info!("Updating skills for agent {}: {:?}", agent_id.0, skills);
            Ok(())
        } else {
            return Err(CallCenterError::internal("Database not configured"));
        }
    }
    
    /// Create a new queue
    pub async fn create_queue(&self, queue_id: &str) -> CallCenterResult<()> {
        self.engine.create_queue(queue_id).await
    }
    
    /// Update queue configuration
    pub async fn update_queue(&self, queue_id: &str, config: QueueConfig) -> CallCenterResult<()> {
        // In a real implementation, this would update queue settings
        tracing::info!("Updating queue {} configuration", queue_id);
        // TODO: Implement queue configuration updates
        Ok(())
    }
    
    /// Delete a queue
    /// 
    /// This will fail if the queue has active calls
    pub async fn delete_queue(&self, queue_id: &str) -> CallCenterResult<()> {
        let queue_manager = self.engine.queue_manager().read().await;
        let stats = queue_manager.get_queue_stats(queue_id)?;
        
        if stats.total_calls > 0 {
            return Err(CallCenterError::validation(
                "Cannot delete queue with active calls"
            ));
        }
        
        drop(queue_manager);
        // TODO: Add proper queue removal method to QueueManager
        Ok(())
    }
    
    /// Get current system configuration
    pub fn get_config(&self) -> &CallCenterConfig {
        self.engine.config()
    }
    
    /// Update routing configuration
    /// 
    /// This allows dynamic updates to routing rules without restart
    pub async fn update_routing_config(&self, config: RoutingConfig) -> CallCenterResult<()> {
        // In a real implementation, this would update the routing engine
        tracing::info!("Updating routing configuration");
        // TODO: Implement dynamic routing updates
        Ok(())
    }
    
    /// Get system health status
    pub async fn get_system_health(&self) -> SystemHealth {
        let stats = self.engine.get_stats().await;
        let database_ok = self.check_database_health().await;
        
        SystemHealth {
            status: if database_ok { HealthStatus::Healthy } else { HealthStatus::Degraded },
            database_connected: database_ok,
            active_sessions: stats.active_calls,
            registered_agents: stats.available_agents + stats.busy_agents,
            queued_calls: stats.queued_calls,
            uptime_seconds: 0, // TODO: Track actual uptime
            warnings: Vec::new(),
        }
    }
    
    /// Perform database maintenance
    pub async fn optimize_database(&self) -> CallCenterResult<()> {
        tracing::info!("Running database optimization");
        // TODO: Implement database optimization
        Ok(())
    }
    
    /// Export system configuration
    pub async fn export_config(&self) -> CallCenterResult<String> {
        let config = self.engine.config();
        serde_json::to_string_pretty(config)
            .map_err(|e| CallCenterError::internal(&format!("Failed to serialize config: {}", e)))
    }
    
    /// Import system configuration
    /// 
    /// Note: This requires a system restart to take effect
    pub async fn import_config(&self, config_json: &str) -> CallCenterResult<CallCenterConfig> {
        serde_json::from_str(config_json)
            .map_err(|e| CallCenterError::validation(&format!("Invalid config JSON: {}", e)))
    }
    
    /// Get detailed queue configuration
    pub async fn get_queue_configs(&self) -> HashMap<String, QueueConfig> {
        // In a real implementation, this would return actual queue configs
        let mut configs = HashMap::new();
        
        // Add default queues with correct field names
        for queue_id in &["general", "sales", "support", "billing", "vip", "premium", "overflow"] {
            configs.insert(
                queue_id.to_string(),
                QueueConfig {
                    default_max_wait_time: 300,
                    max_queue_size: 100,
                    enable_priorities: true,
                    enable_overflow: *queue_id != "overflow",
                    announcement_interval: 30,
                }
            );
        }
        
        configs
    }
    
    /// Check database health
    async fn check_database_health(&self) -> bool {
        if let Some(db) = self.engine.database_manager() {
            // Try to query the database with a simple query
            match db.query("SELECT 1", ()).await {
                Ok(_) => true,
                Err(e) => {
                    tracing::error!("Database health check failed: {}", e);
                    false
                }
            }
        } else {
            // No database configured
            false
        }
    }

    /// Get statistics
    pub async fn get_statistics(&self) -> CallCenterStats {
        let total_agents = if let Some(db) = self.engine.database_manager() {
            db.list_agents().await.unwrap_or_default().len()
        } else {
            0
        };
        
        let active_calls = if let Some(db) = self.engine.database_manager() {
            db.get_active_calls_count().await.unwrap_or(0)
        } else {
            0
        };
        
        let queued_calls = 0; // TODO: get from queue manager
        let available_agents = 0; // TODO: get from database
        
        CallCenterStats {
            total_agents,
            available_agents,
            active_calls,
            queued_calls,
        }
    }
}

/// System health information
#[derive(Debug, Clone)]
pub struct SystemHealth {
    pub status: HealthStatus,
    pub database_connected: bool,
    pub active_sessions: usize,
    pub registered_agents: usize,
    pub queued_calls: usize,
    pub uptime_seconds: u64,
    pub warnings: Vec<String>,
}

/// Health status enum
#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Critical,
}

impl Default for AdminApi {
    fn default() -> Self {
        panic!("AdminApi requires an engine instance")
    }
}

/// Call center statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallCenterStats {
    pub total_agents: usize,
    pub available_agents: usize,
    pub active_calls: usize,
    pub queued_calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    pub skill_name: String,
    pub skill_level: u8,
} 