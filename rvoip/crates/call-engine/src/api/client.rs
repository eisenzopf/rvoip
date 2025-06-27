//! Call center client API for agent applications
//!
//! This module provides a high-level API for agent applications to interact
//! with the call center system.

use std::sync::Arc;
use async_trait::async_trait;
use rvoip_session_core::{SessionId, CallHandler};

use crate::{
    agent::{Agent, AgentId, AgentStatus},
    error::{CallCenterError, Result as CallCenterResult},
    orchestrator::CallCenterEngine,
};

/// Call center client API for agent applications
/// 
/// This provides a simplified interface for agent applications to:
/// - Register with the call center
/// - Update their status (available, busy, away)
/// - Handle incoming calls
/// - Access call history and statistics
#[derive(Clone)]
pub struct CallCenterClient {
    engine: Arc<CallCenterEngine>,
}

impl CallCenterClient {
    /// Create a new call center client connected to the given engine
    pub fn new(engine: Arc<CallCenterEngine>) -> Self {
        Self { engine }
    }
    
    /// Register an agent with the call center
    /// 
    /// # Example
    /// ```no_run
    /// # use rvoip_call_engine::api::CallCenterClient;
    /// # use rvoip_call_engine::agent::{Agent, AgentId};
    /// # async fn example(client: CallCenterClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let agent = Agent {
    ///     id: AgentId::from("agent001"),
    ///     sip_uri: "sip:agent001@example.com".to_string(),
    ///     skills: vec!["sales".to_string(), "support".to_string()],
    ///     max_concurrent_calls: 3,
    /// };
    /// 
    /// let session_id = client.register_agent(&agent).await?;
    /// println!("Agent registered with session: {}", session_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_agent(&self, agent: &Agent) -> CallCenterResult<SessionId> {
        self.engine.register_agent(agent).await
    }
    
    /// Update agent status
    /// 
    /// # Example
    /// ```no_run
    /// # use rvoip_call_engine::api::CallCenterClient;
    /// # use rvoip_call_engine::agent::{AgentId, AgentStatus};
    /// # async fn example(client: CallCenterClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let agent_id = AgentId::from("agent001");
    /// 
    /// // Mark agent as away
    /// client.update_agent_status(&agent_id, AgentStatus::Away).await?;
    /// 
    /// // Mark agent as available again
    /// client.update_agent_status(&agent_id, AgentStatus::Available).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn update_agent_status(
        &self, 
        agent_id: &AgentId, 
        status: AgentStatus
    ) -> CallCenterResult<()> {
        self.engine.update_agent_status(agent_id, status).await
    }
    
    /// Get current agent information
    /// 
    /// Returns detailed information about the agent including:
    /// - Current status
    /// - Active calls count
    /// - Skills
    /// - Performance score
    pub async fn get_agent_info(&self, agent_id: &AgentId) -> Option<crate::orchestrator::types::AgentInfo> {
        self.engine.get_agent_info(agent_id).await
    }
    
    /// Get current queue statistics
    /// 
    /// Returns statistics for all queues the agent has access to
    pub async fn get_queue_stats(&self) -> CallCenterResult<Vec<(String, crate::queue::QueueStats)>> {
        self.engine.get_queue_stats().await
    }
    
    /// Get the underlying session manager for advanced operations
    /// 
    /// This allows direct access to session-core functionality when needed
    pub fn session_manager(&self) -> &Arc<rvoip_session_core::SessionCoordinator> {
        self.engine.session_manager()
    }
    
    /// Get the call handler for this client
    /// 
    /// This can be used to set up additional event handling if needed
    pub fn call_handler(&self) -> Arc<dyn CallHandler> {
        Arc::new(crate::orchestrator::handler::CallCenterCallHandler {
            engine: Arc::downgrade(&self.engine),
        })
    }
}

/// Builder for creating a CallCenterClient
pub struct CallCenterClientBuilder {
    config: Option<crate::config::CallCenterConfig>,
    db_path: Option<String>,
}

impl CallCenterClientBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            config: None,
            db_path: None,
        }
    }
    
    /// Set the configuration
    pub fn with_config(mut self, config: crate::config::CallCenterConfig) -> Self {
        self.config = Some(config);
        self
    }
    
    /// Set the database path
    pub fn with_database_path(mut self, path: String) -> Self {
        self.db_path = Some(path);
        self
    }
    
    /// Build the client
    pub async fn build(self) -> CallCenterResult<CallCenterClient> {
        let config = self.config
            .ok_or_else(|| CallCenterError::configuration("Configuration required"))?;
            
        let engine = CallCenterEngine::new(config, self.db_path).await?;
        Ok(CallCenterClient::new(engine))
    }
}

impl Default for CallCenterClientBuilder {
    fn default() -> Self {
        Self::new()
    }
} 