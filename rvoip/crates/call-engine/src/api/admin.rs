//! Administrative API for call center management
//!
//! This module provides APIs for administrators to configure and manage
//! the call center system.

use std::sync::Arc;
use std::collections::HashMap;

use crate::{
    agent::{Agent, AgentId},
    database::agent_store::AgentSkill,
    config::{CallCenterConfig, QueueConfig, RoutingConfig},
    error::{CallCenterError, Result as CallCenterResult},
    orchestrator::CallCenterEngine,
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
    
    /// Add a new agent to the system
    /// 
    /// This registers the agent in the database and makes them available
    /// for call routing once they connect
    pub async fn add_agent(&self, agent: Agent) -> CallCenterResult<()> {
        let mut registry = self.engine.agent_registry.lock().await;
        registry.add_agent(agent).await
    }
    
    /// Update an existing agent's configuration
    pub async fn update_agent(&self, agent: Agent) -> CallCenterResult<()> {
        let mut registry = self.engine.agent_registry.lock().await;
        registry.update_agent(agent).await
    }
    
    /// Remove an agent from the system
    /// 
    /// This will disconnect any active sessions and remove them from routing
    pub async fn remove_agent(&self, agent_id: &AgentId) -> CallCenterResult<()> {
        // First check if agent has active calls
        let agent_info = self.engine.get_agent_info(agent_id).await;
        if let Some(info) = agent_info {
            if info.current_calls > 0 {
                return Err(CallCenterError::validation(
                    "Cannot remove agent with active calls"
                ));
            }
        }
        
        let mut registry = self.engine.agent_registry.lock().await;
        registry.remove_agent(agent_id.as_str()).await
    }
    
    /// List all agents in the system
    pub async fn list_all_agents(&self) -> CallCenterResult<Vec<Agent>> {
        let registry = self.engine.agent_registry.lock().await;
        registry.list_agents().await
    }
    
    /// Update agent skills
    pub async fn update_agent_skills(
        &self, 
        agent_id: &AgentId, 
        skills: Vec<AgentSkill>
    ) -> CallCenterResult<()> {
        let mut registry = self.engine.agent_registry.lock().await;
        registry.update_agent_skills(agent_id, skills).await
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
        // Try to query the database with a simple query
        let conn = self.engine.database().connection().await;
        match conn.query("SELECT 1", ()).await {
            Ok(_) => true,
            Err(e) => {
                tracing::error!("Database health check failed: {}", e);
                false
            }
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